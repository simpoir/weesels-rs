#![recursion_limit = "1024"]
use futures::select;
use futures::FutureExt;
use log::{error, info, trace};
use notify_rust::NotificationHandle;
use signal_hook::SIGWINCH;
use smol::io::AsyncReadExt;
use smol::net::unix::UnixStream;
use std::error::Error;
use std::process::exit;
use termion::raw::IntoRawMode;
use ui::input::Action;

mod cli;
mod config;
mod errors;
mod ui;
mod wee;

/// Initializes logging. Terminates process with code 1 on error.
fn init_logging(conf: &cli::CmdConf) {
    let default_log =
        std::env::temp_dir().join(format!("weesels.{}.log", nix::unistd::getpid().as_raw()));
    let log_path = conf.log_file.as_ref().unwrap_or(&default_log);

    let level = match conf.verbosity {
        0 => simplelog::LevelFilter::Info,
        1 => simplelog::LevelFilter::Debug,
        2 => simplelog::LevelFilter::Trace,
        _ => simplelog::LevelFilter::max(),
    };
    let config = simplelog::ConfigBuilder::new()
        .set_max_level(level)
        .add_filter_allow_str("weesels")
        .build();

    let log_file = std::fs::File::create(log_path).unwrap_or_else(|e| {
        eprintln!("Could not create log file: {}", e);
        exit(1);
    });
    if let Err(e) = simplelog::WriteLogger::init(simplelog::LevelFilter::max(), config, log_file) {
        eprintln!("Could not initialize logging: {}", e);
        exit(1);
    }
    log::trace!("Logging initialized to {:?}", log_path);
}

fn main() {
    let _p = setup_panic();

    let conf = cli::CmdConf::from_env();
    init_logging(&conf);

    if let Err(e) = smol::block_on(run(conf)) {
        error!("Fatal error: {}", e);
        eprintln!("Fatal error: {}", e);
        exit(2);
    }
}

async fn get_input(stdin: &mut smol::fs::File) -> Result<String, Box<dyn Error>> {
    let mut input = vec![0u8; 10];
    let len = stdin.read(&mut input).await?;
    input.truncate(len);
    Ok(String::from_utf8(input)?)
}

async fn run(conf: cli::CmdConf) -> Result<(), Box<dyn Error>> {
    let conf = config::Loader::new()?.load(&conf)?;

    let mut wee = wee::Wee::connect(&conf).await?;
    wee.buffers().await?;
    wee.run().await?; // actually send request
    wee.run().await?; // receive initial buf list
    wee.switch_current_buffer(&String::from("core.weechat"))
        .await?; // request buffer data
    wee.send("sync", "sync").await?; // subscribe to events

    let mut ui = ui::Ui::new();
    ui.draw(&wee);
    let mut stdin = smol::fs::File::from(termion::get_tty()?);

    trace!("connected");

    let mut signals = Signals::new()?;

    // XXX track multiple?
    let mut notification: Option<NotificationHandle> = None;

    loop {
        select! {
            () = signals.wait().fuse() => {
                ui.draw(&wee);
            },
            incoming = wee.run().fuse() => {
                incoming?;
                if wee.get_current_buffer().is_none() {
                    wee.switch_current_buffer(&String::from("core.weechat")).await?;
                }
                notification = desktop_notify(notification, &wee);
                ui.draw(&wee);
            }
            input = get_input(&mut stdin).fuse() => {
                match input {
                    Ok(s) => {
                        match ui.input.handle_input(s) {
                            Action::Input => {
                                if let Some(buf) = wee.get_current_buffer() {
                                    // send lines separately, as protocol does not handle newlines
                                    for line in ui.input.get_string().lines() {
                                        wee.send("", format!("input {} {}", buf.full_name, line).as_str()).await?;
                                    }
                                    ui.input.clear();
                                    ui.draw(&wee);
                                }
                            }
                            Action::Quit => break,
                            Action::BufChange(i) => {
                                match wee.get_current_buffer() {
                                    None => (),
                                    Some(current) => {
                                        let mut iter = wee.get_buffers().iter();
                                        let mut prev = None;
                                        while let Some(b) = iter.next() {
                                            if b.full_name == current.full_name {
                                                let new_buf = if i > 0 {
                                                    iter.next()
                                                } else {
                                                    prev
                                                };
                                                if let Some(new_buf) = new_buf {

                                                    trace!("buf change {}", new_buf.full_name);
                                                    wee.switch_current_buffer(&new_buf.full_name).await?;
                                                    ui.draw(&wee);
                                                }
                                                break;
                                            } else {
                                                prev = Some(b);
                                            }
                                        }
                                    }
                                };
                            },
                            Action::BufChangeAbs(i) => {
                                if let Some(b) = &wee.get_buffers().get(i){
                                    wee.switch_current_buffer(&b.full_name).await?
                                };
                            }
                            Action::ScrollBack => {
                                wee.scroll_back(10).await?;

                            }
                            Action::Completion(pos, data) => {
                                if let Some(buf) = wee.get_current_buffer() {
                                    wee.send("completion", &format!("completion 0x{} {} {}", buf.ptr_buffer, pos, data)).await?;
                                }
                            },
                            _ => ui.draw(&wee),
                        };
                    }
                    Err(e) => {info!("{:?}", e); break},
                }
            }
        }
    }
    info!("Closing");
    wee.close().await.map_err(Box::new)?;

    // Cleanup any popup on quit.
    if let Some(n) = notification {
        n.close();
    }
    Ok(())
}

struct Signals {
    inner: UnixStream,
}

impl Signals {
    fn new() -> Result<Self, Box<dyn Error>> {
        let (s, r) = UnixStream::pair()?;
        signal_hook::pipe::register(SIGWINCH, s)?;
        Ok(Self { inner: r })
    }

    async fn wait(&mut self) {
        let mut buf = [0u8; 10]; // debounces signals slightly.
        if let Err(e) = self.inner.read(&mut buf).await {
            log::error!("Error reading signals from pipe: {}", e);
        }
    }
}

fn desktop_notify(notif: Option<NotificationHandle>, wee: &wee::Wee) -> Option<NotificationHandle> {
    let hot = wee
        .get_buffers()
        .iter()
        .find(|b| b.hotlist.2 > 0 || b.hotlist.3 > 0);
    if let Some(existing) = notif {
        if let None = hot {
            existing.close();
            None
        } else {
            Some(existing)
        }
    } else if let Some(hot) = hot {
        notify_rust::Notification::new()
            .summary(hot.full_name.as_str())
            .show()
            .map_or_else(
                |e| {
                    log::error!("Could not show notification: {}", e);
                    None
                },
                Some,
            )
    } else {
        None
    }
}

struct PanicHandle {}
impl Drop for PanicHandle {
    fn drop(&mut self) {
        let _ = std::panic::take_hook();
    }
}

/// Configure panic handler for restoring terminal from TUI.
#[must_use]
fn setup_panic() -> PanicHandle {
    let raw_handle = std::io::stdout().into_raw_mode().unwrap();
    std::panic::set_hook(Box::new(move |info| {
        println!("{}", termion::screen::ToMainScreen);
        if let Err(e) = raw_handle.suspend_raw_mode() {
            log::error!("Could not restore terminal on panic: {}", e);
        }
        better_panic::Settings::new().create_panic_handler()(info);
    }));
    PanicHandle {}
}
