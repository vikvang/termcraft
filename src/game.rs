use std::collections::BTreeMap;
use std::path::PathBuf;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

use crate::block::Block;
use crate::entity::{Entity, SAFE_FALL_VEL};
use crate::world::{World, SEA_LEVEL, WORLD_H, WORLD_W};

pub const DAY_LEN: u64 = 2400; // ticks per day (2 minutes at 20 TPS)
pub const REACH: f32 = 5.5;
pub const PLAYER_MAX_HP: i32 = 20;
pub const ZOMBIE_MAX_HP: i32 = 10;
pub const MAX_ZOMBIES: usize = 6;

pub struct Recipe {
    pub label: &'static str,
    pub out: Block,
    pub out_n: u32,
    pub cost: &'static [(Block, u32)],
}

pub const RECIPES: &[Recipe] = &[
    Recipe {
        label: "4x Planks  <-  1x Wood",
        out: Block::Planks,
        out_n: 4,
        cost: &[(Block::Wood, 1)],
    },
    Recipe {
        label: "4x Torch  <-  1x Coal Ore + 1x Planks",
        out: Block::Torch,
        out_n: 4,
        cost: &[(Block::CoalOre, 1), (Block::Planks, 1)],
    },
    Recipe {
        label: "4x Stone Brick  <-  4x Cobblestone",
        out: Block::StoneBrick,
        out_n: 4,
        cost: &[(Block::Cobblestone, 4)],
    },
    Recipe {
        label: "1x Wood  <-  4x Leaves",
        out: Block::Wood,
        out_n: 1,
        cost: &[(Block::Leaves, 4)],
    },
];

/// How long (in ticks) a key press counts as "held" when we can't trust
/// release events (key auto-repeat refreshes it while genuinely held).
pub const HOLD_GRACE_TICKS: u64 = 10;

/// Tracks a possibly-held key. Some terminals claim kitty keyboard support
/// but never deliver release events, which would leave a naive flag stuck
/// forever. Until a real release event is observed, a hold only stays
/// active for a short grace period after the last press/repeat.
#[derive(Clone, Copy, Default)]
pub struct KeyHold {
    down: bool,
    last_seen: u64,
}

impl KeyHold {
    pub fn press(&mut self, now: u64) {
        self.down = true;
        self.last_seen = now;
    }

    pub fn release(&mut self) {
        self.down = false;
    }

    pub fn active(&self, now: u64, trust_releases: bool) -> bool {
        self.down
            && (trust_releases || now.saturating_sub(self.last_seen) <= HOLD_GRACE_TICKS)
    }
}

pub struct Game {
    pub world: World,
    pub seed: u64,
    pub player: Entity,
    pub zombies: Vec<Entity>,
    pub inv: BTreeMap<Block, u32>,
    pub hotbar: [Option<Block>; 9],
    pub selected: usize,
    pub cursor: (i32, i32),
    pub time: u64,
    pub should_quit: bool,
    pub crafting_open: bool,
    pub craft_sel: usize,
    pub help_open: bool,
    pub msg: Option<(String, u64)>,
    pub game_over: bool,
    pub map_area: Rect,
    pub camera: (i32, i32),
    move_dir: i32,
    move_timer: u32,
    /// True when the terminal reports key release events (kitty protocol),
    /// enabling continuous hold-to-move instead of per-keypress nudges.
    hold_mode: bool,
    held_left: KeyHold,
    held_right: KeyHold,
    held_jump: KeyHold,
    /// Set once any key release event arrives, proving the terminal
    /// actually reports them.
    saw_release: bool,
    last_damage_tick: u64,
    zombie_hit_cooldown: u64,
    rng: StdRng,
}

impl Game {
    pub fn new(seed: u64) -> Game {
        let world = World::generate(seed);
        let (sx, sy) = world.spawn;
        let player = Entity::new(sx as f32 + 0.1, sy as f32, PLAYER_MAX_HP);
        let mut g = Game {
            world,
            seed,
            player,
            zombies: Vec::new(),
            inv: BTreeMap::new(),
            hotbar: [None; 9],
            selected: 0,
            cursor: (sx, sy + 2),
            time: 0,
            should_quit: false,
            crafting_open: false,
            craft_sel: 0,
            help_open: false,
            msg: None,
            game_over: false,
            map_area: Rect::new(0, 0, 1, 1),
            camera: (0, 0),
            move_dir: 0,
            move_timer: 0,
            hold_mode: false,
            held_left: KeyHold::default(),
            held_right: KeyHold::default(),
            held_jump: KeyHold::default(),
            saw_release: false,
            last_damage_tick: 0,
            zombie_hit_cooldown: 0,
            rng: StdRng::seed_from_u64(seed ^ 0xC0FFEE),
        };
        g.say("Welcome to TermCraft! Mine with x, place with z, q quits.");
        g
    }

    pub fn say(&mut self, s: &str) {
        self.msg = Some((s.to_string(), self.time + 80));
    }

    pub fn set_hold_mode(&mut self, on: bool) {
        self.hold_mode = on;
    }

    /// 1.0 = full daylight, 0.15 = night.
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

    // ------------------------------------------------------------- actions

    fn in_reach(&self, tx: i32, ty: i32) -> bool {
        let (px, py) = self.player.center();
        let dx = (tx as f32 + 0.5) - px;
        let dy = (ty as f32 + 0.5) - py;
        (dx * dx + dy * dy).sqrt() <= REACH
    }

    pub fn cursor_in_reach(&self) -> bool {
        self.in_reach(self.cursor.0, self.cursor.1)
    }

    fn clamp_cursor(&mut self) {
        let (px, py) = self.player.center();
        let r = REACH as i32;
        self.cursor.0 = self.cursor.0.clamp(px as i32 - r, px as i32 + r);
        self.cursor.1 = self.cursor.1.clamp(py as i32 - r, py as i32 + r);
        self.cursor.0 = self.cursor.0.clamp(0, WORLD_W - 1);
        self.cursor.1 = self.cursor.1.clamp(0, WORLD_H - 1);
    }

    fn mine(&mut self) {
        let (tx, ty) = self.cursor;
        if !self.in_reach(tx, ty) {
            self.say("Too far away!");
            return;
        }
        // Attack a zombie first if one is at the cursor.
        if let Some(z) = self
            .zombies
            .iter_mut()
            .find(|z| z.overlaps_tile(tx, ty))
        {
            z.hp -= 5;
            let kb = if z.center().0 > self.player.center().0 { 0.8 } else { -0.8 };
            z.vx += kb;
            z.vy -= 0.3;
            if z.hp <= 0 {
                self.say("Zombie slain!");
            }
            return;
        }
        let b = self.world.get(tx, ty);
        if !b.is_minable() {
            if b == Block::Bedrock {
                self.say("Bedrock is unbreakable.");
            }
            return;
        }
        self.world.set(tx, ty, Block::Air);
        if let Some(drop) = b.drops() {
            self.add_item(drop, 1);
            let m = format!("+1 {}", drop.name());
            self.say(&m);
        }
    }

    fn place(&mut self) {
        let (tx, ty) = self.cursor;
        if !self.in_reach(tx, ty) {
            self.say("Too far away!");
            return;
        }
        let Some(b) = self.hotbar[self.selected] else {
            self.say("Empty hotbar slot - select a block with 1-9.");
            return;
        };
        if self.count(b) == 0 {
            let m = format!("Out of {}.", b.name());
            self.say(&m);
            return;
        }
        let target = self.world.get(tx, ty);
        let replaceable = target == Block::Air || (target == Block::Water && b != Block::Torch);
        if !replaceable {
            return;
        }
        if b == Block::Torch && target != Block::Air {
            self.say("Torches need air.");
            return;
        }
        if b.is_solid()
            && (self.player.overlaps_tile(tx, ty)
                || self.zombies.iter().any(|z| z.overlaps_tile(tx, ty)))
        {
            self.say("Something is in the way.");
            return;
        }
        self.remove_item(b, 1);
        self.world.set(tx, ty, b);
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

    pub fn can_craft(&self, i: usize) -> bool {
        RECIPES[i].cost.iter().all(|&(b, n)| self.count(b) >= n)
    }

    fn respawn(&mut self) {
        let (sx, sy) = self.world.spawn;
        self.player.x = sx as f32 + 0.1;
        self.player.y = sy as f32;
        self.player.vx = 0.0;
        self.player.vy = 0.0;
        self.player.hp = PLAYER_MAX_HP;
        self.zombies.clear();
        self.game_over = false;
        self.say("You respawned. Your items are intact.");
    }

    // ------------------------------------------------------------- input

    pub fn on_key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            // Seeing any release proves the terminal reports them reliably.
            self.saw_release = true;
            match k.code {
                KeyCode::Char('a') | KeyCode::Char('A') => self.held_left.release(),
                KeyCode::Char('d') | KeyCode::Char('D') => self.held_right.release(),
                KeyCode::Char('w') | KeyCode::Char('W') | KeyCode::Char(' ') => {
                    self.held_jump.release()
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
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.held_left.press(self.time);
                self.move_dir = -1;
                self.move_timer = 4;
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.held_right.press(self.time);
                self.move_dir = 1;
                self.move_timer = 4;
            }
            KeyCode::Char('w') | KeyCode::Char('W') | KeyCode::Char(' ') => {
                self.held_jump.press(self.time);
                self.player.try_jump(&self.world)
            }
            KeyCode::Left => {
                self.cursor.0 -= 1;
                self.clamp_cursor();
            }
            KeyCode::Right => {
                self.cursor.0 += 1;
                self.clamp_cursor();
            }
            KeyCode::Up => {
                self.cursor.1 -= 1;
                self.clamp_cursor();
            }
            KeyCode::Down => {
                self.cursor.1 += 1;
                self.clamp_cursor();
            }
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
            KeyCode::F(5) => self.do_save(),
            _ => {}
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if self.game_over || self.crafting_open || self.help_open {
            return;
        }
        let a = self.map_area;
        if m.column < a.x || m.column >= a.x + a.width || m.row < a.y || m.row >= a.y + a.height {
            return;
        }
        let wx = self.camera.0 + (m.column - a.x) as i32;
        let wy = self.camera.1 + (m.row - a.y) as i32;
        self.cursor = (
            wx.clamp(0, WORLD_W - 1),
            wy.clamp(0, WORLD_H - 1),
        );
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => self.mine(),
            MouseEventKind::Down(MouseButton::Right) => self.place(),
            _ => {}
        }
    }

    // ------------------------------------------------------------- tick

    pub fn tick(&mut self) {
        if self.game_over {
            return;
        }
        self.time += 1;

        // Player movement. In hold mode the held key flags drive movement
        // continuously; otherwise fall back to a short timer refreshed by
        // key auto-repeat.
        if self.hold_mode {
            let trust = self.saw_release;
            let dir = self.held_right.active(self.time, trust) as i32
                - self.held_left.active(self.time, trust) as i32;
            if dir != 0 {
                self.player.vx = dir as f32 * 0.55;
            }
            if self.held_jump.active(self.time, trust) {
                self.player.try_jump(&self.world);
            }
        } else if self.move_timer > 0 {
            self.move_timer -= 1;
            self.player.vx = self.move_dir as f32 * 0.55;
        }
        if let Some(impact) = self.player.step_physics(&self.world) {
            if impact > SAFE_FALL_VEL {
                let dmg = ((impact - SAFE_FALL_VEL) * 16.0) as i32;
                if dmg > 0 {
                    self.hurt_player(dmg, "Ouch! Fall damage.");
                }
            }
        }

        // Water flow
        if self.time.is_multiple_of(3) {
            self.world.flow_water();
        }

        // Drowning is mean in a chill game; skipped. But regen:
        if self.player.hp < self.player.max_hp
            && self.time.saturating_sub(self.last_damage_tick) > 100
            && self.time.is_multiple_of(40)
        {
            self.player.hp += 1;
        }

        self.tick_zombies();

        if self.player.hp <= 0 {
            self.game_over = true;
        }

        if let Some((_, expiry)) = &self.msg {
            if self.time > *expiry {
                self.msg = None;
            }
        }
    }

    fn hurt_player(&mut self, dmg: i32, msg: &str) {
        self.player.hp -= dmg;
        self.last_damage_tick = self.time;
        self.say(msg);
    }

    fn tick_zombies(&mut self) {
        // Spawn at night near (but not on top of) the player.
        if self.is_night() && self.zombies.len() < MAX_ZOMBIES && self.time.is_multiple_of(60) {
            let px = self.player.center().0 as i32;
            let dist = self.rng.gen_range(18..40);
            let sx = if self.rng.gen_bool(0.5) { px + dist } else { px - dist };
            if (1..WORLD_W - 1).contains(&sx) {
                let sy = self.world.surface_at(sx);
                if sy < WORLD_H - 4 && self.world.get(sx, sy) != Block::Water && sy <= SEA_LEVEL + 20 {
                    let z = Entity::new(sx as f32 + 0.1, sy as f32 - 2.0, ZOMBIE_MAX_HP);
                    self.zombies.push(z);
                }
            }
        }

        let day = !self.is_night();
        let (px, py) = self.player.center();
        let mut player_hit = false;

        for z in &mut self.zombies {
            // Burn in sunlight (only if exposed to sky).
            if day {
                let (zx, zy) = z.center();
                if self.world.surface_at(zx as i32) >= zy as i32 {
                    z.hp = 0;
                    continue;
                }
            }
            // Chase the player.
            let (zx, _) = z.center();
            let dir = if px > zx + 0.5 {
                1.0
            } else if px < zx - 0.5 {
                -1.0
            } else {
                0.0
            };
            z.vx = dir * 0.22;
            let before_x = z.x;
            z.step_physics(&self.world);
            // Jump if stuck against a wall.
            if dir != 0.0 && (z.x - before_x).abs() < 0.01 && z.on_ground {
                z.try_jump(&self.world);
            }
            // Contact damage.
            if z.overlaps(&self.player) && self.time >= self.zombie_hit_cooldown {
                player_hit = true;
                let kb = if px > z.center().0 { 0.9 } else { -0.9 };
                self.player.vx += kb;
                self.player.vy -= 0.35;
            }
            let _ = py;
        }

        if player_hit {
            self.zombie_hit_cooldown = self.time + 20;
            self.hurt_player(3, "A zombie hits you!");
        }

        self.zombies.retain(|z| z.hp > 0);
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
        let data = SaveData {
            seed: self.seed,
            tiles: self.world.to_bytes(),
            spawn: self.world.spawn,
            player_x: self.player.x,
            player_y: self.player.y,
            hp: self.player.hp,
            time: self.time,
            inv: self.inv.iter().map(|(b, n)| (b.to_u8(), *n)).collect(),
            hotbar: self
                .hotbar
                .iter()
                .map(|s| s.map(|b| b.to_u8() as i16).unwrap_or(-1))
                .collect(),
            selected: self.selected,
        };
        let path = save_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string(&data)?;
        std::fs::write(path, json)
    }

    pub fn load() -> Option<Game> {
        let json = std::fs::read_to_string(save_path()).ok()?;
        let data: SaveData = serde_json::from_str(&json).ok()?;
        let world = World::from_bytes(&data.tiles, data.spawn)?;
        let mut player = Entity::new(data.player_x, data.player_y, PLAYER_MAX_HP);
        player.hp = data.hp;
        let mut hotbar = [None; 9];
        for (i, v) in data.hotbar.iter().take(9).enumerate() {
            if *v >= 0 {
                hotbar[i] = Some(Block::from_u8(*v as u8));
            }
        }
        let mut g = Game {
            world,
            seed: data.seed,
            player,
            zombies: Vec::new(),
            inv: data
                .inv
                .iter()
                .map(|&(b, n)| (Block::from_u8(b), n))
                .collect(),
            hotbar,
            selected: data.selected.min(8),
            cursor: (data.player_x as i32, data.player_y as i32 + 2),
            time: data.time,
            should_quit: false,
            crafting_open: false,
            craft_sel: 0,
            help_open: false,
            msg: None,
            game_over: false,
            map_area: Rect::new(0, 0, 1, 1),
            camera: (0, 0),
            move_dir: 0,
            move_timer: 0,
            hold_mode: false,
            held_left: KeyHold::default(),
            held_right: KeyHold::default(),
            held_jump: KeyHold::default(),
            saw_release: false,
            last_damage_tick: data.time,
            zombie_hit_cooldown: 0,
            rng: StdRng::seed_from_u64(data.seed ^ data.time),
        };
        g.say("World loaded. Welcome back!");
        Some(g)
    }
}

#[derive(Serialize, Deserialize)]
struct SaveData {
    seed: u64,
    tiles: Vec<u8>,
    spawn: (i32, i32),
    player_x: f32,
    player_y: f32,
    hp: i32,
    time: u64,
    inv: Vec<(u8, u32)>,
    hotbar: Vec<i16>,
    selected: usize,
}

pub fn save_path() -> PathBuf {
    home_dir().join(".termcraft").join("save.json")
}

/// Home directory on unix (`HOME`) or Windows (`USERPROFILE`).
pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mining_adds_drops_to_inventory() {
        let mut g = Game::new(11);
        // Put a dirt block right under the cursor and mine it.
        let (px, py) = g.player.center();
        let (tx, ty) = (px as i32 + 1, py as i32);
        g.world.set(tx, ty, Block::Dirt);
        g.cursor = (tx, ty);
        g.mine();
        assert_eq!(g.count(Block::Dirt), 1);
        assert_eq!(g.world.get(tx, ty), Block::Air);
        assert_eq!(g.hotbar[0], Some(Block::Dirt));
    }

    #[test]
    fn crafting_planks_consumes_wood() {
        let mut g = Game::new(11);
        g.add_item(Block::Wood, 2);
        g.craft_sel = 0;
        g.craft();
        assert_eq!(g.count(Block::Wood), 1);
        assert_eq!(g.count(Block::Planks), 4);
    }

    #[test]
    fn daylight_cycles() {
        let mut g = Game::new(11);
        assert!(g.daylight() > 0.9);
        g.time = 1500;
        assert!(g.daylight() < 0.2);
        g.time = DAY_LEN;
        assert!(g.daylight() > 0.9);
    }
}
