//! Reverse WebSocket tunnel: destination DNS and TCP run on this machine.
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use tungstenite::Message;

use crate::{Client, Error, Result, cdp::proxy};

const FRAME: usize = 65536;
const WINDOW: usize = 262144;
const MAX_STREAMS: usize = 128;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
static CONNECTING: AtomicUsize = AtomicUsize::new(0);

pub fn tunnel_region(id: &str) -> Result<String> {
    let invalid = || Error::Config("invalid local tunnel id".into());
    let parts: Vec<_> = id.split('.').collect();
    if parts.len() != 3 || parts[0] != "lt1" {
        return Err(invalid());
    }
    let uuid = parts[2].as_bytes();
    if uuid.len() != 36
        || !uuid.iter().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                *b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(b)
            }
        })
    {
        return Err(invalid());
    }
    let bytes = URL_SAFE_NO_PAD.decode(parts[1]).map_err(|_| invalid())?;
    let region = String::from_utf8(bytes).map_err(|_| invalid())?;
    if region.is_empty()
        || region.len() > 64
        || region.starts_with('-')
        || !region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || URL_SAFE_NO_PAD.encode(&region) != parts[1]
    {
        return Err(invalid());
    }
    Ok(region)
}

#[derive(Deserialize)]
struct Ticket {
    id: String,
    region_id: String,
    ticket: String,
    ws_url: String,
}

impl Client {
    /// Opens a tunnel and waits for the gateway's ready frame. Keep the handle alive.
    pub fn open_tunnel(&self) -> Result<LocalTunnel> {
        LocalTunnel::open(self)
    }

    pub fn tunnel_status(&self, id: &str) -> Result<Value> {
        self.tunnel_request(Some(id), "status")
    }

    pub fn close_tunnel(&self, id: &str) -> Result<Value> {
        self.tunnel_request(Some(id), "close")
    }

    fn tunnel_request(&self, id: Option<&str>, action: &str) -> Result<Value> {
        let body = self.tunnel_region_body(id)?;
        let path = match id {
            Some(id) if action == "close" => format!("/instance/v1/tunnels/{id}"),
            Some(id) => format!("/instance/v1/tunnels/{id}/{action}"),
            None => "/instance/v1/tunnels".into(),
        };
        self.json(
            if action == "close" {
                Method::DELETE
            } else {
                Method::POST
            },
            &path,
            Some(body),
        )
    }
}

/// Owned local network connector. Drop stops forwarding and best-effort revokes it.
/// Call close explicitly to observe revocation errors. Reconnection never replays TCP.
pub struct LocalTunnel {
    id: String,
    region: String,
    client: Client,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<()>>>,
    revoked: bool,
}

impl LocalTunnel {
    fn open(client: &Client) -> Result<Self> {
        let client = client.tunnel_client()?;
        let ticket: Ticket = serde_json::from_value(client.tunnel_request(None, "")?)?;
        let mut handle = Self {
            id: ticket.id.clone(),
            region: ticket.region_id.clone(),
            client,
            stop: Arc::new(AtomicBool::new(false)),
            ready: Arc::new(AtomicBool::new(false)),
            worker: None,
            revoked: false,
        };
        if tunnel_region(&ticket.id)? != ticket.region_id {
            return Err(Error::Tunnel("gateway returned a mismatched region".into()));
        }
        let connection = Connection::connect(&ticket)?;
        handle.ready.store(true, Ordering::Release);
        let client = handle.client.clone();
        let stop = handle.stop.clone();
        let ready = handle.ready.clone();
        handle.worker = Some(thread::Builder::new().name("local-tunnel".into()).spawn(
            move || {
                let mut connection = connection;
                loop {
                    let _ = connection.run(&stop);
                    ready.store(false, Ordering::Release);
                    // Drop all TCP streams before acquiring a fresh owner epoch.
                    drop(connection);
                    let mut delay = 1;
                    loop {
                        if wait_stop(&stop, Duration::from_secs(delay)) {
                            return Ok(());
                        }
                        let reconnect = client.tunnel_request(Some(&ticket.id), "connect-ticket");
                        let value = match reconnect {
                            Ok(value) => value,
                            Err(Error::NotFound(_)) => return Ok(()),
                            Err(Error::Authentication(_)) => {
                                return Err(Error::Tunnel(
                                    "tunnel authorization was revoked".into(),
                                ));
                            }
                            Err(_) => {
                                delay = (delay * 2).min(15);
                                continue;
                            }
                        };
                        if stop.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let Some(token) = value["ticket"].as_str() else {
                            return Err(Error::Tunnel("missing reconnect ticket".into()));
                        };
                        let next = Ticket {
                            ticket: token.into(),
                            id: ticket.id.clone(),
                            region_id: ticket.region_id.clone(),
                            ws_url: ticket.ws_url.clone(),
                        };
                        match Connection::connect(&next) {
                            Ok(new) => {
                                connection = new;
                                ready.store(true, Ordering::Release);
                                break;
                            }
                            Err(_) => delay = (delay * 2).min(15),
                        }
                    }
                }
            },
        )?);
        Ok(handle)
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn region_id(&self) -> &str {
        &self.region
    }
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn is_running(&self) -> bool {
        self.worker.as_ref().is_some_and(|w| !w.is_finished())
    }

    pub fn close(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        self.ready.store(false, Ordering::Release);
        let worker_result = self
            .worker
            .take()
            .map(|w| {
                w.join().unwrap_or_else(|_| {
                    Err(Error::Tunnel("tunnel worker stopped unexpectedly".into()))
                })
            })
            .unwrap_or(Ok(()));
        if !self.revoked {
            match self.client.close_tunnel(&self.id) {
                Ok(_) | Err(Error::NotFound(_)) => self.revoked = true,
                Err(error) => return Err(error),
            }
        }
        worker_result
    }
}

impl Drop for LocalTunnel {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn wait_stop(stop: &AtomicBool, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if stop.load(Ordering::Acquire) {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    stop.load(Ordering::Acquire)
}

fn tunnel_error() -> Error {
    Error::Tunnel("invalid gateway frame".into())
}
fn would_block(error: &tungstenite::Error) -> bool {
    matches!(error, tungstenite::Error::Io(e) if e.kind() == io::ErrorKind::WouldBlock)
}

struct Stream {
    socket: Option<TcpStream>,
    pending: Option<mpsc::Receiver<Result<TcpStream>>>,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
    send_credit: usize,
    receive_credit: usize,
    incoming: VecDeque<u8>,
    remote_fin: bool,
    write_closed: bool,
    local_fin: bool,
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(socket) = &self.socket {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}

struct ConnectPermit;
impl Drop for ConnectPermit {
    fn drop(&mut self) {
        CONNECTING.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Stream {
    fn open(host: String, port: u16) -> Result<Self> {
        // Keep the bounded admission compatible with both older toolchains and
        // newer Rust versions that renamed AtomicUsize::fetch_update.
        let mut count = CONNECTING.load(Ordering::Acquire);
        loop {
            if count >= MAX_STREAMS {
                return Err(Error::Tunnel("local connect capacity exceeded".into()));
            }
            match CONNECTING.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => count = current,
            }
        }
        let permit = ConnectPermit;
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let (send, receive) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("local-connect".into())
            .spawn(move || {
                let _permit = permit;
                let result = proxy::connect_local_tcp(&host, port, CONNECT_TIMEOUT, &cancel);
                let _ = send.send(result);
            })?;
        Ok(Self {
            socket: None,
            pending: Some(receive),
            cancelled,
            deadline: Instant::now() + CONNECT_TIMEOUT,
            send_credit: WINDOW,
            receive_credit: WINDOW,
            incoming: VecDeque::new(),
            remote_fin: false,
            write_closed: false,
            local_fin: false,
        })
    }
}

struct Connection {
    ws: proxy::Socket,
    transport: Arc<TcpStream>,
    streams: BTreeMap<u32, Stream>,
    outgoing: VecDeque<Message>,
    last_id: u32,
    last_serviced: u32,
    last_ping: Instant,
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.transport.shutdown(Shutdown::Both);
    }
}

impl Connection {
    fn connect(ticket: &Ticket) -> Result<Self> {
        let url = url::Url::parse(&ticket.ws_url)
            .map_err(|_| Error::Config("invalid tunnel endpoint".into()))?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !(url.scheme() == "wss"
                || (url.scheme() == "ws"
                    && ["localhost", "127.0.0.1", "[::1]"].contains(&url.host_str().unwrap_or(""))))
        {
            return Err(Error::Config(
                "tunnel endpoint must use wss (ws is allowed only on loopback)".into(),
            ));
        }
        let (ws, transport) = proxy::connect_tunnel(&ticket.ws_url, &ticket.ticket)
            .map_err(|_| Error::Tunnel("gateway WebSocket handshake failed".into()))?;
        transport.set_nonblocking(true)?;
        let mut this = Self {
            ws,
            transport,
            streams: BTreeMap::new(),
            outgoing: VecDeque::new(),
            last_id: 0,
            last_serviced: 0,
            last_ping: Instant::now(),
        };
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err(Error::Timeout("local tunnel ready".into()));
            }
            match this.ws.read() {
                Ok(Message::Text(text)) => {
                    if text.len() > 4096 {
                        return Err(tunnel_error());
                    }
                    let value: Value = serde_json::from_str(&text).map_err(|_| tunnel_error())?;
                    if value["type"] != "ready"
                        || value["id"] != ticket.id
                        || value.get("epoch").is_none()
                    {
                        return Err(tunnel_error());
                    }
                    return Ok(this);
                }
                Ok(Message::Ping(_)) => {
                    let _ = this.ws.flush();
                }
                Err(e) if would_block(&e) => thread::sleep(Duration::from_millis(10)),
                _ => return Err(Error::Tunnel("gateway closed before ready".into())),
            }
        }
    }

    fn control(&mut self, value: Value) {
        self.outgoing
            .push_back(Message::Text(value.to_string().into()));
    }
    fn reset(&mut self, id: u32) {
        self.streams.remove(&id);
        self.control(json!({"type":"reset","id":id,"code":"stream_closed"}));
    }

    fn receive(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Ping(_) => self.last_ping = Instant::now(),
            Message::Pong(_) => (),
            Message::Close(_) => return Err(Error::Tunnel("gateway disconnected".into())),
            Message::Binary(data) => {
                if data.len() < 5 || data.len() > FRAME + 4 {
                    return Err(tunnel_error());
                }
                let id = u32::from_be_bytes(data[..4].try_into().unwrap());
                if let Some(stream) = self.streams.get_mut(&id) {
                    let count = data.len() - 4;
                    if stream.socket.is_none() || stream.remote_fin || count > stream.receive_credit
                    {
                        return Err(tunnel_error());
                    }
                    stream.receive_credit -= count;
                    stream.incoming.extend(&data[4..]);
                }
            }
            Message::Text(text) => {
                if text.len() > 4096 {
                    return Err(tunnel_error());
                }
                let value: Value = serde_json::from_str(&text).map_err(|_| tunnel_error())?;
                let id = value["id"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(tunnel_error)?;
                match value["type"].as_str() {
                    Some("open") => {
                        let host = value["host"]
                            .as_str()
                            .filter(|s| {
                                !s.is_empty()
                                    && s.len() <= 253
                                    && !s.chars().any(|c| {
                                        c.is_whitespace() || c.is_control() || c == '/' || c == '\\'
                                    })
                            })
                            .ok_or_else(tunnel_error)?;
                        let port = value["port"]
                            .as_u64()
                            .and_then(|n| u16::try_from(n).ok())
                            .filter(|n| *n > 0)
                            .ok_or_else(tunnel_error)?;
                        if id <= self.last_id {
                            return Err(tunnel_error());
                        }
                        self.last_id = id;
                        if self.streams.len() >= MAX_STREAMS {
                            self.reset(id);
                        } else {
                            match Stream::open(host.into(), port) {
                                Ok(stream) => {
                                    self.streams.insert(id, stream);
                                }
                                Err(_) => self.reset(id),
                            }
                        }
                    }
                    Some("reset") => {
                        self.streams.remove(&id);
                    }
                    Some("window") => {
                        if let Some(stream) = self.streams.get_mut(&id) {
                            let credit = value["credit"]
                                .as_u64()
                                .filter(|n| *n > 0 && *n <= WINDOW as u64)
                                .ok_or_else(tunnel_error)?
                                as usize;
                            if stream.send_credit + credit > WINDOW {
                                return Err(tunnel_error());
                            }
                            stream.send_credit += credit;
                        }
                    }
                    Some("fin") => {
                        if let Some(stream) = self.streams.get_mut(&id) {
                            if stream.socket.is_none() || stream.remote_fin {
                                return Err(tunnel_error());
                            }
                            stream.remote_fin = true;
                        }
                    }
                    _ => return Err(tunnel_error()),
                }
            }
            _ => return Err(tunnel_error()),
        }
        Ok(())
    }

    // Each frame accepted by tungstenite is removed exactly once, including
    // WouldBlock (already buffered). WriteBufferFull retains the original frame.
    fn flush(&mut self) -> Result<bool> {
        match self.ws.flush() {
            Ok(()) => (),
            Err(e) if would_block(&e) => return Ok(false),
            Err(e) => return Err(e.into()),
        }
        while let Some(message) = self.outgoing.pop_front() {
            match self.ws.send(message) {
                Ok(()) => (),
                Err(e) if would_block(&e) => return Ok(false),
                Err(tungstenite::Error::WriteBufferFull(message)) => {
                    self.outgoing.push_front(message);
                    return Ok(false);
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(true)
    }

    fn run(&mut self, stop: &AtomicBool) -> Result<()> {
        let mut buffer = [0u8; FRAME];
        while !stop.load(Ordering::Acquire) {
            if self.last_ping.elapsed() > Duration::from_secs(20) {
                return Err(Error::Timeout("local tunnel heartbeat".into()));
            }
            for _ in 0..256 {
                if self.outgoing.len() >= 256 {
                    break;
                }
                match self.ws.read() {
                    Ok(message) => self.receive(message)?,
                    Err(e) if would_block(&e) => break,
                    Err(e) => return Err(e.into()),
                }
            }
            if !self.flush()? {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            let mut ids: Vec<_> = self.streams.keys().copied().collect();
            let start = ids.partition_point(|id| *id <= self.last_serviced);
            ids.rotate_left(start);
            for id in ids {
                self.last_serviced = id;
                let Some(stream) = self.streams.get_mut(&id) else {
                    continue;
                };
                if let Some(pending) = &stream.pending {
                    match pending.try_recv() {
                        Ok(Ok(socket)) => {
                            stream.socket = Some(socket);
                            stream.pending = None;
                            self.control(json!({"type":"opened","id":id}));
                        }
                        Err(mpsc::TryRecvError::Empty) if Instant::now() < stream.deadline => (),
                        _ => self.reset(id),
                    }
                    continue;
                }
                let socket = stream.socket.as_mut().unwrap();
                let mut controls = Vec::new();
                if !stream.incoming.is_empty() {
                    match socket.write(stream.incoming.make_contiguous()) {
                        Ok(0) => {
                            self.reset(id);
                            continue;
                        }
                        Ok(n) => {
                            stream.incoming.drain(..n);
                            stream.receive_credit += n;
                            controls.push(json!({"type":"window","id":id,"credit":n}));
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
                        Err(_) => {
                            self.reset(id);
                            continue;
                        }
                    }
                }
                if stream.remote_fin && stream.incoming.is_empty() && !stream.write_closed {
                    if socket.shutdown(Shutdown::Write).is_err() {
                        self.reset(id);
                        continue;
                    }
                    stream.write_closed = true;
                }
                if !stream.local_fin && stream.send_credit > 0 {
                    match socket.read(&mut buffer[..FRAME.min(stream.send_credit)]) {
                        Ok(0) => {
                            stream.local_fin = true;
                            controls.push(json!({"type":"fin","id":id}));
                        }
                        Ok(n) => {
                            stream.send_credit -= n;
                            let mut data = id.to_be_bytes().to_vec();
                            data.extend_from_slice(&buffer[..n]);
                            self.outgoing.push_back(Message::Binary(data.into()));
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
                        Err(_) => {
                            self.reset(id);
                            continue;
                        }
                    }
                }
                let finished = stream.local_fin && stream.write_closed;
                for control in controls {
                    self.control(control);
                }
                if finished {
                    self.streams.remove(&id);
                }
                if !self.flush()? {
                    break;
                }
            }
            self.flush()?;
            thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }
}
