use async_channel::{Receiver, Sender};
use async_native_tls::TlsStream;
use futures::{future::FutureExt, select};
use log::{info, trace};
use smol::{io::AsyncReadExt, io::AsyncWriteExt, Async};
use std::borrow::Borrow;
use std::cell::RefCell;
use std::net::TcpStream;

pub use messages::{Buffer, CompletionData, LineData};

const BUFFER_CACHE_SIZE: usize = 100;

pub mod auth;
mod de;
mod messages;

#[derive(Debug)]
pub enum Error {
    PacketError { source: de::Error },
    ProtocolError(&'static str),
    IOError { source: std::io::Error },
    TlsError { source: async_native_tls::Error },
    OpensslError { source: openssl::error::ErrorStack },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        f.write_fmt(format_args!("{:?}", self))
    }
}

impl std::error::Error for Error {}

impl std::convert::From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::IOError { source }
    }
}

impl std::convert::From<de::Error> for Error {
    fn from(source: de::Error) -> Self {
        Error::PacketError { source }
    }
}

impl std::convert::From<async_native_tls::Error> for Error {
    fn from(source: async_native_tls::Error) -> Self {
        Error::TlsError { source }
    }
}

impl std::convert::From<openssl::error::ErrorStack> for Error {
    fn from(source: openssl::error::ErrorStack) -> Self {
        Error::OpensslError { source }
    }
}

type Stream = TlsStream<Async<TcpStream>>;
type Result<T> = std::result::Result<T, Error>;

/// Weechat relay client.
pub struct Wee {
    stream: Stream,
    current_buffer: RefCell<String>,
    bufs: Vec<Buffer>,
    buf_lines: Vec<messages::LineData>,
    send_queue: (Sender<String>, Receiver<String>),
    completion: RefCell<Option<messages::CompletionData>>,
    pub is_scrolling: bool,
}

impl Wee {
    pub async fn connect(host: &str, port: u16, pass: &str) -> Result<Wee> {
        let stream = connect(host, port).await.unwrap();
        let current_buffer = RefCell::new(String::from(""));
        let mut wee = Wee {
            stream,
            current_buffer,
            bufs: vec![],
            buf_lines: vec![],
            send_queue: async_channel::unbounded(),
            is_scrolling: false,
            completion: RefCell::new(None),
        };
        wee.auth(pass).await?;
        Ok(wee)
    }

    pub fn get_buffers(&self) -> &Vec<Buffer> {
        &self.bufs
    }

    pub fn get_lines(&self) -> &Vec<LineData> {
        &self.buf_lines
    }

    pub async fn switch_current_buffer(&self, full_name: &String) -> Result<()> {
        self.current_buffer.replace(full_name.clone());
        if let Some(current) = self.get_current_buffer() {
            self.send(
                "backlog_lines",
                format!(
                    "hdata buffer:0x{}/own_lines/last_line(-{})/data",
                    current.ptr_buffer, BUFFER_CACHE_SIZE
                )
                .as_str(),
            )
            .await
        } else {
            Ok(())
        }
    }

    pub async fn scroll_back(&self, scroll: usize) -> Result<()> {
        if let Some(last_line) = self.buf_lines.get(
            self.buf_lines
                .len()
                .checked_sub(1 + scroll)
                .unwrap_or_default(),
        ) {
            if let Some(ref ptr_line) = last_line.ptr_line {
                self.send(
                    "scrollback_lines",
                    format!("hdata line:0x{}(-{})/data", ptr_line, BUFFER_CACHE_SIZE).as_str(),
                )
                .await?;
            } else {
                self.switch_current_buffer(&self.current_buffer.borrow())
                    .await?;
            }
        }
        Ok(())
    }

    pub fn get_current_buffer(&self) -> Option<&Buffer> {
        let current_name = self.current_buffer.borrow();
        self.bufs
            .iter()
            .find(|b| b.full_name.as_str() == current_name.as_str())
    }

    pub async fn close(&self) -> Result<()> {
        self.send_queue
            .0
            .send(String::from(""))
            .await
            .expect("Queueing close");
        Ok(())
    }

    pub async fn send(&self, id: &str, command: &str) -> Result<()> {
        let msg = format!("({}) {}\n", id, command);
        self.send_queue
            .0
            .send(msg)
            .await
            .expect("Queueing outgoing command");
        Ok(())
    }

    pub async fn buffers(&self) -> Result<()> {
        // XXX it would be super nice to limit sent fields instead of receiving
        // all that unused data.
        self.send("gui_buffers", "hdata buffer:gui_buffers(*)")
            .await
    }

    pub async fn hotlist(&self) -> Result<()> {
        self.send("gui_hotlist", "hdata hotlist:gui_hotlist(*)")
            .await
    }

    /// Run and exchange messages.
    pub async fn run(&mut self) -> Result<()> {
        // The blocks deserve an explanation...
        // Considering this function is expected to be selected from a higher
        // callsite. As such, the initial poll is cancelable, but as soon
        // as we know there is data, we block. This ensures that reads
        // and writes don't get cancelled mid-work and get all out of sync.
        //
        // The unpleasant side effect, is that the process will hang if
        // the writes/reads block. The pleasant side-effect is that we don't
        // have to keep state over partial reads, thus keeping everything
        // simple. The other side effect, is that this future can be dropped
        // to release the mutable borrow.
        select! {
            len = read_u32(&mut self.stream).fuse() => {
                smol::block_on(self.handle_one(len? as usize))?;
                while self.stream.buffered_read_size()? > 0 {
                    let len = smol::block_on(read_u32(&mut self.stream))?;
                    smol::block_on(self.handle_one(len as usize))?;
                }
            },
            outgoing = self.send_queue.1.recv().fuse() => {
                let outgoing = outgoing.expect("Reading send queue");
                if outgoing.len() == 0 {
                    self.stream.write("(quit) quit\n".as_bytes()).await?;
                } else {
                    smol::block_on(self.stream.write(outgoing.as_bytes()))?;
                }
                while !self.send_queue.1.is_empty() {
                    smol::block_on(
                        self.stream.write(
                            smol::block_on(self.send_queue.1.recv())
                                .expect("Reading send queue").as_bytes())
                    )?;
                }
            },
        };
        Ok(())
    }

    async fn handle_one(&mut self, len: usize) -> Result<()> {
        let mut comp = [0u8; 1];
        self.stream.read_exact(&mut comp).await?;
        assert_eq!(0, comp[0], "compression not implemented");
        let mut buf = vec![0u8; len - 5];
        self.stream.read_exact(&mut buf).await?;
        let msg_id = de::peek_str(&buf)?;
        trace!("got message {:?}", msg_id);
        match msg_id {
            Some("gui_buffers") => {
                let bufs: messages::BuffersResponse = de::from_bytes(&buf[..])?;
                trace!("got buffers {:?}", bufs);
                self.bufs = bufs.hda;
            }
            Some("gui_hotlist") => {
                let hl: messages::Hdata<messages::Hotlist> = de::from_bytes(&buf[..])?;
                trace!("got hotlist {:?}", hl);
                // clear existing hotlist
                for mut b in &mut self.bufs {
                    b.hotlist = (0, 0, 0, 0);
                }
                for hot in hl.hda {
                    self.bufs
                        .iter_mut()
                        .find(|b| b.ptr_buffer == hot.buffer)
                        .map_or((), |b| {
                            b.hotlist = hot.count;
                        });
                }
                if self.current_buffer.borrow().as_str() == "" {
                    self.switch_current_buffer(&String::from("core.weechat"))
                        .await?;
                }
            }
            Some("backlog_lines") | Some("scrollback_lines") => {
                self.is_scrolling = msg_id == Some("scrollback_lines");
                self.buf_lines.clear();
                let bl: messages::Hdata<LineData> = de::from_bytes(&buf[..])?;
                // mark buffer as read
                if bl.hda.len() > 0 && !self.is_scrolling {
                    self.send_queue
                        .0
                        .send(format!(
                            "input 0x{} /buffer set hotlist -1\n",
                            bl.hda[0].buffer
                        ))
                        .await
                        .expect("Queue message");
                    self.hotlist().await.expect("requesting hotlist");
                }

                // buffer messages
                for mut l in bl.hda {
                    if let Some(prefix) = l.prefix {
                        l.prefix = Some(strip_colors(prefix));
                    }
                    l.message = strip_colors(l.message);
                    self.buf_lines.insert(0, l); // request was from end. reverse the list.
                }
            }
            Some("_buffer_opened")
            | Some("_buffer_closing")
            | Some("_buffer_renamed")
            | Some("_buffer_title_changed") => {
                self.buffers().await?;
                self.hotlist().await?;
            }
            Some("_buffer_line_added") => {
                let mut msg: messages::LineAddedEvent = de::from_bytes(&buf[..])?;
                let current = self.current_buffer.borrow();
                for mut buf in self.bufs.iter_mut() {
                    {
                        if buf.ptr_buffer.as_str() != msg.hda.0.buffer.as_str() {
                            continue;
                        }
                    }
                    if buf.full_name.as_str() == current.as_str() && !self.is_scrolling {
                        // add to current lines
                        if buf.ptr_buffer == msg.hda.0.buffer {
                            if let Some(prefix) = msg.hda.0.prefix {
                                msg.hda.0.prefix = Some(strip_colors(prefix));
                            }
                            msg.hda.0.message = strip_colors(msg.hda.0.message);
                            self.buf_lines.push(msg.hda.0);
                        }
                    } else {
                        // increment hotlist
                        match msg.hda.0.notify_level {
                            0 => buf.hotlist.0 += 1,
                            1 => buf.hotlist.1 += 1,
                            2 => buf.hotlist.2 += 1,
                            3 => buf.hotlist.3 += 1,
                            _ => (),
                        }
                    }
                    break;
                }
                trace!("{:?}", self.bufs);
            }
            Some("completion") => {
                let mut msg: messages::CompletionResponse = de::from_bytes(&buf[..])?;
                log::trace!("completion: {:?}", msg);
                if msg.hda.len() > 0 {
                    self.completion.replace(Some(msg.hda.remove(0)));
                }
            }
            msg_id => {
                trace!("received ignored messsage {:?}", msg_id);
                trace!("{:?}", &buf);
            }
        }
        Ok(())
    }

    async fn auth(&mut self, pass: &str) -> Result<()> {
        self.stream
            .write(
                format!(
                    "(handshake) handshake compression=off,password_hash_algo={}\n",
                    auth::SUPPORTED_HASHES
                )
                .as_bytes(),
            )
            .await?;
        let res: messages::HandshakeResponse = get_message(&mut self.stream).await?;
        assert_eq!("handshake", res.id, "expected handshake response");
        trace!("handshake response: {:?}", res);

        trace!("Sending auth");
        let auth = format!(
            "init {}\n",
            auth::create_auth(res.htb.borrow().into(), pass),
        );
        self.stream.write(auth.as_bytes()).await?;

        trace!("checking version info");
        self.stream.write(b"(version_check) info version\n").await?;
        let received: messages::Info = get_message(&mut self.stream).await.or_else(|e| {
            Err(match e {
                Error::PacketError {
                    source: de::Error::Eof,
                } => Error::ProtocolError("Connection unexpectedly closed. Check password."),
                e => e,
            })
        })?;
        info!("Server version {:?}", received.inf.1);
        Ok(())
    }

    /// Return the current completion data.
    pub fn consume_completion(&self) -> Option<messages::CompletionData> {
        self.completion.replace(None)
    }
}

async fn connect(host: &str, port: u16) -> Result<Stream> {
    trace!("creating stream");
    let stream = Async::new(TcpStream::connect((host, port))?)?;

    trace!("doing tls handshake");
    Ok(async_native_tls::connect(host, stream).await?)
}

async fn read_u32<S>(stream: &mut S) -> Result<u32>
where
    S: AsyncReadExt + std::marker::Unpin,
{
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf).await?;
    Ok(u32::from_be_bytes(buf))
}

async fn get_message<S, T>(stream: &mut S) -> Result<T>
where
    S: AsyncReadExt + std::marker::Unpin,
    T: serde::de::DeserializeOwned,
{
    let len = read_u32(stream).await? as usize;
    let mut comp = [0u8; 1];
    stream.read_exact(&mut comp).await?;
    assert_eq!(0, comp[0], "compression not implemented");
    let mut buf = vec![0u8; len - 5];
    stream.read_exact(&mut buf).await?;
    Ok(de::from_bytes(&buf[..])?)
}

/// strip weechat colors from string
fn strip_colors(input: String) -> String {
    let mut output = String::new();
    let mut it = input.chars().peekable();
    loop {
        let c = match it.next() {
            None => break,
            Some(c) => c,
        };

        if c == '\x1c' {
        } else if c == '\x19' {
            if let 'F' | 'B' = it.peek().unwrap() {
                it.next();
            }
            if let '*' | '!' | '/' | '_' | '|' = it.peek().unwrap() {
                it.next();
            }
            match it.peek().unwrap() {
                '@' => {
                    for _ in 0..6 {
                        it.next();
                    }
                }
                _ => {
                    it.next();
                    it.next();
                }
            }
            match it.peek() {
                Some('~') | Some(',') => {
                    it.next();
                    match it.peek().unwrap() {
                        '@' => {
                            for _ in 0..6 {
                                it.next();
                            }
                        }
                        _ => {
                            it.next();
                            it.next();
                        }
                    }
                }
                _ => (),
            }
        } else {
            output.push(c);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_colors() {
        let inputs = [
            "\x1904foobar",
            "foo\x1901bar",
            "foo\x19F01bar",
            "foo\x19B22bar",
            "foo\x19F@12345bar",
            "foo\x19@12345,23bar",
            "foo\x19@12345,@12345bar",
            "foo\x19@12345~@12345bar",
            "foo\x19*@12345~@12345bar",
        ];
        for input in inputs.iter() {
            assert_eq!(
                String::from("foobar"),
                strip_colors(String::from(*input)),
                "Stripping colors from {:?}",
                input
            );
        }
    }
}
