use crate::block::Block;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

pub const WORLD_W: i32 = 600;
pub const WORLD_H: i32 = 160;
pub const SEA_LEVEL: i32 = 72;

pub struct World {
    tiles: Vec<Block>,
    pub spawn: (i32, i32),
}

fn hash01(seed: u64, x: i64) -> f64 {
    let mut h = seed ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h as f64) / (u64::MAX as f64)
}

/// 1D smooth value noise in [0, 1].
fn vnoise(seed: u64, x: f64) -> f64 {
    let x0 = x.floor();
    let t = x - x0;
    let a = hash01(seed, x0 as i64);
    let b = hash01(seed, x0 as i64 + 1);
    let s = t * t * (3.0 - 2.0 * t);
    a + (b - a) * s
}

impl World {
    pub fn get(&self, x: i32, y: i32) -> Block {
        if !(0..WORLD_W).contains(&x) || y >= WORLD_H {
            return Block::Bedrock; // walls of the world
        }
        if y < 0 {
            return Block::Air; // open sky
        }
        self.tiles[(y * WORLD_W + x) as usize]
    }

    pub fn set(&mut self, x: i32, y: i32, b: Block) {
        if !(0..WORLD_W).contains(&x) || !(0..WORLD_H).contains(&y) {
            return;
        }
        self.tiles[(y * WORLD_W + x) as usize] = b;
    }

    /// y of the first solid block in a column (sky exposure).
    pub fn surface_at(&self, x: i32) -> i32 {
        for y in 0..WORLD_H {
            if self.get(x, y).is_solid() {
                return y;
            }
        }
        WORLD_H
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.tiles.iter().map(|b| b.to_u8()).collect()
    }

    pub fn from_bytes(bytes: &[u8], spawn: (i32, i32)) -> Option<World> {
        if bytes.len() != (WORLD_W * WORLD_H) as usize {
            return None;
        }
        Some(World {
            tiles: bytes.iter().map(|&v| Block::from_u8(v)).collect(),
            spawn,
        })
    }

    pub fn generate(seed: u64) -> World {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut w = World {
            tiles: vec![Block::Air; (WORLD_W * WORLD_H) as usize],
            spawn: (WORLD_W / 2, 0),
        };

        // --- Terrain heightmap -------------------------------------------------
        let mut surface = vec![0i32; WORLD_W as usize];
        for x in 0..WORLD_W {
            let fx = x as f64;
            let h1 = vnoise(seed, fx / 48.0) * 30.0;
            let h2 = vnoise(seed ^ 0x5151, fx / 16.0) * 10.0;
            let h3 = vnoise(seed ^ 0xABCD, fx / 7.0) * 4.0;
            let s = (62.0 + h1 + h2 + h3 - 22.0) as i32;
            surface[x as usize] = s.clamp(20, WORLD_H - 20);
        }

        for x in 0..WORLD_W {
            let s = surface[x as usize];
            for y in s..WORLD_H {
                let b = if y == s {
                    Block::Grass
                } else if y < s + 4 {
                    Block::Dirt
                } else {
                    Block::Stone
                };
                w.set(x, y, b);
            }
        }

        // --- Oceans / lakes: fill dips below sea level, beach the shores -------
        for x in 0..WORLD_W {
            let s = surface[x as usize];
            if s > SEA_LEVEL {
                for y in SEA_LEVEL..s {
                    w.set(x, y, Block::Water);
                }
                // sandy floor
                for y in s..(s + 3).min(WORLD_H) {
                    w.set(x, y, Block::Sand);
                }
            } else if s >= SEA_LEVEL - 2 {
                // beach near the waterline
                w.set(x, s, Block::Sand);
                w.set(x, s + 1, Block::Sand);
            }
        }

        // --- Caves: random-walk worms ------------------------------------------
        for _ in 0..55 {
            let mut cx = rng.gen_range(0.0..WORLD_W as f64);
            let mut cy = rng.gen_range(80.0..(WORLD_H - 12) as f64);
            let mut angle = rng.gen_range(0.0..std::f64::consts::TAU);
            let len = rng.gen_range(50..140);
            for _ in 0..len {
                let r = rng.gen_range(1..=2);
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy <= r * r {
                            let (tx, ty) = (cx as i32 + dx, cy as i32 + dy);
                            if matches!(w.get(tx, ty), Block::Stone | Block::Dirt) {
                                w.set(tx, ty, Block::Air);
                            }
                        }
                    }
                }
                angle += rng.gen_range(-0.5..0.5);
                cx += angle.cos();
                cy += angle.sin() * 0.7;
                if cy < 60.0 {
                    cy = 60.0;
                }
            }
        }

        // --- Ore blobs ----------------------------------------------------------
        let blob = |w: &mut World, rng: &mut StdRng, ore: Block, min_y: i32, count: u32| {
            for _ in 0..count {
                let bx = rng.gen_range(0..WORLD_W);
                let by = rng.gen_range(min_y..WORLD_H - 4);
                let (mut x, mut y) = (bx, by);
                for _ in 0..rng.gen_range(3..7) {
                    if w.get(x, y) == Block::Stone {
                        w.set(x, y, ore);
                    }
                    x += rng.gen_range(-1..=1);
                    y += rng.gen_range(-1..=1);
                }
            }
        };
        blob(&mut w, &mut rng, Block::CoalOre, 70, 130);
        blob(&mut w, &mut rng, Block::IronOre, 95, 80);

        // --- Bedrock floor -------------------------------------------------------
        for x in 0..WORLD_W {
            w.set(x, WORLD_H - 1, Block::Bedrock);
            w.set(x, WORLD_H - 2, Block::Bedrock);
            if hash01(seed ^ 0xBED, x as i64) > 0.5 {
                w.set(x, WORLD_H - 3, Block::Bedrock);
            }
        }

        // --- Trees -----------------------------------------------------------------
        let mut x = rng.gen_range(2..10);
        while x < WORLD_W - 3 {
            let s = surface[x as usize];
            if w.get(x, s) == Block::Grass && s <= SEA_LEVEL {
                let trunk_h = rng.gen_range(3..6);
                for i in 1..=trunk_h {
                    w.set(x, s - i, Block::Wood);
                }
                let top = s - trunk_h;
                for dy in -2i32..=0 {
                    for dx in -2..=2i32 {
                        if dx.abs() + dy.abs() < 4 && !(dx == 0 && dy >= -1) {
                            let (lx, ly) = (x + dx, top + dy - 1);
                            if w.get(lx, ly) == Block::Air {
                                w.set(lx, ly, Block::Leaves);
                            }
                        }
                    }
                }
                w.set(x, top - 1, Block::Leaves);
            }
            x += rng.gen_range(6..15);
        }

        // --- Spawn point: a dry column near the middle ------------------------------
        let mut best = WORLD_W / 2;
        for dx in 0..WORLD_W / 2 {
            for sx in [WORLD_W / 2 + dx, WORLD_W / 2 - dx] {
                if (0..WORLD_W).contains(&sx) {
                    let s = w.surface_at(sx);
                    if s <= SEA_LEVEL && w.get(sx, s) != Block::Water {
                        best = sx;
                        break;
                    }
                }
            }
            if best != WORLD_W / 2 || dx == 0 {
                let s = w.surface_at(best);
                if s <= SEA_LEVEL {
                    break;
                }
            }
        }
        let sy = w.surface_at(best);
        w.spawn = (best, sy - 3);
        w
    }

    /// Very simple cellular water flow: water falls into air below, then
    /// spreads sideways. Sources are infinite (spreading duplicates water).
    pub fn flow_water(&mut self) {
        let mut changes: Vec<(i32, i32)> = Vec::new();
        for y in (0..WORLD_H - 1).rev() {
            for x in 0..WORLD_W {
                if self.get(x, y) != Block::Water {
                    continue;
                }
                if self.get(x, y + 1) == Block::Air {
                    changes.push((x, y + 1));
                } else if self.get(x, y + 1).is_solid() || self.get(x, y + 1) == Block::Water {
                    if self.get(x - 1, y) == Block::Air {
                        changes.push((x - 1, y));
                    }
                    if self.get(x + 1, y) == Block::Air {
                        changes.push((x + 1, y));
                    }
                }
            }
        }
        for (x, y) in changes {
            self.set(x, y, Block::Water);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_generates_with_valid_spawn() {
        let w = World::generate(42);
        let (sx, sy) = w.spawn;
        assert!((0..WORLD_W).contains(&sx));
        assert!((0..WORLD_H).contains(&sy));
        // spawn should be in open air
        assert!(!w.get(sx, sy).is_solid());
        assert!(!w.get(sx, sy + 1).is_solid());
    }

    #[test]
    fn world_roundtrips_through_bytes() {
        let w = World::generate(7);
        let bytes = w.to_bytes();
        let w2 = World::from_bytes(&bytes, w.spawn).unwrap();
        for y in 0..WORLD_H {
            for x in 0..WORLD_W {
                assert_eq!(w.get(x, y), w2.get(x, y));
            }
        }
    }

    #[test]
    fn bedrock_floor_exists() {
        let w = World::generate(1);
        for x in 0..WORLD_W {
            assert_eq!(w.get(x, WORLD_H - 1), Block::Bedrock);
        }
    }
}
