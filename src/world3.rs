use crate::block::Block;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

pub const W3: i32 = 128; // x
pub const H3: i32 = 64; // y (up)
pub const D3: i32 = 128; // z
pub const SEA3: i32 = 24;

pub struct World3 {
    tiles: Vec<Block>,
    /// Topmost solid y per (x, z) column, -1 if none. Index: z * W3 + x.
    heights: Vec<i32>,
    pub torches: Vec<(i32, i32, i32)>,
    pub spawn: (f32, f32, f32),
}

#[inline]
fn idx(x: i32, y: i32, z: i32) -> usize {
    ((y * D3 + z) * W3 + x) as usize
}

fn hash01(seed: u64, x: i64, z: i64) -> f64 {
    let mut h = seed
        ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h as f64) / (u64::MAX as f64)
}

/// 2D smooth value noise in [0, 1].
fn vnoise2(seed: u64, x: f64, z: f64) -> f64 {
    let x0 = x.floor();
    let z0 = z.floor();
    let tx = x - x0;
    let tz = z - z0;
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sz = tz * tz * (3.0 - 2.0 * tz);
    let (xi, zi) = (x0 as i64, z0 as i64);
    let a = hash01(seed, xi, zi);
    let b = hash01(seed, xi + 1, zi);
    let c = hash01(seed, xi, zi + 1);
    let d = hash01(seed, xi + 1, zi + 1);
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    ab + (cd - ab) * sz
}

impl World3 {
    pub fn get(&self, x: i32, y: i32, z: i32) -> Block {
        if !(0..W3).contains(&x) || !(0..D3).contains(&z) || y < 0 {
            return Block::Bedrock; // world walls and floor
        }
        if y >= H3 {
            return Block::Air; // open sky
        }
        self.tiles[idx(x, y, z)]
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, b: Block) {
        if !(0..W3).contains(&x) || !(0..D3).contains(&z) || !(0..H3).contains(&y) {
            return;
        }
        let i = idx(x, y, z);
        let old = self.tiles[i];
        if old == b {
            return;
        }
        if old == Block::Torch {
            self.torches.retain(|&t| t != (x, y, z));
        }
        self.tiles[i] = b;
        if b == Block::Torch {
            self.torches.push((x, y, z));
        }
        // Keep the heights cache in sync.
        let ci = (z * W3 + x) as usize;
        if b.is_solid() {
            if y > self.heights[ci] {
                self.heights[ci] = y;
            }
        } else if self.heights[ci] == y {
            let mut ny = -1;
            for yy in (0..y).rev() {
                if self.tiles[idx(x, yy, z)].is_solid() {
                    ny = yy;
                    break;
                }
            }
            self.heights[ci] = ny;
        }
    }

    /// Topmost solid y of a column (-1 if empty). Out of bounds counts as covered.
    pub fn height_at(&self, x: i32, z: i32) -> i32 {
        if !(0..W3).contains(&x) || !(0..D3).contains(&z) {
            return H3;
        }
        self.heights[(z * W3 + x) as usize]
    }

    fn compute_heights(&mut self) {
        for z in 0..D3 {
            for x in 0..W3 {
                let mut h = -1;
                for y in (0..H3).rev() {
                    if self.tiles[idx(x, y, z)].is_solid() {
                        h = y;
                        break;
                    }
                }
                self.heights[(z * W3 + x) as usize] = h;
            }
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.tiles.iter().map(|b| b.to_u8()).collect()
    }

    pub fn from_bytes(bytes: &[u8], spawn: (f32, f32, f32)) -> Option<World3> {
        if bytes.len() != (W3 * H3 * D3) as usize {
            return None;
        }
        let tiles: Vec<Block> = bytes.iter().map(|&v| Block::from_u8(v)).collect();
        let mut torches = Vec::new();
        for y in 0..H3 {
            for z in 0..D3 {
                for x in 0..W3 {
                    if tiles[idx(x, y, z)] == Block::Torch {
                        torches.push((x, y, z));
                    }
                }
            }
        }
        let mut w = World3 {
            tiles,
            heights: vec![-1; (W3 * D3) as usize],
            torches,
            spawn,
        };
        w.compute_heights();
        Some(w)
    }

    pub fn generate(seed: u64) -> World3 {
        let mut rng = StdRng::seed_from_u64(seed ^ 0x3D);
        let mut w = World3 {
            tiles: vec![Block::Air; (W3 * H3 * D3) as usize],
            heights: vec![-1; (W3 * D3) as usize],
            torches: Vec::new(),
            spawn: (W3 as f32 / 2.0, H3 as f32 - 4.0, D3 as f32 / 2.0),
        };

        // --- Terrain ------------------------------------------------------------
        let mut hmap = vec![0i32; (W3 * D3) as usize];
        for z in 0..D3 {
            for x in 0..W3 {
                let (fx, fz) = (x as f64, z as f64);
                let n1 = vnoise2(seed, fx / 26.0, fz / 26.0) * 17.0;
                let n2 = vnoise2(seed ^ 0x5151, fx / 9.0, fz / 9.0) * 6.0;
                let n3 = vnoise2(seed ^ 0xABCD, fx / 4.0, fz / 4.0) * 2.0;
                let h = (13.0 + n1 + n2 + n3) as i32;
                let h = h.clamp(5, H3 - 12);
                hmap[(z * W3 + x) as usize] = h;
                for y in 0..=h {
                    let b = if y <= 1 {
                        Block::Bedrock
                    } else if y == h {
                        if h <= SEA3 + 1 {
                            Block::Sand
                        } else {
                            Block::Grass
                        }
                    } else if y >= h - 3 {
                        Block::Dirt
                    } else {
                        Block::Stone
                    };
                    w.tiles[idx(x, y, z)] = b;
                }
                // Oceans / lakes
                if h < SEA3 {
                    for y in (h + 1)..=SEA3 {
                        w.tiles[idx(x, y, z)] = Block::Water;
                    }
                }
            }
        }

        // --- Caves: 3D random-walk worms -----------------------------------------
        for _ in 0..70 {
            let mut cx = rng.gen_range(0.0..W3 as f64);
            let mut cy = rng.gen_range(4.0..(SEA3 + 6) as f64);
            let mut cz = rng.gen_range(0.0..D3 as f64);
            let mut yaw = rng.gen_range(0.0..std::f64::consts::TAU);
            let mut pitch: f64 = rng.gen_range(-0.4..0.4);
            for _ in 0..rng.gen_range(40..120) {
                let r = rng.gen_range(1..=2);
                for dy in -r..=r {
                    for dz in -r..=r {
                        for dx in -r..=r {
                            if dx * dx + dy * dy + dz * dz <= r * r {
                                let (tx, ty, tz) =
                                    (cx as i32 + dx, cy as i32 + dy, cz as i32 + dz);
                                if (0..W3).contains(&tx)
                                    && (2..H3).contains(&ty)
                                    && (0..D3).contains(&tz)
                                    && matches!(
                                        w.tiles[idx(tx, ty, tz)],
                                        Block::Stone | Block::Dirt
                                    )
                                {
                                    w.tiles[idx(tx, ty, tz)] = Block::Air;
                                }
                            }
                        }
                    }
                }
                yaw += rng.gen_range(-0.4..0.4);
                pitch = (pitch + rng.gen_range(-0.2..0.2)).clamp(-0.7, 0.7);
                cx += yaw.cos() * pitch.cos();
                cz += yaw.sin() * pitch.cos();
                cy += pitch.sin() * 0.7;
            }
        }

        // --- Ore blobs -------------------------------------------------------------
        let blob = |w: &mut World3, rng: &mut StdRng, ore: Block, max_y: i32, count: u32| {
            for _ in 0..count {
                let (mut x, mut y, mut z) = (
                    rng.gen_range(0..W3),
                    rng.gen_range(2..max_y),
                    rng.gen_range(0..D3),
                );
                for _ in 0..rng.gen_range(3..8) {
                    if (0..W3).contains(&x) && (0..H3).contains(&y) && (0..D3).contains(&z)
                        && w.tiles[idx(x, y, z)] == Block::Stone
                    {
                        w.tiles[idx(x, y, z)] = ore;
                    }
                    x += rng.gen_range(-1..=1);
                    y += rng.gen_range(-1..=1);
                    z += rng.gen_range(-1..=1);
                }
            }
        };
        blob(&mut w, &mut rng, Block::CoalOre, 28, 220);
        blob(&mut w, &mut rng, Block::IronOre, 18, 130);

        // --- Trees -------------------------------------------------------------------
        for _ in 0..150 {
            let x = rng.gen_range(3..W3 - 3);
            let z = rng.gen_range(3..D3 - 3);
            let h = hmap[(z * W3 + x) as usize];
            if h > SEA3 + 1 && w.tiles[idx(x, h, z)] == Block::Grass && h + 7 < H3 {
                let trunk = rng.gen_range(3..6);
                for i in 1..=trunk {
                    w.tiles[idx(x, h + i, z)] = Block::Wood;
                }
                let top = h + trunk;
                for dy in -1..=2i32 {
                    for dz in -2..=2i32 {
                        for dx in -2..=2i32 {
                            let d2 = dx * dx + dz * dz + dy * dy * 2;
                            if d2 <= 5 && !(dx == 0 && dz == 0 && dy <= 0) {
                                let (lx, ly, lz) = (x + dx, top + dy, z + dz);
                                if (0..H3).contains(&ly)
                                    && w.tiles[idx(lx, ly, lz)] == Block::Air
                                {
                                    w.tiles[idx(lx, ly, lz)] = Block::Leaves;
                                }
                            }
                        }
                    }
                }
            }
        }

        w.compute_heights();

        // --- Spawn: nearest dry grass column to the middle ----------------------------
        let (mx, mz) = (W3 / 2, D3 / 2);
        'outer: for r in 0..W3 / 2 {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    let (x, z) = (mx + dx, mz + dz);
                    if !(0..W3).contains(&x) || !(0..D3).contains(&z) {
                        continue;
                    }
                    let h = w.height_at(x, z);
                    if h > SEA3 && w.get(x, h, z) == Block::Grass {
                        w.spawn = (x as f32 + 0.5, h as f32 + 1.0, z as f32 + 0.5);
                        break 'outer;
                    }
                }
            }
        }
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world3_spawn_is_in_open_air() {
        let w = World3::generate(42);
        let (sx, sy, sz) = w.spawn;
        let (x, y, z) = (sx.floor() as i32, sy.floor() as i32, sz.floor() as i32);
        assert!(!w.get(x, y, z).is_solid());
        assert!(!w.get(x, y + 1, z).is_solid());
        assert!(w.get(x, y - 1, z).is_solid());
    }

    #[test]
    fn world3_roundtrips_through_bytes() {
        let w = World3::generate(7);
        let w2 = World3::from_bytes(&w.to_bytes(), w.spawn).unwrap();
        for y in 0..H3 {
            for z in 0..D3 {
                for x in 0..W3 {
                    assert_eq!(w.get(x, y, z), w2.get(x, y, z));
                }
            }
        }
    }

    #[test]
    fn heights_cache_tracks_set() {
        let mut w = World3::generate(5);
        let (x, z) = (10, 10);
        let h = w.height_at(x, z);
        w.set(x, h + 5, z, Block::Stone);
        assert_eq!(w.height_at(x, z), h + 5);
        w.set(x, h + 5, z, Block::Air);
        assert_eq!(w.height_at(x, z), h);
    }

    #[test]
    fn torch_registry_tracks_set() {
        let mut w = World3::generate(5);
        w.set(20, 30, 20, Block::Torch);
        assert!(w.torches.contains(&(20, 30, 20)));
        w.set(20, 30, 20, Block::Air);
        assert!(!w.torches.contains(&(20, 30, 20)));
    }
}
