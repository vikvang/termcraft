use std::collections::BTreeMap;
use std::path::PathBuf;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use serde::{Deserialize, Serialize};

use crate::block::Block;
use crate::game::{DAY_LEN, PLAYER_MAX_HP, RECIPES};
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
    let (mut ix, mut iy, mut iz) = (
        o.0.floor() as i32,
        o.1.floor() as i32,
        o.2.floor() as i32,
    );
    let step = (d.0.signum() as i32, d.1.signum() as i32, d.2.signum() as i32);
    let inv = |v: f32| if v != 0.0 { (1.0 / v).abs() } else { f32::INFINITY };
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
    pub msg: Option<(String, u64)>,
    pub game_over: bool,
    move_fwd: f32,
    move_strafe: f32,
    move_timer: u32,
    /// True when the terminal reports key release events (kitty protocol),
    /// enabling continuous hold-to-move instead of per-keypress nudges.
    hold_mode: bool,
    held_w: bool,
    held_s: bool,
    held_a: bool,
    held_d: bool,
    held_jump: bool,
    /// True while swimming and pushing horizontally against a solid block
    /// (used to kick the player up so they can climb out of water).
    swim_blocked: bool,
    last_damage_tick: u64,
    last_mouse: Option<(u16, u16)>,
}

impl Game3 {
    pub fn new(seed: u64) -> Game3 {
        let world = World3::generate(seed);
        let (sx, sy, sz) = world.spawn;
        let mut g = Game3 {
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
            hp: PLAYER_MAX_HP,
            inv: BTreeMap::new(),
            hotbar: [None; 9],
            selected: 0,
            time: 0,
            should_quit: false,
            crafting_open: false,
            craft_sel: 0,
            msg: None,
            game_over: false,
            move_fwd: 0.0,
            move_strafe: 0.0,
            move_timer: 0,
            hold_mode: false,
            held_w: false,
            held_s: false,
            held_a: false,
            held_d: false,
            held_jump: false,
            swim_blocked: false,
            last_damage_tick: 0,
            last_mouse: None,
        };
        g.say("Welcome to TermCraft 3D! Aim with arrows, mine with x.");
        g
    }

    pub fn say(&mut self, s: &str) {
        self.msg = Some((s.to_string(), self.time + 80));
    }

    pub fn set_hold_mode(&mut self, on: bool) {
        self.hold_mode = on;
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
        self.world.set(hit.x, hit.y, hit.z, Block::Air);
        if let Some(drop) = hit.block.drops() {
            self.add_item(drop, 1);
            let m = format!("+1 {}", drop.name());
            self.say(&m);
        }
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
        self.remove_item(b, 1);
        self.world.set(tx, ty, tz, b);
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
            match k.code {
                KeyCode::Char('w') | KeyCode::Char('W') => self.held_w = false,
                KeyCode::Char('s') | KeyCode::Char('S') => self.held_s = false,
                KeyCode::Char('a') | KeyCode::Char('A') => self.held_a = false,
                KeyCode::Char('d') | KeyCode::Char('D') => self.held_d = false,
                KeyCode::Char(' ') => self.held_jump = false,
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
        if k.code == KeyCode::Char('s') && k.modifiers.contains(KeyModifiers::CONTROL) {
            self.do_save();
            return;
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('w') | KeyCode::Char('W') => {
                self.held_w = true;
                self.move_fwd = 1.0;
                self.move_timer = 4;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.held_s = true;
                self.move_fwd = -1.0;
                self.move_timer = 4;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.held_a = true;
                self.move_strafe = -1.0;
                self.move_timer = 4;
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.held_d = true;
                self.move_strafe = 1.0;
                self.move_timer = 4;
            }
            KeyCode::Char(' ') => {
                self.held_jump = true;
                self.jump_or_swim();
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
            KeyCode::Char(ch @ '1'..='9') => {
                self.selected = ch as usize - '1' as usize;
            }
            KeyCode::F(5) => self.do_save(),
            _ => {}
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if self.game_over || self.crafting_open {
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
            return;
        }
        self.time += 1;

        // Movement intent relative to yaw (horizontal only). In hold mode the
        // held key flags drive movement continuously; otherwise fall back to a
        // short timer refreshed by key auto-repeat.
        let (mf, ms) = if self.hold_mode {
            (
                (self.held_w as i32 - self.held_s as i32) as f32,
                (self.held_d as i32 - self.held_a as i32) as f32,
            )
        } else if self.move_timer > 0 {
            self.move_timer -= 1;
            (self.move_fwd, self.move_strafe)
        } else {
            self.move_fwd = 0.0;
            self.move_strafe = 0.0;
            (0.0, 0.0)
        };
        if mf != 0.0 || ms != 0.0 {
            let (sy, cy) = self.yaw.sin_cos();
            let fx = cy * mf - sy * ms;
            let fz = sy * mf + cy * ms;
            let len = (fx * fx + fz * fz).sqrt().max(0.001);
            let speed = if self.in_water() { MOVE_SPEED * 0.6 } else { MOVE_SPEED };
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
        if self.hold_mode && self.held_jump {
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
        let path = save3_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string(&data)?;
        std::fs::write(path, json)
    }

    pub fn load() -> Option<Game3> {
        let json = std::fs::read_to_string(save3_path()).ok()?;
        let data: Save3 = serde_json::from_str(&json).ok()?;
        let world = World3::from_bytes(&data.tiles, data.spawn)?;
        let mut hotbar = [None; 9];
        for (i, v) in data.hotbar.iter().take(9).enumerate() {
            if *v >= 0 {
                hotbar[i] = Some(Block::from_u8(*v as u8));
            }
        }
        let mut g = Game3 {
            world,
            seed: data.seed,
            px: data.pos.0,
            py: data.pos.1,
            pz: data.pos.2,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            yaw: data.yaw,
            pitch: data.pitch,
            on_ground: false,
            hp: data.hp,
            inv: data
                .inv
                .iter()
                .map(|&(b, n)| (Block::from_u8(b), n))
                .collect(),
            hotbar,
            selected: data.selected.min(8),
            time: data.time,
            should_quit: false,
            crafting_open: false,
            craft_sel: 0,
            msg: None,
            game_over: false,
            move_fwd: 0.0,
            move_strafe: 0.0,
            move_timer: 0,
            hold_mode: false,
            held_w: false,
            held_s: false,
            held_a: false,
            held_d: false,
            held_jump: false,
            swim_blocked: false,
            last_damage_tick: data.time,
            last_mouse: None,
        };
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
    crate::game::home_dir().join(".termcraft").join("save3d.json")
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
    fn raycast_down_hits_ground() {
        let g = Game3::new(9);
        let hit = raycast(&g.world, g.eye(), (0.0, -1.0, 0.0), 10.0).expect("ground below");
        assert!(hit.block.is_solid());
        assert_eq!(hit.ny, 1); // entered through the top face
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
        assert!(drift < 0.2, "expected to stop after release, drifted {drift}");
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
