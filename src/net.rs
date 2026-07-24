//! Multiplayer networking for the 3D mode.
//!
//! There is no separate server binary: the first terminal to claim the
//! seed's rendezvous port becomes the authoritative host, and every other
//! terminal with the same seed connects to it as a client. The wire format
//! is newline-delimited JSON so it stays debuggable with `nc`.
//!
//! All socket I/O lives on background threads. The game thread only ever
//! touches `mpsc` channels, so a slow or dead peer can never stall a frame.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Ports are picked from this range so a seed maps to a stable rendezvous.
const PORT_BASE: u16 = 30_000;
const PORT_SPAN: u16 = 20_000;
/// How long a joining client waits for the host's world snapshot.
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);

pub type PlayerId = u32;

/// A world handed to a joining client: voxel bytes, spawn point, world clock.
pub type WorldSnapshot = (Vec<u8>, (f32, f32, f32), u64);

/// Which end of the connection we are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Host,
    Client,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Host => "host",
            Role::Client => "guest",
        }
    }
}

/// How to reach the session. Built from the CLI flags.
#[derive(Clone, Debug)]
pub struct NetOpts {
    pub name: String,
    /// Explicit address to join instead of the local rendezvous.
    pub join: Option<String>,
    /// Bind on all interfaces so other machines on the LAN can join.
    pub open: bool,
}

impl Default for NetOpts {
    fn default() -> NetOpts {
        NetOpts {
            name: default_name(),
            join: None,
            open: false,
        }
    }
}

pub fn default_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .map(|n| sanitize_name(&n))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "player".to_string())
}

/// Names go straight into other people's terminals, so keep them boring.
pub fn sanitize_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .take(16)
        .collect()
}

/// Rendezvous port for a seed. Same seed on the same host means same port.
pub fn port_for_seed(seed: u64) -> u16 {
    let mut h = seed ^ 0x5DEE_CE66_D1E5_1234;
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    PORT_BASE + (h % PORT_SPAN as u64) as u16
}

// ----------------------------------------------------------------- protocol

/// Everything a client tells the host.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ToHost {
    Hello { seed: u64, name: String },
    State { st: PlayerState },
    SetBlock { x: i32, y: i32, z: i32, b: u8 },
    Chat { text: String },
    Punch { target: PlayerId, dmg: i32 },
}

/// Everything the host tells a client.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ToClient {
    /// Sent once, right after a successful `Hello`.
    Welcome {
        your_id: PlayerId,
        seed: u64,
        /// Run-length encoded voxel array (see [`rle_encode`]).
        tiles: Vec<(u8, u32)>,
        spawn: (f32, f32, f32),
        time: u64,
    },
    Reject {
        why: String,
    },
    Peer {
        id: PlayerId,
        name: String,
        st: PlayerState,
    },
    Left {
        id: PlayerId,
        name: String,
    },
    SetBlock {
        x: i32,
        y: i32,
        z: i32,
        b: u8,
    },
    Chat {
        from: String,
        text: String,
    },
    Hurt {
        from: String,
        dmg: i32,
    },
    Time {
        t: u64,
    },
}

/// The bit of a player everyone else needs in order to draw them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerState {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub hp: i32,
}

/// What the game loop gets back from [`Net::poll`].
#[derive(Clone, Debug)]
pub enum NetEvent {
    /// A peer appeared or moved.
    Peer {
        id: PlayerId,
        name: String,
        st: PlayerState,
    },
    Left {
        id: PlayerId,
    },
    /// Someone changed the world; apply it locally without re-broadcasting.
    SetBlock {
        x: i32,
        y: i32,
        z: i32,
        b: u8,
    },
    Chat {
        from: String,
        text: String,
    },
    Hurt {
        from: String,
        dmg: i32,
    },
    /// Host's authoritative world clock (clients follow it).
    Time(u64),
    /// Connection-level notice worth showing in the HUD.
    Notice(String),
    /// The link is gone; the game drops back to single player.
    Disconnected(String),
}

// ------------------------------------------------------------- world codec

/// Run-length encodes the voxel array. A freshly generated world is mostly
/// long runs of air and stone, so 1 MiB collapses to a few KB of JSON.
pub fn rle_encode(bytes: &[u8]) -> Vec<(u8, u32)> {
    let mut out: Vec<(u8, u32)> = Vec::new();
    for &b in bytes {
        match out.last_mut() {
            Some((v, n)) if *v == b && *n < u32::MAX => *n += 1,
            _ => out.push((b, 1)),
        }
    }
    out
}

pub fn rle_decode(runs: &[(u8, u32)], expect_len: usize) -> Option<Vec<u8>> {
    let total: u64 = runs.iter().map(|&(_, n)| n as u64).sum();
    if total != expect_len as u64 {
        return None;
    }
    let mut out = Vec::with_capacity(expect_len);
    for &(v, n) in runs {
        out.extend(std::iter::repeat_n(v, n as usize));
    }
    Some(out)
}

// -------------------------------------------------------------------- Net

/// One connected client, from the host's point of view.
struct Conn {
    id: PlayerId,
    name: String,
    st: PlayerState,
    /// Set once `Hello` has been accepted.
    joined: bool,
    out: Sender<ToClient>,
}

enum Inner {
    Host {
        conns: Vec<Conn>,
        rx: Receiver<HostIn>,
        /// Kept alive so `rx` never reports "disconnected" just because no
        /// client happens to be connected right now.
        #[allow(dead_code)]
        tx: Sender<HostIn>,
    },
    Client {
        rx: Receiver<ToClient>,
        tx: Sender<ToHost>,
        alive: Arc<Mutex<bool>>,
    },
}

/// Messages the host's background threads push at the game thread.
enum HostIn {
    Joined { id: PlayerId, out: Sender<ToClient> },
    Msg { id: PlayerId, msg: ToHost },
    Dropped { id: PlayerId },
}

pub struct Net {
    role: Role,
    seed: u64,
    addr: SocketAddr,
    me: String,
    inner: Inner,
    /// Filled in by the host when a client needs a world snapshot; the game
    /// thread owns the world, so it does the encoding.
    pending_snapshots: Vec<PlayerId>,
}

impl Net {
    /// Joins the session for `seed`, becoming the host if nobody else has.
    ///
    /// Returns `Err` only when neither joining nor hosting is possible; the
    /// caller then falls back to single player.
    pub fn start(seed: u64, opts: &NetOpts) -> std::io::Result<Net> {
        let name = {
            let n = sanitize_name(&opts.name);
            if n.is_empty() {
                default_name()
            } else {
                n
            }
        };
        let port = port_for_seed(seed);

        // An explicit --join address is client-only: never silently host a
        // different machine's world.
        if let Some(target) = &opts.join {
            let addr = resolve(target, port)?;
            let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
            return Self::client(seed, name, addr, stream);
        }

        // Otherwise: try to join the local session first, then host it.
        let local: SocketAddr = ([127, 0, 0, 1], port).into();
        if let Ok(stream) = TcpStream::connect_timeout(&local, Duration::from_millis(600)) {
            return Self::client(seed, name, local, stream);
        }
        let bind: SocketAddr = if opts.open {
            ([0, 0, 0, 0], port).into()
        } else {
            local
        };
        let listener = TcpListener::bind(bind)?;
        Self::host(seed, name, bind, listener)
    }

    fn host(
        seed: u64,
        me: String,
        addr: SocketAddr,
        listener: TcpListener,
    ) -> std::io::Result<Net> {
        let (tx, rx) = channel::<HostIn>();
        let accept_tx = tx.clone();
        thread::Builder::new()
            .name("termcraft-accept".into())
            .spawn(move || {
                let mut next_conn = 1u32;
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    let id = next_conn;
                    next_conn += 1;
                    let (out_tx, out_rx) = channel::<ToClient>();
                    if accept_tx.send(HostIn::Joined { id, out: out_tx }).is_err() {
                        break; // game is gone
                    }
                    spawn_writer(stream.try_clone().ok(), out_rx);
                    let reader_tx = accept_tx.clone();
                    thread::Builder::new()
                        .name(format!("termcraft-rx-{id}"))
                        .spawn(move || {
                            let reader = BufReader::new(stream);
                            for line in reader.lines() {
                                let Ok(line) = line else { break };
                                match serde_json::from_str::<ToHost>(&line) {
                                    Ok(msg) => {
                                        if reader_tx.send(HostIn::Msg { id, msg }).is_err() {
                                            return;
                                        }
                                    }
                                    Err(_) => continue, // ignore junk lines
                                }
                            }
                            let _ = reader_tx.send(HostIn::Dropped { id });
                        })
                        .ok();
                }
            })?;
        Ok(Net {
            role: Role::Host,
            seed,
            addr,
            me,
            inner: Inner::Host {
                conns: Vec::new(),
                rx,
                tx,
            },
            pending_snapshots: Vec::new(),
        })
    }

    fn client(seed: u64, me: String, addr: SocketAddr, stream: TcpStream) -> std::io::Result<Net> {
        stream.set_nodelay(true).ok();
        let (in_tx, in_rx) = channel::<ToClient>();
        let (out_tx, out_rx) = channel::<ToHost>();
        let alive = Arc::new(Mutex::new(true));

        spawn_writer_to_host(stream.try_clone()?, out_rx);
        let read_stream = stream;
        let dead = Arc::clone(&alive);
        thread::Builder::new()
            .name("termcraft-rx-host".into())
            .spawn(move || {
                let reader = BufReader::new(read_stream);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if let Ok(msg) = serde_json::from_str::<ToClient>(&line) {
                        if in_tx.send(msg).is_err() {
                            break;
                        }
                    }
                }
                if let Ok(mut a) = dead.lock() {
                    *a = false;
                }
            })?;

        let net = Net {
            role: Role::Client,
            seed,
            addr,
            me: me.clone(),
            inner: Inner::Client {
                rx: in_rx,
                tx: out_tx,
                alive,
            },
            pending_snapshots: Vec::new(),
        };
        net.send_to_host(ToHost::Hello { seed, name: me });
        Ok(net)
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn name(&self) -> &str {
        &self.me
    }

    /// True when this end owns the world (and therefore the save file).
    pub fn is_authority(&self) -> bool {
        self.role == Role::Host
    }

    /// Blocks (briefly) for the host's `Welcome`, returning the world bytes,
    /// spawn point and world clock. Clients only.
    pub fn await_world(&mut self, expect_len: usize) -> Result<WorldSnapshot, String> {
        let Inner::Client { rx, .. } = &self.inner else {
            return Err("not a client".into());
        };
        let deadline = std::time::Instant::now() + JOIN_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err("timed out waiting for the host's world".into());
            }
            match rx.recv_timeout(left) {
                Ok(ToClient::Welcome {
                    seed,
                    tiles,
                    spawn,
                    time,
                    ..
                }) => {
                    if seed != self.seed {
                        return Err(format!(
                            "host is running seed {seed}, you asked for {}",
                            self.seed
                        ));
                    }
                    let bytes = rle_decode(&tiles, expect_len)
                        .ok_or_else(|| "host sent a malformed world".to_string())?;
                    return Ok((bytes, spawn, time));
                }
                Ok(ToClient::Reject { why }) => return Err(why),
                // Anything else this early is chatter from before we loaded.
                Ok(_) => continue,
                Err(_) => return Err("lost the connection while joining".into()),
            }
        }
    }

    // --------------------------------------------------------------- sending

    fn send_to_host(&self, msg: ToHost) {
        if let Inner::Client { tx, .. } = &self.inner {
            let _ = tx.send(msg);
        }
    }

    /// Host-side fan-out, optionally skipping the connection it came from.
    fn broadcast(&mut self, msg: ToClient, except: Option<PlayerId>) {
        if let Inner::Host { conns, .. } = &mut self.inner {
            for c in conns.iter() {
                if c.joined && Some(c.id) != except {
                    let _ = c.out.send(msg.clone());
                }
            }
        }
    }

    pub fn send_state(&mut self, st: PlayerState) {
        match self.role {
            Role::Client => self.send_to_host(ToHost::State { st }),
            Role::Host => {
                let (name, id) = (self.me.clone(), 0);
                self.broadcast(ToClient::Peer { id, name, st }, None);
            }
        }
    }

    pub fn send_block(&mut self, x: i32, y: i32, z: i32, b: u8) {
        match self.role {
            Role::Client => self.send_to_host(ToHost::SetBlock { x, y, z, b }),
            Role::Host => self.broadcast(ToClient::SetBlock { x, y, z, b }, None),
        }
    }

    pub fn send_chat(&mut self, text: &str) {
        match self.role {
            Role::Client => self.send_to_host(ToHost::Chat {
                text: text.to_string(),
            }),
            Role::Host => {
                let from = self.me.clone();
                self.broadcast(
                    ToClient::Chat {
                        from,
                        text: text.to_string(),
                    },
                    None,
                );
            }
        }
    }

    /// Punches `target`. On the host this is applied directly; on a client the
    /// host relays it.
    pub fn send_punch(&mut self, target: PlayerId, dmg: i32) {
        match self.role {
            Role::Client => self.send_to_host(ToHost::Punch { target, dmg }),
            Role::Host => {
                let from = self.me.clone();
                self.hurt(target, &from, dmg);
            }
        }
    }

    fn hurt(&mut self, target: PlayerId, from: &str, dmg: i32) {
        if let Inner::Host { conns, .. } = &mut self.inner {
            if let Some(c) = conns.iter().find(|c| c.id == target && c.joined) {
                let _ = c.out.send(ToClient::Hurt {
                    from: from.to_string(),
                    dmg,
                });
            }
        }
    }

    /// Host only: publish the world clock so day/night stays in sync.
    pub fn send_time(&mut self, t: u64) {
        if self.role == Role::Host {
            self.broadcast(ToClient::Time { t }, None);
        }
    }

    /// Host only: ids that are waiting for a world snapshot. The caller
    /// encodes the world (it owns it) and answers with [`Net::send_snapshot`].
    pub fn take_snapshot_requests(&mut self) -> Vec<PlayerId> {
        std::mem::take(&mut self.pending_snapshots)
    }

    pub fn send_snapshot(&mut self, to: PlayerId, tiles: &[u8], spawn: (f32, f32, f32), time: u64) {
        let seed = self.seed;
        let runs = rle_encode(tiles);
        if let Inner::Host { conns, .. } = &mut self.inner {
            if let Some(c) = conns.iter().find(|c| c.id == to) {
                let _ = c.out.send(ToClient::Welcome {
                    your_id: to,
                    seed,
                    tiles: runs,
                    spawn,
                    time,
                });
            }
        }
    }

    // --------------------------------------------------------------- polling

    /// Drains everything that arrived since the last call. Never blocks.
    pub fn poll(&mut self) -> Vec<NetEvent> {
        let mut out = Vec::new();
        match self.role {
            Role::Host => self.poll_host(&mut out),
            Role::Client => self.poll_client(&mut out),
        }
        out
    }

    fn poll_host(&mut self, out: &mut Vec<NetEvent>) {
        // Collect first so we can mutate `self` (broadcasts) afterwards.
        let mut inbox = Vec::new();
        if let Inner::Host { rx, .. } = &self.inner {
            while let Ok(m) = rx.try_recv() {
                inbox.push(m);
            }
        }
        for m in inbox {
            match m {
                HostIn::Joined { id, out: tx } => {
                    if let Inner::Host { conns, .. } = &mut self.inner {
                        conns.push(Conn {
                            id,
                            name: String::new(),
                            st: PlayerState::default(),
                            joined: false,
                            out: tx,
                        });
                    }
                }
                HostIn::Msg { id, msg } => self.host_handle(id, msg, out),
                HostIn::Dropped { id } => {
                    let mut gone = None;
                    if let Inner::Host { conns, .. } = &mut self.inner {
                        if let Some(pos) = conns.iter().position(|c| c.id == id) {
                            let c = conns.remove(pos);
                            if c.joined {
                                gone = Some(c.name);
                            }
                        }
                    }
                    if let Some(name) = gone {
                        out.push(NetEvent::Left { id });
                        out.push(NetEvent::Notice(format!("{name} left the world.")));
                        self.broadcast(ToClient::Left { id, name }, None);
                    }
                }
            }
        }
    }

    fn host_handle(&mut self, id: PlayerId, msg: ToHost, out: &mut Vec<NetEvent>) {
        match msg {
            ToHost::Hello { seed, name } => {
                if seed != self.seed {
                    let why = format!("this world is seed {}, not {seed}", self.seed);
                    if let Inner::Host { conns, .. } = &mut self.inner {
                        if let Some(c) = conns.iter().find(|c| c.id == id) {
                            let _ = c.out.send(ToClient::Reject { why });
                        }
                    }
                    return;
                }
                let name = {
                    let n = sanitize_name(&name);
                    if n.is_empty() {
                        format!("guest{id}")
                    } else {
                        n
                    }
                };
                // Make sure names stay distinguishable in the player list.
                let name = if let Inner::Host { conns, .. } = &self.inner {
                    let clash = conns.iter().any(|c| c.joined && c.name == name) || name == self.me;
                    if clash {
                        format!("{name}#{id}")
                    } else {
                        name
                    }
                } else {
                    name
                };
                let mut existing: Vec<(PlayerId, String, PlayerState)> = Vec::new();
                if let Inner::Host { conns, .. } = &mut self.inner {
                    if let Some(c) = conns.iter_mut().find(|c| c.id == id) {
                        c.joined = true;
                        c.name = name.clone();
                    }
                    existing = conns
                        .iter()
                        .filter(|c| c.joined && c.id != id)
                        .map(|c| (c.id, c.name.clone(), c.st))
                        .collect();
                }
                self.pending_snapshots.push(id);
                out.push(NetEvent::Notice(format!("{name} joined the world.")));
                // Tell the newcomer who is already here...
                if let Inner::Host { conns, .. } = &self.inner {
                    if let Some(c) = conns.iter().find(|c| c.id == id) {
                        for (pid, pname, st) in existing {
                            let _ = c.out.send(ToClient::Peer {
                                id: pid,
                                name: pname,
                                st,
                            });
                        }
                    }
                }
                // ...and let everyone else know about them.
                self.broadcast(
                    ToClient::Chat {
                        from: "*".into(),
                        text: format!("{name} joined"),
                    },
                    Some(id),
                );
            }
            ToHost::State { st } => {
                let mut name = None;
                if let Inner::Host { conns, .. } = &mut self.inner {
                    if let Some(c) = conns.iter_mut().find(|c| c.id == id && c.joined) {
                        c.st = st;
                        name = Some(c.name.clone());
                    }
                }
                if let Some(name) = name {
                    out.push(NetEvent::Peer {
                        id,
                        name: name.clone(),
                        st,
                    });
                    self.broadcast(ToClient::Peer { id, name, st }, Some(id));
                }
            }
            ToHost::SetBlock { x, y, z, b } => {
                out.push(NetEvent::SetBlock { x, y, z, b });
                self.broadcast(ToClient::SetBlock { x, y, z, b }, Some(id));
            }
            ToHost::Chat { text } => {
                let from = self.conn_name(id).unwrap_or_else(|| format!("guest{id}"));
                let text = trim_chat(&text);
                out.push(NetEvent::Chat {
                    from: from.clone(),
                    text: text.clone(),
                });
                self.broadcast(ToClient::Chat { from, text }, Some(id));
            }
            ToHost::Punch { target, dmg } => {
                let from = self.conn_name(id).unwrap_or_else(|| format!("guest{id}"));
                let dmg = dmg.clamp(0, 4);
                if target == 0 {
                    out.push(NetEvent::Hurt { from, dmg }); // the host got hit
                } else {
                    self.hurt(target, &from, dmg);
                }
            }
        }
    }

    fn conn_name(&self, id: PlayerId) -> Option<String> {
        if let Inner::Host { conns, .. } = &self.inner {
            conns
                .iter()
                .find(|c| c.id == id && c.joined)
                .map(|c| c.name.clone())
        } else {
            None
        }
    }

    fn poll_client(&mut self, out: &mut Vec<NetEvent>) {
        let mut lost = false;
        if let Inner::Client { rx, alive, .. } = &self.inner {
            loop {
                match rx.try_recv() {
                    Ok(ToClient::Peer { id, name, st }) => {
                        out.push(NetEvent::Peer { id, name, st })
                    }
                    Ok(ToClient::Left { id, name }) => {
                        out.push(NetEvent::Left { id });
                        out.push(NetEvent::Notice(format!("{name} left the world.")));
                    }
                    Ok(ToClient::SetBlock { x, y, z, b }) => {
                        out.push(NetEvent::SetBlock { x, y, z, b })
                    }
                    Ok(ToClient::Chat { from, text }) => out.push(NetEvent::Chat { from, text }),
                    Ok(ToClient::Hurt { from, dmg }) => out.push(NetEvent::Hurt {
                        from,
                        dmg: dmg.clamp(0, 4),
                    }),
                    Ok(ToClient::Time { t }) => out.push(NetEvent::Time(t)),
                    Ok(ToClient::Reject { why }) => out.push(NetEvent::Disconnected(why)),
                    Ok(ToClient::Welcome { .. }) => {} // already handled at join
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        lost = true;
                        break;
                    }
                }
            }
            if !lost && !*alive.lock().unwrap_or_else(|e| e.into_inner()) {
                lost = true;
            }
        }
        if lost {
            out.push(NetEvent::Disconnected("the host closed the world".into()));
        }
    }

    /// Host: number of joined clients. Client: 1 (the host).
    #[cfg(test)]
    pub fn peer_count(&self) -> usize {
        match &self.inner {
            Inner::Host { conns, .. } => conns.iter().filter(|c| c.joined).count(),
            Inner::Client { .. } => 1,
        }
    }
}

fn trim_chat(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect::<String>()
        .trim()
        .to_string()
}

fn resolve(target: &str, default_port: u16) -> std::io::Result<SocketAddr> {
    let with_port = if target.contains(':') {
        target.to_string()
    } else {
        format!("{target}:{default_port}")
    };
    with_port
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::other(format!("could not resolve {target}")))
}

/// Serializes `ToClient` messages onto a socket from a dedicated thread.
fn spawn_writer(stream: Option<TcpStream>, rx: Receiver<ToClient>) {
    let Some(mut stream) = stream else { return };
    stream.set_nodelay(true).ok();
    thread::Builder::new()
        .name("termcraft-tx".into())
        .spawn(move || {
            while let Ok(msg) = rx.recv() {
                let Ok(mut line) = serde_json::to_vec(&msg) else {
                    continue;
                };
                line.push(b'\n');
                if stream.write_all(&line).is_err() {
                    break;
                }
            }
            let _ = stream.flush();
        })
        .ok();
}

fn spawn_writer_to_host(mut stream: TcpStream, rx: Receiver<ToHost>) {
    thread::Builder::new()
        .name("termcraft-tx-host".into())
        .spawn(move || {
            while let Ok(msg) = rx.recv() {
                let Ok(mut line) = serde_json::to_vec(&msg) else {
                    continue;
                };
                line.push(b'\n');
                if stream.write_all(&line).is_err() {
                    break;
                }
            }
            let _ = stream.flush();
        })
        .ok();
}

/// Cache of who is nearby, kept by the game and drawn by the renderer.
#[derive(Clone, Debug)]
pub struct Peer {
    pub id: PlayerId,
    pub name: String,
    pub st: PlayerState,
    /// Tick of the last update, used to evict silent peers.
    pub last_seen: u64,
}

impl Peer {
    /// A stable, readable color per player so avatars are tellable apart.
    pub fn tint(&self) -> (u8, u8, u8) {
        const PALETTE: [(u8, u8, u8); 8] = [
            (232, 92, 92),
            (92, 156, 232),
            (108, 216, 128),
            (236, 192, 84),
            (196, 124, 236),
            (96, 216, 216),
            (240, 148, 96),
            (216, 216, 232),
        ];
        let mut h = self.id as usize;
        for b in self.name.bytes() {
            h = h.wrapping_mul(31).wrapping_add(b as usize);
        }
        PALETTE[h % PALETTE.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_port_different_seed_different_port() {
        assert_eq!(port_for_seed(7), port_for_seed(7));
        assert_ne!(port_for_seed(7), port_for_seed(8));
        for seed in [0u64, 1, 42, u64::MAX] {
            let p = port_for_seed(seed);
            assert!((PORT_BASE..PORT_BASE + PORT_SPAN).contains(&p));
        }
    }

    #[test]
    fn rle_roundtrips() {
        let bytes: Vec<u8> = (0..5000u32)
            .map(|i| if i < 4000 { 0 } else { (i % 7) as u8 })
            .collect();
        let runs = rle_encode(&bytes);
        assert!(runs.len() < bytes.len());
        assert_eq!(rle_decode(&runs, bytes.len()).unwrap(), bytes);
        // Length mismatches are rejected rather than trusted.
        assert!(rle_decode(&runs, bytes.len() + 1).is_none());
    }

    #[test]
    fn names_are_sanitized() {
        assert_eq!(sanitize_name("Vik Vang!!"), "VikVang");
        assert_eq!(sanitize_name("\u{1b}[31mred"), "31mred");
        assert_eq!(sanitize_name("aaaaaaaaaaaaaaaaaaaaaa").len(), 16);
    }

    /// Spins up a host and a client on a real loopback socket and checks the
    /// handshake plus edit propagation in both directions.
    #[test]
    fn host_and_client_exchange_state_and_edits() {
        // Use a seed unlikely to clash with a running game.
        let seed = 0xC0FF_EE00_1234_5678;
        let opts = NetOpts {
            name: "host".into(),
            join: None,
            open: false,
        };
        let mut host = Net::start(seed, &opts).expect("host binds");
        assert_eq!(host.role(), Role::Host);
        assert!(host.is_authority());

        let client_handle = thread::spawn(move || {
            let mut c = Net::start(
                seed,
                &NetOpts {
                    name: "guest".into(),
                    join: None,
                    open: false,
                },
            )
            .expect("client connects");
            assert_eq!(c.role(), Role::Client);
            let world = c.await_world(16).expect("gets a world");
            assert_eq!(world.0.len(), 16);
            c.send_block(1, 2, 3, 9);
            c
        });

        // Host loop: answer the snapshot request, then wait for the edit.
        let mut edit = None;
        let mut joined = false;
        for _ in 0..400 {
            for ev in host.poll() {
                match ev {
                    NetEvent::Notice(n) => {
                        assert!(n.contains("guest"));
                        joined = true;
                    }
                    NetEvent::SetBlock { x, y, z, b } => edit = Some((x, y, z, b)),
                    _ => {}
                }
            }
            for id in host.take_snapshot_requests() {
                host.send_snapshot(id, &[3u8; 16], (1.0, 2.0, 3.0), 99);
            }
            if edit.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(joined, "host never saw the join");
        assert_eq!(edit, Some((1, 2, 3, 9)));
        assert_eq!(host.peer_count(), 1);

        // And the other direction: host edit reaches the client.
        let mut client = client_handle.join().unwrap();
        host.send_block(4, 5, 6, 2);
        let mut got = None;
        for _ in 0..200 {
            for ev in client.poll() {
                if let NetEvent::SetBlock { x, y, z, b } = ev {
                    got = Some((x, y, z, b));
                }
            }
            if got.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got, Some((4, 5, 6, 2)));
    }

    #[test]
    fn seed_mismatch_is_rejected() {
        let seed = 0xC0FF_EE00_8765_4321;
        let mut host = Net::start(seed, &NetOpts::default()).expect("host binds");
        let port = port_for_seed(seed);
        // Keep the host serving while the client waits for its world.
        let pump = thread::spawn(move || {
            for _ in 0..300 {
                host.poll();
                for id in host.take_snapshot_requests() {
                    host.send_snapshot(id, &[0u8; 16], (0.0, 0.0, 0.0), 0);
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let stream = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], port))).unwrap();
        let mut wrong = Net::client(
            seed + 1,
            "guest".into(),
            stream.peer_addr().unwrap(),
            stream,
        )
        .expect("connects");
        let why = wrong
            .await_world(16)
            .expect_err("client with the wrong seed was let in");
        assert!(why.contains("seed"), "unexpected reason: {why}");
        drop(wrong);
        pump.join().unwrap();
    }
}
