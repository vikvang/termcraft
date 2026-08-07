use std::collections::BTreeMap;
use std::path::PathBuf;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use serde::{Deserialize, Serialize};

use crate::block::Block;
use crate::game::{KeyHold, DAY_LEN, PLAYER_MAX_HP, RECIPES};
use crate::net::{Net, NetEvent, Peer, PlayerId, PlayerState, Role};
use crate::world3::{World3, H3};

pub const REACH3: f32 = 5.0;
pub const EYE_HEIGHT: f32 = 1.62;
pub const PLAYER_HALF_W: f32 = 0.3;
pub const PLAYER_H: f32 = 1.8;

const GRAVITY: f32 = 0.04;
const JUMP_VEL: f32 = 0.34;
const MAX_FALL: f32 = -1.0;
const MOVE_SPEED: f32 = 0.18;
const SWIM_UP: f32 = 0.16;
const SWIM_MAX_RISE: f32 = 0.35;
const SAFE_FALL: f32 = -0.62;
const LOOK_STEP: f32 = 0.10;
/// Damage one punch deals to another player.
const PUNCH_DMG: i32 = 2;
/// Peers we haven't heard from in this many ticks are assumed gone.
const PEER_TIMEOUT: u64 = 60;
/// Republish our state at least this often, even standing perfectly still,
/// so nobody times us out and newcomers see us right away.
const STATE_HEARTBEAT: u64 = 10;
/// How often the host publishes its world clock.
const TIME_SYNC_EVERY: u64 = 100;
/// Chat lines older than this stop being drawn.
pub const CHAT_TTL: u64 = 400;
pub const CHAT_LINES: usize = 6;
/// Half-width of another player's hitbox (slightly wider than the collider so
/// punches connect the way you'd expect).
const PEER_HALF_W: f32 = 0.35;

/// A line in the chat log: who said it, what they said, and when.
pub type ChatLine = (String, String, u64);

/// Slab test of a ray against an axis-aligned box. Returns the entry distance.
pub fn ray_aabb(
    o: (f32, f32, f32),
    d: (f32, f32, f32),
    min: (f32, f32, f32),
    max: (f32, f32, f32),
) -> Option<f32> {
    let mut t0 = 0.0f32;
    let mut t1 = f32::INFINITY;
    let axes = [
        (o.0, d.0, min.0, max.0),
        (o.1, d.1, min.1, max.1),
        (o.2, d.2, min.2, max.2),
    ];
    for (o, d, lo, hi) in axes {
        if d.abs() < 1e-6 {
            if o < lo || o > hi {
                return None; // parallel and outside the slab
            }
            continue;
        }
        let (mut a, mut b) = ((lo - o) / d, (hi - o) / d);
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// The world-space box a peer's avatar occupies.
pub fn peer_box(st: &PlayerState) -> ((f32, f32, f32), (f32, f32, f32)) {
    (
        (st.x - PEER_HALF_W, st.y, st.z - PEER_HALF_W),
        (st.x + PEER_HALF_W, st.y + PLAYER_H, st.z + PEER_HALF_W),
    )
}

/// True when a peer is standing in the cell someone is trying to build in.
fn peer_overlaps_cell(p: &Peer, x: i32, y: i32, z: i32) -> bool {
    let (min, max) = peer_box(&p.st);
    min.0 < (x + 1) as f32
        && max.0 > x as f32
        && min.1 < (y + 1) as f32
        && max.1 > y as f32
        && min.2 < (z + 1) as f32
        && max.2 > z as f32
}

/// A block hit by a ray: cell coords, the face normal it entered through, and distance.
pub struct Hit {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub nx: i32,
    pub ny: i32,
    pub nz: i32,
    pub block: Block,
    #[allow(dead_code)]
    pub t: f32,
}

/// DDA voxel raycast; hits anything that isn't air or water.
pub fn raycast(world: &World3, o: (f32, f32, f32), d: (f32, f32, f32), max_t: f32) -> Option<Hit> {
    let (mut ix, mut iy, mut iz) = (o.0.floor() as i32, o.1.floor() as i32, o.2.floor() as i32);
    let step = (
        d.0.signum() as i32,
        d.1.signum() as i32,
        d.2.signum() as i32,
    );
    let inv = |v: f32| {
        if v != 0.0 {
            (1.0 / v).abs()
        } else {
            f32::INFINITY
        }
    };
    let t_delta = (inv(d.0), inv(d.1), inv(d.2));
    let frac = |o: f32, d: f32, i: i32| -> f32 {
        if d > 0.0 {
            ((i + 1) as f32 - o) / d
        } else if d < 0.0 {
            (i as f32 - o) / d
        } else {
            f32::INFINITY
        }
    };
    let mut t_max = (frac(o.0, d.0, ix), frac(o.1, d.1, iy), frac(o.2, d.2, iz));
    let mut normal = (0, 0, 0);
    let mut t = 0.0f32;
    while t <= max_t {
        let b = world.get(ix, iy, iz);
        if b != Block::Air && b != Block::Water && t > 0.0 {
            return Some(Hit {
                x: ix,
                y: iy,
                z: iz,
                nx: normal.0,
                ny: normal.1,
                nz: normal.2,
                block: b,
                t,
            });
        }
        if t_max.0 <= t_max.1 && t_max.0 <= t_max.2 {
            t = t_max.0;
            t_max.0 += t_delta.0;
            ix += step.0;
            normal = (-step.0, 0, 0);
        } else if t_max.1 <= t_max.2 {
            t = t_max.1;
            t_max.1 += t_delta.1;
            iy += step.1;
            normal = (0, -step.1, 0);
        } else {
            t = t_max.2;
            t_max.2 += t_delta.2;
            iz += step.2;
            normal = (0, 0, -step.2);
        }
    }
    None
}

pub struct Game3 {
    pub world: World3,
    pub seed: u64,
    // player (pos = feet center)
    pub px: f32,
    pub py: f32,
    pub pz: f32,
    pub vx: f32,
    pub vy: f32,
    pub vz: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    /// Creative: always flying, no gravity/fall damage, look-relative fly.
    pub creative: bool,
    pub hp: i32,
    // items
    pub inv: BTreeMap<Block, u32>,
    pub hotbar: [Option<Block>; 9],
    pub selected: usize,
    // state
    pub time: u64,
    pub should_quit: bool,
    pub crafting_open: bool,
    pub craft_sel: usize,
    pub help_open: bool,
    pub msg: Option<(String, u64)>,
    pub game_over: bool,
    // multiplayer
    /// `None` in single player, or after the link drops.
    pub net: Option<Net>,
    pub peers: BTreeMap<PlayerId, Peer>,
    pub name: String,
    pub chat: Vec<ChatLine>,
    pub chat_open: bool,
    pub chat_input: String,
    pub players_open: bool,
    /// Last state published to the session, so we only send real changes.
    last_sent: PlayerState,
    /// Where this session saves, and whether it's allowed to (guests must not
    /// clobber the host's file).
    save_path: PathBuf,
    owns_save: bool,
    move_fwd: f32,
    move_strafe: f32,
    move_timer: u32,
    /// True when the terminal reports key release events (kitty protocol),
    /// enabling continuous hold-to-move instead of per-keypress nudges.
    hold_mode: bool,
    held_w: KeyHold,
    held_s: KeyHold,
    held_a: KeyHold,
    held_d: KeyHold,
    held_jump: KeyHold,
    /// Descend while flying in creative mode.
    held_down: KeyHold,
    /// Vertical fly intent for non-hold terminals (+1 up / -1 down).
    fly_vertical: f32,
    /// Set once any key release event arrives, proving the terminal
    /// actually reports them.
    saw_release: bool,
    /// True while swimming and pushing horizontally against a solid block
    /// (used to kick the player up so they can climb out of water).
    swim_blocked: bool,
    last_damage_tick: u64,
    last_mouse: Option<(u16, u16)>,
}

impl Game3 {
    pub fn new(seed: u64) -> Game3 {
        let mut g = Game3::from_world(seed, World3::generate(seed));
        g.say("Welcome to TermCraft 3D! Press h for help, x to mine.");
        g
    }

    /// Builds a fresh session around an existing world (used when a guest
    /// receives the host's world snapshot).
    pub fn from_world(seed: u64, world: World3) -> Game3 {
        let (sx, sy, sz) = world.spawn;
        Game3 {
            world,
            seed,
            px: sx,
            py: sy,
            pz: sz,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
            creative: false,
            hp: PLAYER_MAX_HP,
            inv: BTreeMap::new(),
            hotbar: [None; 9],
            selected: 0,
            time: 0,
            should_quit: false,
            crafting_open: false,
            craft_sel: 0,
            help_open: false,
            msg: None,
            game_over: false,
            move_fwd: 0.0,
            move_strafe: 0.0,
            move_timer: 0,
            hold_mode: false,
            held_w: KeyHold::default(),
            held_s: KeyHold::default(),
            held_a: KeyHold::default(),
            held_d: KeyHold::default(),
            held_jump: KeyHold::default(),
            held_down: KeyHold::default(),
            fly_vertical: 0.0,
            saw_release: false,
            swim_blocked: false,
            last_damage_tick: 0,
            last_mouse: None,
            net: None,
            peers: BTreeMap::new(),
            name: crate::net::default_name(),
            chat: Vec::new(),
            chat_open: false,
            chat_input: String::new(),
            players_open: false,
            last_sent: PlayerState::default(),
            save_path: save3_path(),
            owns_save: true,
        }
    }

    pub fn say(&mut self, s: &str) {
        self.msg = Some((s.to_string(), self.time + 80));
    }

    pub fn set_hold_mode(&mut self, on: bool) {
        self.hold_mode = on;
    }

    /// Creative mode: always flying, no gravity or fall damage.
    pub fn set_creative(&mut self, on: bool) {
        self.creative = on;
        if on {
            self.vy = 0.0;
            self.on_ground = false;
            self.say("Creative mode: WASD fly, space up, f down.");
        }
    }

    pub fn daylight(&self) -> f32 {
        let t = (self.time % DAY_LEN) as f32;
        match t {
            t if t < 1000.0 => 1.0,
            t if t < 1200.0 => 1.0 - 0.85 * (t - 1000.0) / 200.0,
            t if t < 2200.0 => 0.15,
            t => 0.15 + 0.85 * (t - 2200.0) / 200.0,
        }
    }

    pub fn is_night(&self) -> bool {
        self.daylight() < 0.3
    }

    pub fn day_number(&self) -> u64 {
        self.time / DAY_LEN + 1
    }

    pub fn eye(&self) -> (f32, f32, f32) {
        (self.px, self.py + EYE_HEIGHT, self.pz)
    }

    /// Unit view direction from yaw/pitch (y-up).
    pub fn forward(&self) -> (f32, f32, f32) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        (cy * cp, sp, sy * cp)
    }

    pub fn target(&self) -> Option<Hit> {
        raycast(&self.world, self.eye(), self.forward(), REACH3)
    }

    // ---------------------------------------------------------- multiplayer

    /// Joins this session to a network link. Also adopts the link's player
    /// name, which is what other people see.
    pub fn attach_net(&mut self, net: Net) {
        self.name = net.name().to_string();
        let (role, addr, seed) = (net.role(), net.addr(), self.seed);
        self.net = Some(net);
        let m = match role {
            Role::Host => format!("Hosting seed {seed} on {addr} as {}.", self.name),
            Role::Client => format!("Joined seed {seed} at {addr} as {}.", self.name),
        };
        self.log_chat("*", &m);
        self.say(&m);
    }

    /// Points this session at a specific save file. Guests pass `owns: false`
    /// so they never overwrite the host's world.
    pub fn set_save(&mut self, path: PathBuf, owns: bool) {
        self.save_path = path;
        self.owns_save = owns;
    }

    pub fn owns_save(&self) -> bool {
        self.owns_save
    }

    pub fn save_file(&self) -> &std::path::Path {
        &self.save_path
    }

    pub fn is_multiplayer(&self) -> bool {
        self.net.is_some()
    }

    /// Short badge for the HUD, e.g. `host 2`.
    pub fn net_badge(&self) -> Option<String> {
        let net = self.net.as_ref()?;
        Some(format!(
            "{} {}",
            net.role().label(),
            self.peers.len().max(0)
        ))
    }

    pub fn log_chat(&mut self, from: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        self.chat
            .push((from.to_string(), text.to_string(), self.time));
        // Keep the log bounded; only the tail is ever drawn.
        if self.chat.len() > 64 {
            let drop = self.chat.len() - 64;
            self.chat.drain(0..drop);
        }
    }

    /// Chat lines still worth drawing, oldest first.
    pub fn recent_chat(&self) -> Vec<&ChatLine> {
        let mut lines: Vec<&ChatLine> = self
            .chat
            .iter()
            .filter(|(_, _, t)| self.time.saturating_sub(*t) < CHAT_TTL)
            .collect();
        if lines.len() > CHAT_LINES {
            lines.drain(0..lines.len() - CHAT_LINES);
        }
        lines
    }

    fn send_chat_line(&mut self) {
        let text = self.chat_input.trim().to_string();
        self.chat_input.clear();
        self.chat_open = false;
        if text.is_empty() {
            return;
        }
        let me = self.name.clone();
        self.log_chat(&me, &text);
        if let Some(net) = &mut self.net {
            net.send_chat(&text);
        }
    }

    /// The peer under the crosshair, if one is closer than any block.
    pub fn peer_in_crosshair(&self) -> Option<(PlayerId, f32)> {
        let eye = self.eye();
        let dir = self.forward();
        let wall = self.target().map(|h| h.t).unwrap_or(f32::INFINITY);
        let mut best: Option<(PlayerId, f32)> = None;
        for p in self.peers.values() {
            let (min, max) = peer_box(&p.st);
            let Some(t) = ray_aabb(eye, dir, min, max) else {
                continue;
            };
            if t > REACH3 || t > wall {
                continue;
            }
            if best.is_none_or(|(_, bt)| t < bt) {
                best = Some((p.id, t));
            }
        }
        best
    }

    /// Applies a block change locally and tells the session about it.
    fn set_block_synced(&mut self, x: i32, y: i32, z: i32, b: Block) {
        self.world.set(x, y, z, b);
        if let Some(net) = &mut self.net {
            net.send_block(x, y, z, b.to_u8());
        }
    }

    fn take_damage(&mut self, dmg: i32, from: &str) {
        if dmg <= 0 {
            return;
        }
        self.hp -= dmg;
        self.last_damage_tick = self.time;
        let m = format!("{from} hit you!");
        self.say(&m);
        if self.hp <= 0 {
            self.game_over = true;
        }
    }

    /// Drains the network, applies what came in, and publishes our own state.
    fn net_sync(&mut self) {
        let Some(mut net) = self.net.take() else {
            return;
        };

        // The world lives here, so the host encodes snapshots for joiners.
        for id in net.take_snapshot_requests() {
            let tiles = self.world.to_bytes();
            net.send_snapshot(id, &tiles, self.world.spawn, self.time);
        }

        let mut dropped = None;
        for ev in net.poll() {
            match ev {
                NetEvent::Peer { id, name, st } => {
                    let entry = self.peers.entry(id).or_insert_with(|| Peer {
                        id,
                        name: name.clone(),
                        st,
                        last_seen: self.time,
                    });
                    entry.name = name;
                    entry.st = st;
                    entry.last_seen = self.time;
                }
                NetEvent::Left { id } => {
                    self.peers.remove(&id);
                }
                NetEvent::SetBlock { x, y, z, b } => {
                    // Remote edit: apply it, don't echo it back.
                    self.world.set(x, y, z, Block::from_u8(b));
                }
                NetEvent::Chat { from, text } => self.log_chat(&from, &text),
                NetEvent::Hurt { from, dmg } => self.take_damage(dmg, &from),
                NetEvent::Time(t) => {
                    if !net.is_authority() {
                        self.time = t;
                    }
                }
                NetEvent::Notice(n) => {
                    self.log_chat("*", &n);
                    self.say(&n);
                }
                NetEvent::Disconnected(why) => dropped = Some(why),
            }
        }

        // Forget peers that went quiet (crash, kill -9, network drop).
        let now = self.time;
        self.peers
            .retain(|_, p| now.saturating_sub(p.last_seen) < PEER_TIMEOUT);

        let st = PlayerState {
            x: self.px,
            y: self.py,
            z: self.pz,
            yaw: self.yaw,
            pitch: self.pitch,
            hp: self.hp,
        };
        if st != self.last_sent || self.time.is_multiple_of(STATE_HEARTBEAT) {
            net.send_state(st);
            self.last_sent = st;
        }
        if net.is_authority() && self.time.is_multiple_of(TIME_SYNC_EVERY) {
            net.send_time(self.time);
        }

        match dropped {
            Some(why) => {
                self.peers.clear();
                let m = format!("Multiplayer ended: {why}. Playing solo.");
                self.log_chat("*", &m);
                self.say(&m);
            }
            None => self.net = Some(net),
        }
    }

    // ------------------------------------------------------------- inventory

    pub fn count(&self, b: Block) -> u32 {
        self.inv.get(&b).copied().unwrap_or(0)
    }

    pub fn add_item(&mut self, b: Block, n: u32) {
        *self.inv.entry(b).or_insert(0) += n;
        if !self.hotbar.contains(&Some(b)) {
            if let Some(slot) = self.hotbar.iter_mut().find(|s| s.is_none()) {
                *slot = Some(b);
            }
        }
    }

    pub fn remove_item(&mut self, b: Block, n: u32) -> bool {
        match self.inv.get_mut(&b) {
            Some(c) if *c >= n => {
                *c -= n;
                true
            }
            _ => false,
        }
    }

    pub fn can_craft(&self, i: usize) -> bool {
        RECIPES[i].cost.iter().all(|&(b, n)| self.count(b) >= n)
    }

    fn craft(&mut self) {
        let r = &RECIPES[self.craft_sel];
        if !r.cost.iter().all(|&(b, n)| self.count(b) >= n) {
            self.say("Not enough materials.");
            return;
        }
        for &(b, n) in r.cost {
            self.remove_item(b, n);
        }
        self.add_item(r.out, r.out_n);
        let m = format!("Crafted {}x {}!", r.out_n, r.out.name());
        self.say(&m);
    }

    // ------------------------------------------------------------- actions

    fn mine(&mut self) {
        // Another player standing between you and the block takes the hit.
        if let Some((id, _)) = self.peer_in_crosshair() {
            self.punch(id);
            return;
        }
        let Some(hit) = self.target() else {
            self.say("Nothing in reach.");
            return;
        };
        if !hit.block.is_minable() {
            if hit.block == Block::Bedrock {
                self.say("Bedrock is unbreakable.");
            }
            return;
        }
        self.set_block_synced(hit.x, hit.y, hit.z, Block::Air);
        if let Some(drop) = hit.block.drops() {
            self.add_item(drop, 1);
            let m = format!("+1 {}", drop.name());
            self.say(&m);
        }
    }

    fn punch(&mut self, id: PlayerId) {
        let name = self
            .peers
            .get(&id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "someone".to_string());
        if let Some(net) = &mut self.net {
            net.send_punch(id, PUNCH_DMG);
        }
        let m = format!("You punched {name}!");
        self.say(&m);
    }

    fn place(&mut self) {
        let Some(b) = self.hotbar[self.selected] else {
            self.say("Empty hotbar slot - select a block with 1-9.");
            return;
        };
        if self.count(b) == 0 {
            let m = format!("Out of {}.", b.name());
            self.say(&m);
            return;
        }
        let Some(hit) = self.target() else {
            self.say("Aim at a block face to place against.");
            return;
        };
        let (tx, ty, tz) = (hit.x + hit.nx, hit.y + hit.ny, hit.z + hit.nz);
        if !(0..H3).contains(&ty) {
            return;
        }
        let dest = self.world.get(tx, ty, tz);
        if dest != Block::Air && dest != Block::Water {
            return;
        }
        if b == Block::Torch && dest == Block::Water {
            self.say("Torches need air.");
            return;
        }
        if b.is_solid() && self.aabb_overlaps_cell(tx, ty, tz) {
            self.say("You're standing there!");
            return;
        }
        if b.is_solid() {
            if let Some(p) = self
                .peers
                .values()
                .find(|p| peer_overlaps_cell(p, tx, ty, tz))
            {
                let m = format!("{} is standing there!", p.name);
                self.say(&m);
                return;
            }
        }
        self.remove_item(b, 1);
        self.set_block_synced(tx, ty, tz, b);
    }

    fn aabb_overlaps_cell(&self, x: i32, y: i32, z: i32) -> bool {
        self.px - PLAYER_HALF_W < (x + 1) as f32
            && self.px + PLAYER_HALF_W > x as f32
            && self.py < (y + 1) as f32
            && self.py + PLAYER_H > y as f32
            && self.pz - PLAYER_HALF_W < (z + 1) as f32
            && self.pz + PLAYER_HALF_W > z as f32
    }

    fn respawn(&mut self) {
        let (sx, sy, sz) = self.world.spawn;
        self.px = sx;
        self.py = sy;
        self.pz = sz;
        self.vx = 0.0;
        self.vy = 0.0;
        self.vz = 0.0;
        self.hp = PLAYER_MAX_HP;
        self.game_over = false;
        self.say("You respawned. Your items are intact.");
    }

    // ------------------------------------------------------------- physics

    fn collides(&self, px: f32, py: f32, pz: f32) -> bool {
        let x0 = (px - PLAYER_HALF_W).floor() as i32;
        let x1 = (px + PLAYER_HALF_W - 0.001).floor() as i32;
        let y0 = py.floor() as i32;
        let y1 = (py + PLAYER_H - 0.001).floor() as i32;
        let z0 = (pz - PLAYER_HALF_W).floor() as i32;
        let z1 = (pz + PLAYER_HALF_W - 0.001).floor() as i32;
        for y in y0..=y1 {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    if self.world.get(x, y, z).is_solid() {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn in_water(&self) -> bool {
        let (x, z) = (self.px.floor() as i32, self.pz.floor() as i32);
        self.world.get(x, (self.py + 0.1).floor() as i32, z) == Block::Water
            || self.world.get(x, (self.py + 0.9).floor() as i32, z) == Block::Water
            || self.world.get(x, (self.py + EYE_HEIGHT).floor() as i32, z) == Block::Water
    }

    /// Space bar action: a real jump when standing on something (even in
    /// shallow water), a strong kick when swimming against a bank so you can
    /// climb out, and otherwise a gentle swim upward.
    fn jump_or_swim(&mut self) {
        if self.on_ground {
            self.vy = JUMP_VEL;
            self.on_ground = false;
        } else if self.in_water() {
            self.vy = if self.swim_blocked { JUMP_VEL } else { SWIM_UP };
        }
    }

    fn step_axis(&mut self, axis: usize, amount: f32) -> bool {
        // Returns true if blocked.
        let steps = (amount.abs() / 0.1).ceil().max(1.0) as i32;
        let d = amount / steps as f32;
        for _ in 0..steps {
            let (nx, ny, nz) = match axis {
                0 => (self.px + d, self.py, self.pz),
                1 => (self.px, self.py + d, self.pz),
                _ => (self.px, self.py, self.pz + d),
            };
            if self.collides(nx, ny, nz) {
                return true;
            }
            self.px = nx;
            self.py = ny;
            self.pz = nz;
        }
        false
    }

    // ------------------------------------------------------------- input

    pub fn on_key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            // Seeing any release proves the terminal reports them reliably.
            self.saw_release = true;
            match k.code {
                KeyCode::Char('w') | KeyCode::Char('W') => self.held_w.release(),
                KeyCode::Char('s') | KeyCode::Char('S') => self.held_s.release(),
                KeyCode::Char('a') | KeyCode::Char('A') => self.held_a.release(),
                KeyCode::Char('d') | KeyCode::Char('D') => self.held_d.release(),
                KeyCode::Char(' ') => self.held_jump.release(),
                KeyCode::Char('f') | KeyCode::Char('F') => self.held_down.release(),
                _ => {}
            }
            return;
        }
        // Chat swallows every key while it's open, so you can type "quit"
        // without quitting.
        if self.chat_open {
            match k.code {
                KeyCode::Esc => {
                    self.chat_open = false;
                    self.chat_input.clear();
                }
                KeyCode::Enter => self.send_chat_line(),
                KeyCode::Backspace => {
                    self.chat_input.pop();
                }
                KeyCode::Char(c) => {
                    if self.chat_input.chars().count() < 100 && !c.is_control() {
                        self.chat_input.push(c);
                    }
                }
                _ => {}
            }
            return;
        }
        if self.game_over {
            match k.code {
                KeyCode::Char('r') | KeyCode::Char('R') => self.respawn(),
                KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                _ => {}
            }
            return;
        }
        if self.crafting_open {
            match k.code {
                KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('q') => {
                    self.crafting_open = false
                }
                KeyCode::Up | KeyCode::Char('w') => {
                    self.craft_sel = self.craft_sel.checked_sub(1).unwrap_or(RECIPES.len() - 1)
                }
                KeyCode::Down | KeyCode::Char('s') => {
                    self.craft_sel = (self.craft_sel + 1) % RECIPES.len()
                }
                KeyCode::Enter | KeyCode::Char(' ') => self.craft(),
                _ => {}
            }
            return;
        }
        if self.help_open {
            if matches!(
                k.code,
                KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('?') | KeyCode::Char('q')
            ) {
                self.help_open = false;
            }
            return;
        }
        if k.code == KeyCode::Char('s') && k.modifiers.contains(KeyModifiers::CONTROL) {
            self.do_save();
            return;
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('w') | KeyCode::Char('W') => {
                self.held_w.press(self.time);
                self.move_fwd = 1.0;
                self.move_timer = 4;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.held_s.press(self.time);
                self.move_fwd = -1.0;
                self.move_timer = 4;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.held_a.press(self.time);
                self.move_strafe = -1.0;
                self.move_timer = 4;
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.held_d.press(self.time);
                self.move_strafe = 1.0;
                self.move_timer = 4;
            }
            KeyCode::Char(' ') => {
                self.held_jump.press(self.time);
                if self.creative {
                    self.fly_vertical = 1.0;
                    self.move_timer = 4;
                } else {
                    self.jump_or_swim();
                }
            }
            KeyCode::Char('f') | KeyCode::Char('F') => {
                self.held_down.press(self.time);
                if self.creative {
                    self.fly_vertical = -1.0;
                    self.move_timer = 4;
                }
            }
            KeyCode::Left => self.yaw -= LOOK_STEP,
            KeyCode::Right => self.yaw += LOOK_STEP,
            KeyCode::Up => self.pitch = (self.pitch + LOOK_STEP).min(1.45),
            KeyCode::Down => self.pitch = (self.pitch - LOOK_STEP).max(-1.45),
            KeyCode::Char('x') | KeyCode::Char('X') | KeyCode::Enter => self.mine(),
            KeyCode::Char('z') | KeyCode::Char('Z') | KeyCode::Char('p') => self.place(),
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.crafting_open = true;
                self.craft_sel = 0;
            }
            KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Char('?') | KeyCode::F(1) => {
                self.help_open = true;
            }
            KeyCode::Char(ch @ '1'..='9') => {
                self.selected = ch as usize - '1' as usize;
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                if self.is_multiplayer() {
                    self.chat_open = true;
                    self.chat_input.clear();
                } else {
                    self.say("Chat needs a multiplayer world (--seed <N>).");
                }
            }
            KeyCode::Tab => self.players_open = !self.players_open,
            KeyCode::F(5) => self.do_save(),
            _ => {}
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if self.game_over || self.crafting_open || self.help_open || self.chat_open {
            return;
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.last_mouse = Some((m.column, m.row));
                self.mine();
            }
            MouseEventKind::Down(MouseButton::Right) => self.place(),
            MouseEventKind::Drag(_) => {
                if let Some((lx, ly)) = self.last_mouse {
                    let dx = m.column as f32 - lx as f32;
                    let dy = m.row as f32 - ly as f32;
                    self.yaw += dx * 0.02;
                    self.pitch = (self.pitch - dy * 0.04).clamp(-1.45, 1.45);
                }
                self.last_mouse = Some((m.column, m.row));
            }
            MouseEventKind::Up(_) => self.last_mouse = None,
            _ => {}
        }
    }

    // ------------------------------------------------------------- tick

    pub fn tick(&mut self) {
        if self.game_over {
            // Stay in the session while dead so chat and peers keep updating.
            self.net_sync();
            return;
        }
        self.time += 1;
        self.net_sync();

        // Movement intent. In hold mode the held key flags drive movement
        // continuously; otherwise fall back to a short timer refreshed by
        // key auto-repeat.
        let (mf, ms, mu) = if self.hold_mode {
            let trust = self.saw_release;
            (
                (self.held_w.active(self.time, trust) as i32
                    - self.held_s.active(self.time, trust) as i32) as f32,
                (self.held_d.active(self.time, trust) as i32
                    - self.held_a.active(self.time, trust) as i32) as f32,
                if self.creative {
                    (self.held_jump.active(self.time, trust) as i32
                        - self.held_down.active(self.time, trust) as i32) as f32
                } else {
                    0.0
                },
            )
        } else if self.move_timer > 0 {
            self.move_timer -= 1;
            (
                self.move_fwd,
                self.move_strafe,
                if self.creative {
                    self.fly_vertical
                } else {
                    0.0
                },
            )
        } else {
            self.move_fwd = 0.0;
            self.move_strafe = 0.0;
            self.fly_vertical = 0.0;
            (0.0, 0.0, 0.0)
        };

        if self.creative {
            // Fly where you look: WASD includes pitch; space/f are world up/down.
            let (fwd_x, fwd_y, fwd_z) = self.forward();
            let (sy, cy) = self.yaw.sin_cos();
            let (rx, rz) = (-sy, cy);
            let mut dx = fwd_x * mf + rx * ms;
            let mut dy = fwd_y * mf + mu;
            let mut dz = fwd_z * mf + rz * ms;
            let len = (dx * dx + dy * dy + dz * dz).sqrt();
            if len > 0.001 {
                dx = dx / len * MOVE_SPEED;
                dy = dy / len * MOVE_SPEED;
                dz = dz / len * MOVE_SPEED;
            } else {
                dx = 0.0;
                dy = 0.0;
                dz = 0.0;
            }
            if self.step_axis(0, dx) {
                dx = 0.0;
            }
            if self.step_axis(1, dy) {
                dy = 0.0;
            }
            if self.step_axis(2, dz) {
                dz = 0.0;
            }
            let _ = (dx, dy, dz);
            // No gravity, no residual velocity, no fall damage.
            self.vx = 0.0;
            self.vy = 0.0;
            self.vz = 0.0;
            self.on_ground = false;
            self.swim_blocked = false;
        } else {
            if mf != 0.0 || ms != 0.0 {
                let (sy, cy) = self.yaw.sin_cos();
                let fx = cy * mf - sy * ms;
                let fz = sy * mf + cy * ms;
                let len = (fx * fx + fz * fz).sqrt().max(0.001);
                let speed = if self.in_water() {
                    MOVE_SPEED * 0.6
                } else {
                    MOVE_SPEED
                };
                self.vx = fx / len * speed;
                self.vz = fz / len * speed;
            }
            // Horizontal movement first, so we know whether we're pushing
            // against a bank while swimming.
            let blocked_x = self.step_axis(0, self.vx);
            if blocked_x {
                self.vx = 0.0;
            }
            let blocked_z = self.step_axis(2, self.vz);
            if blocked_z {
                self.vz = 0.0;
            }
            self.swim_blocked = (blocked_x || blocked_z) && self.in_water();

            // Holding space keeps jumping / swimming up.
            if self.hold_mode && self.held_jump.active(self.time, self.saw_release) {
                self.jump_or_swim();
            }

            // Gravity / buoyancy.
            if self.in_water() {
                self.vy = (self.vy - GRAVITY * 0.25).clamp(-0.18, SWIM_MAX_RISE);
            } else {
                self.vy = (self.vy - GRAVITY).max(MAX_FALL);
            }
            let falling = self.vy;
            self.on_ground = false;
            if self.step_axis(1, self.vy) {
                if self.vy < 0.0 {
                    self.on_ground = true;
                    if falling < SAFE_FALL && !self.in_water() {
                        let dmg = ((SAFE_FALL - falling) * 22.0) as i32;
                        if dmg > 0 {
                            self.hp -= dmg;
                            self.last_damage_tick = self.time;
                            self.say("Ouch! Fall damage.");
                        }
                    }
                }
                self.vy = 0.0;
            }
            // Friction.
            self.vx *= 0.5;
            self.vz *= 0.5;
        }

        // Regen.
        if self.hp < PLAYER_MAX_HP
            && self.time.saturating_sub(self.last_damage_tick) > 100
            && self.time.is_multiple_of(40)
        {
            self.hp += 1;
        }

        if self.hp <= 0 {
            self.game_over = true;
        }

        if let Some((_, expiry)) = &self.msg {
            if self.time > *expiry {
                self.msg = None;
            }
        }
    }

    // ------------------------------------------------------------- save/load

    pub fn do_save(&mut self) {
        match self.save() {
            Ok(_) => self.say("World saved."),
            Err(e) => {
                let m = format!("Save failed: {e}");
                self.say(&m);
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        if !self.owns_save {
            return Err(std::io::Error::other(
                "the host owns this world - nothing saved locally",
            ));
        }
        let data = Save3 {
            seed: self.seed,
            tiles: self.world.to_bytes(),
            spawn: self.world.spawn,
            pos: (self.px, self.py, self.pz),
            yaw: self.yaw,
            pitch: self.pitch,
            hp: self.hp,
            time: self.time,
            inv: self.inv.iter().map(|(b, n)| (b.to_u8(), *n)).collect(),
            hotbar: self
                .hotbar
                .iter()
                .map(|s| s.map(|b| b.to_u8() as i16).unwrap_or(-1))
                .collect(),
            selected: self.selected,
        };
        if let Some(dir) = self.save_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string(&data)?;
        std::fs::write(&self.save_path, json)
    }

    pub fn load() -> Option<Game3> {
        Game3::load_from(&save3_path())
    }

    /// Loads a world from a specific save file, e.g. a per-seed shared world.
    pub fn load_from(path: &std::path::Path) -> Option<Game3> {
        let json = std::fs::read_to_string(path).ok()?;
        let data: Save3 = serde_json::from_str(&json).ok()?;
        let world = World3::from_bytes(&data.tiles, data.spawn)?;
        let mut g = Game3::from_world(data.seed, world);
        for (i, v) in data.hotbar.iter().take(9).enumerate() {
            if *v >= 0 {
                g.hotbar[i] = Some(Block::from_u8(*v as u8));
            }
        }
        g.px = data.pos.0;
        g.py = data.pos.1;
        g.pz = data.pos.2;
        g.yaw = data.yaw;
        g.pitch = data.pitch;
        g.hp = data.hp;
        g.inv = data
            .inv
            .iter()
            .map(|&(b, n)| (Block::from_u8(b), n))
            .collect();
        g.selected = data.selected.min(8);
        g.time = data.time;
        g.last_damage_tick = data.time;
        g.save_path = path.to_path_buf();
        g.say("World loaded. Welcome back!");
        Some(g)
    }
}

#[derive(Serialize, Deserialize)]
struct Save3 {
    seed: u64,
    tiles: Vec<u8>,
    spawn: (f32, f32, f32),
    pos: (f32, f32, f32),
    yaw: f32,
    pitch: f32,
    hp: i32,
    time: u64,
    inv: Vec<(u8, u32)>,
    hotbar: Vec<i16>,
    selected: usize,
}

pub fn save3_path() -> PathBuf {
    crate::game::home_dir()
        .join(".termcraft")
        .join("save3d.json")
}

/// Worlds played with an explicit seed get their own file, so a shared seed
/// keeps everyone's buildings between sessions.
pub fn seed_save_path(seed: u64) -> PathBuf {
    crate::game::home_dir()
        .join(".termcraft")
        .join(format!("world-{seed}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_settles_on_ground() {
        let mut g = Game3::new(9);
        for _ in 0..200 {
            g.tick();
        }
        assert!(g.on_ground || g.in_water());
        assert!(g.py > 0.0);
    }

    #[test]
    fn creative_idle_in_air_does_not_fall_and_space_raises() {
        let mut g = Game3::new(9);
        g.set_creative(true);
        g.set_hold_mode(true);
        enable_trusted_releases(&mut g);
        // Hover well above the world with no vertical velocity.
        g.py = 40.0;
        g.vy = 0.0;
        let idle_y = g.py;
        for _ in 0..40 {
            g.tick();
        }
        assert!(
            (g.py - idle_y).abs() < 0.01,
            "creative idle must not fall: was {idle_y}, now {}",
            g.py
        );

        g.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        for _ in 0..20 {
            g.tick();
        }
        assert!(
            g.py > idle_y + 1.0,
            "space should raise py in creative: was {idle_y}, now {}",
            g.py
        );
    }

    #[test]
    fn raycast_down_hits_ground() {
        let g = Game3::new(9);
        let hit = raycast(&g.world, g.eye(), (0.0, -1.0, 0.0), 10.0).expect("ground below");
        assert!(hit.block.is_solid());
        assert_eq!(hit.ny, 1); // entered through the top face
    }

    /// Sends a release of an unused key, marking the terminal as one that
    /// reliably reports release events.
    fn enable_trusted_releases(g: &mut Game3) {
        use crossterm::event::KeyEventState;
        g.on_key(KeyEvent {
            code: KeyCode::Char('n'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
    }

    #[test]
    fn help_menu_toggles_without_quitting() {
        let mut g = Game3::new(9);
        g.on_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        assert!(g.help_open);
        // Keys other than the close keys do nothing while help is open.
        g.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(g.help_open);
        g.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!g.help_open);
        assert!(!g.should_quit);
    }

    #[test]
    fn can_climb_out_of_water_at_a_bank() {
        let mut g = Game3::new(9);
        g.set_hold_mode(true);
        // Build a controlled pool: open box with a stone floor at y=9,
        // water at y=10..=11, and a bank one block above the water (top y=13).
        for x in 55..65 {
            for z in 55..65 {
                for y in 10..30 {
                    g.world.set(x, y, z, Block::Air);
                }
                g.world.set(x, 9, z, Block::Stone);
            }
        }
        for x in 56..60 {
            for z in 56..64 {
                g.world.set(x, 10, z, Block::Water);
                g.world.set(x, 11, z, Block::Water);
            }
        }
        for x in 60..64 {
            for z in 56..64 {
                for y in 10..=12 {
                    g.world.set(x, y, z, Block::Stone);
                }
            }
        }
        // Float the player in the pool, facing the bank (+x).
        g.px = 58.5;
        g.py = 10.2;
        g.pz = 60.5;
        g.vx = 0.0;
        g.vy = 0.0;
        g.vz = 0.0;
        g.yaw = 0.0;
        assert!(g.in_water());
        // Hold forward + space and swim at the bank.
        enable_trusted_releases(&mut g);
        g.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        g.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        // Succeed as soon as the player stands clear of the water on the
        // bank (the test keeps holding 'w', so it would otherwise walk
        // right across and off the far side).
        let mut climbed_out = false;
        for _ in 0..100 {
            g.tick();
            if g.py >= 12.5 && !g.in_water() && g.px >= 60.0 {
                climbed_out = true;
                break;
            }
        }
        assert!(
            climbed_out,
            "expected to climb out onto the bank, feet at y={} x={}",
            g.py, g.px
        );
    }

    #[test]
    fn holding_w_moves_until_released() {
        use crossterm::event::KeyEventState;
        let mut g = Game3::new(9);
        g.set_hold_mode(true);
        for _ in 0..100 {
            g.tick(); // settle
        }
        enable_trusted_releases(&mut g);
        let start = (g.px, g.pz);
        // Press 'w' once (no repeats) and keep ticking: should keep moving.
        g.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        for _ in 0..20 {
            g.tick();
        }
        let moved = (g.px - start.0).hypot(g.pz - start.1);
        assert!(moved > 1.5, "expected continuous movement, moved {moved}");
        // Release 'w': movement should stop.
        g.on_key(KeyEvent {
            code: KeyCode::Char('w'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
        for _ in 0..5 {
            g.tick(); // let velocity decay
        }
        let stopped_at = (g.px, g.pz);
        for _ in 0..10 {
            g.tick();
        }
        let drift = (g.px - stopped_at.0).hypot(g.pz - stopped_at.1);
        assert!(
            drift < 0.2,
            "expected to stop after release, drifted {drift}"
        );
    }

    #[test]
    fn movement_expires_when_releases_never_arrive() {
        // Regression: a terminal that claims kitty support but never sends
        // release events must not leave the player moving/jumping forever.
        let mut g = Game3::new(9);
        g.set_hold_mode(true);
        for _ in 0..100 {
            g.tick(); // settle
        }
        g.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        g.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        for _ in 0..60 {
            g.tick(); // never send a release
        }
        let pos = (g.px, g.pz);
        let mut jumped = false;
        for _ in 0..30 {
            g.tick();
            if !g.on_ground && !g.in_water() && g.vy > 0.1 {
                jumped = true;
            }
        }
        let drift = (g.px - pos.0).hypot(g.pz - pos.1);
        assert!(
            drift < 0.2,
            "still moving without key repeats, drifted {drift}"
        );
        assert!(!jumped, "still jumping without key repeats");
    }

    /// Drops a peer right in front of the player, facing +x.
    fn place_peer_in_front(g: &mut Game3, id: PlayerId) {
        g.yaw = 0.0;
        g.pitch = 0.0;
        let st = PlayerState {
            x: g.px + 2.0,
            y: g.py,
            z: g.pz,
            yaw: 0.0,
            pitch: 0.0,
            hp: PLAYER_MAX_HP,
        };
        g.peers.insert(
            id,
            Peer {
                id,
                name: "buddy".into(),
                st,
                last_seen: g.time,
            },
        );
    }

    #[test]
    fn crosshair_finds_a_peer_in_front() {
        let mut g = Game3::new(9);
        for _ in 0..100 {
            g.tick(); // settle
        }
        // Clear the air around the player so nothing occludes the peer.
        let (bx, by, bz) = (g.px as i32, g.py as i32, g.pz as i32);
        for x in bx..=bx + 4 {
            for y in by..by + 3 {
                for z in bz - 1..=bz + 1 {
                    g.world.set(x, y, z, Block::Air);
                }
            }
        }
        place_peer_in_front(&mut g, 3);
        let (id, dist) = g.peer_in_crosshair().expect("peer under the crosshair");
        assert_eq!(id, 3);
        assert!((1.0..2.5).contains(&dist), "unexpected distance {dist}");

        // A block between us hides them again.
        g.world.set(bx + 1, by + 1, bz, Block::Stone);
        assert!(g.peer_in_crosshair().is_none());
    }

    #[test]
    fn mining_through_a_peer_does_not_break_blocks() {
        let mut g = Game3::new(9);
        for _ in 0..100 {
            g.tick();
        }
        g.pitch = -1.45; // look nearly straight down
        let below = g.target().expect("ground in reach");
        // Put the peer between the player and that block.
        g.peers.insert(
            7,
            Peer {
                id: 7,
                name: "shield".into(),
                st: PlayerState {
                    x: g.px,
                    y: g.py - 1.4,
                    z: g.pz,
                    yaw: 0.0,
                    pitch: 0.0,
                    hp: PLAYER_MAX_HP,
                },
                last_seen: g.time,
            },
        );
        assert!(g.peer_in_crosshair().is_some());
        g.mine();
        assert_ne!(
            g.world.get(below.x, below.y, below.z),
            Block::Air,
            "the punch should not also mine the block behind them"
        );
    }

    #[test]
    fn peers_expire_when_they_go_quiet() {
        let mut g = Game3::new(9);
        place_peer_in_front(&mut g, 2);
        assert_eq!(g.peers.len(), 1);
        // No net link, so net_sync is a no-op; age the peer by hand.
        g.time += PEER_TIMEOUT + 1;
        let now = g.time;
        g.peers
            .retain(|_, p| now.saturating_sub(p.last_seen) < PEER_TIMEOUT);
        assert!(g.peers.is_empty());
    }

    #[test]
    fn chat_typing_does_not_leak_into_the_game() {
        let mut g = Game3::new(9);
        g.chat_open = true;
        for c in "quit x".chars() {
            g.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(g.chat_input, "quit x");
        assert!(!g.should_quit, "typing 'q' must not quit");
        g.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(g.chat_input, "quit ");
        // Enter posts the line locally even in single player.
        g.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!g.chat_open);
        assert_eq!(g.recent_chat().len(), 1);
        assert_eq!(g.recent_chat()[0].1, "quit");
    }

    #[test]
    fn guests_never_write_the_hosts_save() {
        let mut g = Game3::new(9);
        g.set_save(PathBuf::from("/definitely/not/writable/world.json"), false);
        assert!(!g.owns_save());
        assert!(g.save().is_err());
    }

    #[test]
    fn ray_aabb_hits_and_misses() {
        let min = (1.0, 0.0, -0.5);
        let max = (2.0, 1.8, 0.5);
        let t = ray_aabb((0.0, 1.0, 0.0), (1.0, 0.0, 0.0), min, max).expect("straight-on hit");
        assert!((t - 1.0).abs() < 1e-3);
        // Pointing away misses.
        assert!(ray_aabb((0.0, 1.0, 0.0), (-1.0, 0.0, 0.0), min, max).is_none());
        // Passing above misses.
        assert!(ray_aabb((0.0, 3.0, 0.0), (1.0, 0.0, 0.0), min, max).is_none());
    }

    #[test]
    fn mine_and_place_roundtrip() {
        let mut g = Game3::new(9);
        for _ in 0..100 {
            g.tick(); // settle
        }
        g.pitch = -1.2; // look down
        let before = g.target().expect("looking at ground");
        g.mine();
        assert!(g.world.get(before.x, before.y, before.z) == Block::Air);
        assert!(!g.inv.is_empty());
        // Place it back.
        let item = g.hotbar[0].expect("picked up a block");
        let n = g.count(item);
        g.place();
        assert_eq!(g.count(item), n - 1);
    }
}
