use daemonize::Daemonize;
use serde_derive::{Deserialize, Serialize};
use std::env;
use std::io::{BufRead, BufReader};
use std::io::{BufWriter, ErrorKind};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::{exit, Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::db::init_db;
use crate::session::Session;

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename = "lowercase")]
enum MsgType {
    Token,
    Title,
    Summary,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename = "lowercase")]
pub enum RelayMsg {
    Heartbeat,
    Token(String),
    Title(String),
    Summary(String),
    Error(String),
}

impl RelayMsg {
    pub fn encode_no_len(&self) -> crate::Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| crate::Error::CommunicationError(e.into()))
    }

    pub fn encode(&self) -> crate::Result<Vec<u8>> {
        let encoded = self.encode_no_len()?;
        let mut msg = encoded.len().to_string().into_bytes();

        msg.push(b'\n');
        msg.extend_from_slice(encoded.as_slice());
        Ok(msg)
    }

    pub fn decode(bytes: &[u8]) -> crate::Result<Self> {
        serde_json::from_slice(bytes).map_err(|e| crate::Error::CommunicationError(e.into()))
    }
}

/// Try to read bytes from the reader and decode them as a RelayMsg
/// Returns Ok(None) if the reader has no content (like if the TCP connection is closed)
pub fn read_msg<T: BufRead>(stream: &mut T) -> crate::Result<Option<RelayMsg>> {
    let mut buf = String::new();

    // length header and content are separated by a newline
    // try to read just the header first
    match stream.read_line(&mut buf) {
        // case: stream has closed
        Ok(0) => Ok(None),

        // case: stream provided some bytes
        Ok(_) => {
            // try to read the header for content length
            let content_len = buf
                .parse::<usize>()
                .map_err(|e| crate::Error::CommunicationError(e.into()))?;

            // read the rest of the message
            let mut content_buf = vec![0u8; content_len];
            stream.read_exact(content_buf.as_mut_slice())?;

            RelayMsg::decode(content_buf.as_slice()).map(|msg| Some(msg))
        }

        // case: error
        Err(e) => Err(e.into()),
    }
}

pub struct RelayConnection {
    addr: SocketAddr,

    writer: BufWriter<TcpStream>,
    reader: BufReader<TcpStream>,
}

impl RelayConnection {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn send(&mut self, message: RelayMsg) -> crate::Result<()> {
        self.writer.write_all(message.encode()?.as_slice())?;
        Ok(())
    }

    pub fn init_from_listener(listener: TcpListener) -> crate::Result<Self> {
        let (stream, addr) = listener.accept()?;
        let mut conn = Self::connect(stream)?;
        conn.addr = addr;
        Ok(conn)
    }

    pub fn connect(stream: TcpStream) -> crate::Result<Self> {
        let addr = stream.local_addr()?;

        let reader = BufReader::new(stream.try_clone()?);
        let writer = BufWriter::new(stream);

        Ok(Self {
            reader,
            writer,
            addr,
        })
    }

    /// read the next message in the stream (blocking)
    /// returns None is the stream is closed
    pub fn read_next(&mut self) -> crate::Result<Option<RelayMsg>> {
        read_msg(&mut self.reader)
    }

    // returns Ok(true) if there is data remaining in the stream
    pub fn poll(&mut self) -> crate::Result<bool> {
        let mut buf = [0u8];
        Ok(self.reader.get_ref().peek(&mut buf)? > 0)
    }
}

impl From<crate::Error> for RelayMsg {
    fn from(value: crate::Error) -> Self {
        Self::Error(value.to_string())
    }
}

/// Start a new seperate relay daemon process
/// Returns a RelayConnection wrapping a TcpListener to recieve from the new process
/// Blocks until a connection is established
pub fn spawn_relay() -> crate::Result<RelayConnection> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    Command::new(env::current_exe()?)
        .arg("__relay")
        .arg(port.to_string())
        .spawn()?;

    RelayConnection::init_from_listener(listener)
}

fn handle_relay_messages(incoming: RelayMsg) -> crate::Result<Option<RelayMsg>> {
    todo!();
}

/// Entry point for the relay daemon
pub fn run(port: &str) -> crate::Result<()> {
    let port_n = port
        .parse::<u16>()
        .expect(&format!("Failed to parse port '{port}'"));

    let mut daemon_config = Daemonize::new();

    #[cfg(debug_assertions)]
    {
        let stdout_file =
            std::fs::File::create(concat!(env!("CARGO_MANIFEST_DIR"), "/relay.stdout"))?;

        let stderr_file =
            std::fs::File::create(concat!(env!("CARGO_MANIFEST_DIR"), "/relay.stderr"))?;

        daemon_config = daemon_config.stdout(stdout_file).stderr(stderr_file);
    }

    daemon_config.start().expect("Failed to start daemon");

    eprintln!("Started daemon: {}", std::process::id());

    let timeout = Duration::from_millis(250);
    let socket_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port_n);

    let mut stream = TcpStream::connect_timeout(&socket_addr, timeout)?;

    eprintln!("Initialized connection at {}", socket_addr);
    //TODO

    // setup the db
    let db_conn = init_db()?;
    let mut session = Session::new()?;
    session.load_threads()?;

    Ok(())
}
