#![recursion_limit = "1024"]
use futures::select;
use futures::FutureExt;
use futures::StreamExt;
use log::{error, info, trace};
use notify_rust::NotificationHandle;
use signal_hook::{SIGINT, SIGWINCH};
use smol::io::AsyncReadExt;
use std::error::Error;
// use std::os::unix::io::AsRawFd;
use ui::input::Action;

mod config;
mod errors;
mod ui;
mod wee;

fn main() {
    // let log = std::fs::File::create("weesels.log").unwrap();
    // nix::unistd::dup2(log.as_raw_fd(), std::io::stderr().as_raw_fd()).unwrap();
    env_logger::builder()
        .format_timestamp_secs()
        .target(env_logger::Target::Stderr)
        .filter_level(log::LevelFilter::Info)
        .init();

    smol::block_on(run()).unwrap_or_else(|e| {
        error!("Fatal error: {}", e);
    });
}

async fn get_input(stdin: &mut smol::fs::File) -> Result<String, Box<dyn Error>> {
    let mut input = vec![0u8; 10];
    let len = stdin.read(&mut input).await?;
    input.truncate(len);
    Ok(String::from_utf8(input)?)
}

async fn run() -> Result<(), Box<dyn Error>> {
    let conf = config::load_default()?;

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

    let mut signals = signals();

    // XXX track multiple.
    let mut notification: Option<NotificationHandle> = None;

    loop {
        select! {
            sig = signals.next().fuse() => {
                match sig {
                    Some(libc::SIGINT) => { break },
                    Some(libc::SIGWINCH) => ui.draw(&wee),
                    _ => break,  // unhandled or HUP
                }
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
    Ok(())
}

fn signals() -> async_channel::Receiver<i32> {
    let (s, r) = async_channel::unbounded();
    unsafe {
        let s2 = s.clone();
        signal_hook::register(SIGWINCH, move || {
            s.try_send(SIGWINCH).unwrap();
        })
        .unwrap();
        signal_hook::register(SIGINT, move || {
            s2.try_send(SIGINT).unwrap();
        })
        .unwrap();
    }
    r
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
    } else {
        if let Some(hot) = hot {
            Some(
                notify_rust::Notification::new()
                    .summary(hot.full_name.as_str())
                    .show()
                    .unwrap(),
            )
        } else {
            None
        }
    }
}
