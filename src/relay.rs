use daemonize::Daemonize;
use serde_derive::{Deserialize, Serialize};
use std::env;
use std::io::{BufRead, BufReader};
use std::io::{BufWriter, ErrorKind};
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::{exit, Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::db::init_db;
use crate::session::Session;
use tokio::io::AsyncBufReadExt;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename = "lowercase")]
pub enum DaemonMsg {
    Heartbeat,
    Token(String),
    Title(String),
    Summary(String),
    Error(String),
}

pub type DaemonConnection<'a> = RelayConnection<'a, DaemonMsg, ClientMessage>;
pub type ClientConnection<'a> = RelayConnection<'a, ClientMessage, DaemonMsg>;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename = "lowercase")]
pub enum ClientMessage {}

pub trait RelayMsg<'de>: serde::Serialize + serde::Deserialize<'de> {
    fn encode_no_len(&self) -> crate::Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| crate::Error::CommunicationError(e.into()))
    }

    fn encode(&self) -> crate::Result<Vec<u8>> {
        let encoded = self.encode_no_len()?;
        let mut msg = encoded.len().to_string().into_bytes();

        msg.push(b'\n');
        msg.extend_from_slice(encoded.as_slice());
        Ok(msg)
    }

    fn decode(bytes: &'de [u8]) -> crate::Result<Self> {
        serde_json::from_slice(bytes).map_err(|e| crate::Error::CommunicationError(e.into()))
    }
}

impl RelayMsg<'_> for DaemonMsg {}
impl RelayMsg<'_> for ClientMessage {}

pub struct RelayConnection<'de, S, R>
where
    S: RelayMsg<'de>,
    R: RelayMsg<'de>,
{
    addr: SocketAddr,

    writer: BufWriter<TcpStream>,
    reader: BufReader<TcpStream>,

    de_buf: Vec<u8>,
    _send_type: PhantomData<S>,
    _recv_type: PhantomData<R>,

    _lifetime_marker: PhantomData<&'de ()>,
}

impl<'de, S, R> RelayConnection<'de, S, R>
where
    S: RelayMsg<'de>,
    R: RelayMsg<'de>,
{
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn send(&mut self, message: S) -> crate::Result<()> {
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
            de_buf: Vec::new(),
            _send_type: PhantomData::<S>::default(),
            _recv_type: PhantomData::<R>::default(),
            _lifetime_marker: PhantomData::<&'de ()>::default(),
        })
    }

    /// read the next message in the stream (blocking)
    /// returns None is the stream is closed
    pub fn read_next(&'de mut self) -> crate::Result<Option<R>> {
        let mut buf = String::new();

        // length header and content are separated by a newline
        // try to read just the header first
        match self.reader.read_line(&mut buf) {
            // case: stream has closed
            Ok(0) => Ok(None),

            // case: stream provided some bytes
            Ok(_) => {
                // try to read the header for content length
                let content_len = buf
                    .parse::<usize>()
                    .map_err(|e| crate::Error::CommunicationError(e.into()))?;

                // read the rest of the message

                self.de_buf.resize(content_len, 0u8);
                self.reader.read_exact(self.de_buf.as_mut_slice())?;

                R::decode(&self.de_buf.as_slice()).map(|msg| Some(msg))
            }

            // case: error
            Err(e) => Err(e.into()),
        }
    }

    // returns Ok(true) if there is data remaining in the stream
    pub fn poll(&mut self) -> crate::Result<bool> {
        let mut buf = [0u8];
        Ok(self.reader.get_ref().peek(&mut buf)? > 0)
    }

    async fn next(&mut self) {}
}

impl From<crate::Error> for DaemonMsg {
    fn from(value: crate::Error) -> Self {
        Self::Error(value.to_string())
    }
}

/// Start a new seperate relay daemon process
/// Returns a RelayConnection wrapping a TcpListener to recieve from the new process
/// Blocks until a connection is established
pub fn spawn_relay<'a>() -> crate::Result<ClientConnection<'a>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    Command::new(env::current_exe()?)
        .arg("__relay")
        .arg(port.to_string())
        .spawn()?;

    RelayConnection::init_from_listener(listener)
}

/// "main" function for the relay daemon process
async fn daemon_main(connection: DaemonConnection<'_>) -> crate::Result<()> {
    // TODO
    Ok(())
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

    let stream = TcpStream::connect_timeout(&socket_addr, timeout)?;

    let relay_connection = DaemonConnection::connect(stream)?;

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move { daemon_main(relay_connection).await })?;

    Ok(())
}
