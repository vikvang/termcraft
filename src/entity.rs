use crate::block::Block;
use crate::world::World;

pub const GRAVITY: f32 = 0.14;
pub const MAX_FALL: f32 = 1.3;
pub const JUMP_VEL: f32 = -0.95;
pub const SWIM_GRAVITY: f32 = 0.03;
pub const SWIM_MAX: f32 = 0.35;
/// Maximum upward speed while swimming (faster than sinking, so you can
/// climb out of water).
pub const SWIM_RISE: f32 = 0.5;
/// Falling faster than this on landing hurts.
pub const SAFE_FALL_VEL: f32 = 0.98;

#[derive(Clone, Debug)]
pub struct Entity {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub w: f32,
    pub h: f32,
    pub on_ground: bool,
    pub hp: i32,
    pub max_hp: i32,
}

impl Entity {
    pub fn new(x: f32, y: f32, max_hp: i32) -> Entity {
        Entity {
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            w: 0.8,
            h: 1.8,
            on_ground: false,
            hp: max_hp,
            max_hp,
        }
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn overlaps_tile(&self, tx: i32, ty: i32) -> bool {
        self.x < (tx + 1) as f32
            && self.x + self.w > tx as f32
            && self.y < (ty + 1) as f32
            && self.y + self.h > ty as f32
    }

    pub fn overlaps(&self, other: &Entity) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.y < other.y + other.h
            && self.y + self.h > other.y
    }

    pub fn in_water(&self, world: &World) -> bool {
        let (cx, cy) = self.center();
        world.get(cx.floor() as i32, cy.floor() as i32) == Block::Water
            || world.get(cx.floor() as i32, self.y.floor() as i32) == Block::Water
            || world.get(cx.floor() as i32, (self.y + self.h - 0.1).floor() as i32) == Block::Water
    }

    fn collides(world: &World, x: f32, y: f32, w: f32, h: f32) -> bool {
        let x0 = x.floor() as i32;
        let x1 = (x + w - 0.001).floor() as i32;
        let y0 = y.floor() as i32;
        let y1 = (y + h - 0.001).floor() as i32;
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                if world.get(tx, ty).is_solid() {
                    return true;
                }
            }
        }
        false
    }

    /// Applies gravity and velocity with axis-separated collision.
    /// Returns the downward velocity at the moment of landing (for fall damage).
    pub fn step_physics(&mut self, world: &World) -> Option<f32> {
        let swimming = self.in_water(world);
        if swimming {
            self.vy = (self.vy + SWIM_GRAVITY).clamp(-SWIM_RISE, SWIM_MAX);
        } else {
            self.vy = (self.vy + GRAVITY).min(MAX_FALL);
        }

        // Horizontal, in small steps so we never tunnel through a block.
        let steps = (self.vx.abs() / 0.1).ceil().max(1.0) as i32;
        let dx = self.vx / steps as f32;
        for _ in 0..steps {
            if Self::collides(world, self.x + dx, self.y, self.w, self.h) {
                self.vx = 0.0;
                break;
            }
            self.x += dx;
        }

        // Vertical.
        let mut landed = None;
        let steps = (self.vy.abs() / 0.1).ceil().max(1.0) as i32;
        let dy = self.vy / steps as f32;
        self.on_ground = false;
        for _ in 0..steps {
            if Self::collides(world, self.x, self.y + dy, self.w, self.h) {
                if self.vy > 0.0 {
                    self.on_ground = true;
                    landed = Some(self.vy);
                }
                self.vy = 0.0;
                break;
            }
            self.y += dy;
        }

        // friction
        self.vx *= 0.6;
        if self.vx.abs() < 0.02 {
            self.vx = 0.0;
        }
        landed
    }

    pub fn try_jump(&mut self, world: &World) {
        if self.on_ground {
            // A real jump, even when standing on the bottom of shallow water.
            self.vy = JUMP_VEL;
            self.on_ground = false;
        } else if self.in_water(world) {
            self.vy = -SWIM_RISE;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn entity_falls_and_lands() {
        let w = World::generate(3);
        let (sx, sy) = w.spawn;
        let mut e = Entity::new(sx as f32, sy as f32, 20);
        for _ in 0..600 {
            e.step_physics(&w);
        }
        assert!(e.on_ground || e.in_water(&w), "entity should settle");
    }
}
