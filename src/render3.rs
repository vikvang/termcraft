use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WBlock, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::block::{Block, Rgb};
use crate::game::{PLAYER_MAX_HP, RECIPES};
use crate::game3::{peer_box, ray_aabb, raycast, Game3, PLAYER_H};

const MAX_DIST: f32 = 60.0;
const HFOV: f32 = 1.5; // ~86 degrees horizontal
const AMBIENT: f32 = 0.18;
const TORCH_RADIUS3: f32 = 8.0;

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

fn mul(c: Rgb, f: f32) -> Rgb {
    let f = f.clamp(0.0, 1.6);
    (
        (c.0 as f32 * f).min(255.0) as u8,
        (c.1 as f32 * f).min(255.0) as u8,
        (c.2 as f32 * f).min(255.0) as u8,
    )
}

/// Small deterministic per-block brightness variation for a textured look.
fn dither(x: i32, y: i32, z: i32) -> f32 {
    let mut h = (x as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        .wrapping_add((z as u64).wrapping_mul(0x1656_67B1_9E37_79F9));
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    0.92 + 0.16 * ((h % 1000) as f32 / 1000.0)
}

/// Everything the raymarcher needs to draw one other player.
struct PeerVis {
    min: (f32, f32, f32),
    max: (f32, f32, f32),
    tint: Rgb,
    /// Feet height, used to split the avatar into head / torso / legs.
    foot_y: f32,
}

struct Scene<'a> {
    g: &'a Game3,
    day: f32,
    sky_zenith: Rgb,
    sky_horizon: Rgb,
    torches: Vec<(f32, f32, f32)>,
    target: Option<(i32, i32, i32)>,
    peers: Vec<PeerVis>,
}

impl<'a> Scene<'a> {
    fn sky(&self, dir_y: f32) -> Rgb {
        let t = dir_y.clamp(0.0, 1.0);
        if dir_y < 0.0 {
            return mul(self.sky_horizon, 0.6);
        }
        lerp(self.sky_horizon, self.sky_zenith, t)
    }

    fn torch_light(&self, px: f32, py: f32, pz: f32) -> f32 {
        let mut l = 0.0f32;
        for &(tx, ty, tz) in &self.torches {
            let (dx, dy, dz) = (tx - px, ty - py, tz - pz);
            let d = (dx * dx + dy * dy + dz * dz).sqrt();
            if d < TORCH_RADIUS3 {
                l = l.max(1.0 - d / TORCH_RADIUS3);
            }
        }
        l
    }

    /// Nearest other player along the ray: distance plus its shaded color.
    fn peer_hit(&self, o: (f32, f32, f32), d: (f32, f32, f32)) -> Option<(f32, Rgb)> {
        let mut best: Option<(f32, Rgb)> = None;
        for p in &self.peers {
            let Some(t) = ray_aabb(o, d, p.min, p.max) else {
                continue;
            };
            if t <= 0.0 || t > MAX_DIST || best.is_some_and(|(bt, _)| t >= bt) {
                continue;
            }
            // Body part from the height of the hit point.
            let hy = o.1 + d.1 * t - p.foot_y;
            let shade = if hy > PLAYER_H - 0.45 {
                1.25 // head catches the light
            } else if hy > 0.75 {
                1.0 // torso
            } else {
                0.7 // legs
            };
            // Light avatars like the block they stand in front of.
            let lit = self
                .day
                .max(self.torch_light(o.0 + d.0 * t, o.1 + d.1 * t, o.2 + d.2 * t))
                .clamp(0.25, 1.0);
            best = Some((t, mul(p.tint, shade * lit)));
        }
        best
    }

    /// Casts one ray and returns the final pixel color.
    fn cast(&self, o: (f32, f32, f32), d: (f32, f32, f32)) -> Rgb {
        let w = &self.g.world;
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
        let mut water_seen = false;
        let peer = self.peer_hit(o, d);

        while t <= MAX_DIST {
            // A player standing in this cell (or a nearer one) hides the
            // blocks behind them.
            if let Some((pt, pc)) = peer {
                if pt <= t {
                    let fog = (pt / MAX_DIST).powf(1.4) * 0.9;
                    let mut c = lerp(pc, self.sky(0.1), fog);
                    if water_seen {
                        c = lerp(c, (30, 70, 160), 0.55);
                    }
                    return c;
                }
            }
            let b = w.get(ix, iy, iz);
            if b == Block::Water {
                water_seen = true;
            } else if b != Block::Air && t > 0.0 {
                // --- Solid hit ------------------------------------------------
                let base = mul(b.color3d(), dither(ix, iy, iz));
                let face = match normal {
                    (_, 1, _) => 1.0,
                    (_, -1, _) => 0.45,
                    (x, _, _) if x != 0 => 0.8,
                    _ => 0.62,
                };
                // Sky exposure of the air cell this face borders.
                let (ax, ay, az) = (ix + normal.0, iy + normal.1, iz + normal.2);
                let exposed = ay > w.height_at(ax, az);
                let mut light = if exposed { self.day } else { AMBIENT };
                let p = (o.0 + d.0 * t, o.1 + d.1 * t, o.2 + d.2 * t);
                light = light.max(self.torch_light(p.0, p.1, p.2));
                if b == Block::Torch {
                    light = 1.0;
                }
                let mut c = mul(base, face * light.clamp(0.06, 1.0));
                // Distance fog toward the horizon color.
                let fog = (t / MAX_DIST).powf(1.4) * 0.9;
                c = lerp(c, self.sky(0.1), fog);
                // Underwater tint.
                if water_seen {
                    c = lerp(c, (30, 70, 160), 0.55);
                }
                // Targeted block highlight.
                if self.target == Some((ix, iy, iz)) {
                    c = lerp(c, (255, 255, 255), 0.28);
                }
                return c;
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
        let sky = self.sky(d.1);
        if water_seen {
            lerp(sky, (30, 70, 160), 0.6)
        } else {
            sky
        }
    }
}

pub fn draw(f: &mut Frame, g: &mut Game3) {
    let area = f.area();
    if area.width < 40 || area.height < 12 {
        f.render_widget(
            Paragraph::new("Terminal too small for TermCraft - resize to at least 40x12."),
            area,
        );
        return;
    }

    let hud_h = 3u16;
    let view = Rect::new(area.x, area.y, area.width, area.height - hud_h);

    let day = g.daylight();
    let dn = ((day - 0.15) / 0.85).clamp(0.0, 1.0);
    let eye = g.eye();
    let torches: Vec<(f32, f32, f32)> = g
        .world
        .torches
        .iter()
        .filter(|&&(x, y, z)| {
            let (dx, dy, dz) = (x as f32 - eye.0, y as f32 - eye.1, z as f32 - eye.2);
            dx * dx + dy * dy + dz * dz < (MAX_DIST + TORCH_RADIUS3).powi(2)
        })
        .map(|&(x, y, z)| (x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5))
        .collect();
    let target = g.target().map(|h| (h.x, h.y, h.z));
    let peers: Vec<PeerVis> = g
        .peers
        .values()
        .map(|p| {
            let (min, max) = peer_box(&p.st);
            PeerVis {
                min,
                max,
                tint: p.tint(),
                foot_y: p.st.y,
            }
        })
        .collect();

    let scene = Scene {
        g,
        day,
        sky_zenith: lerp((8, 10, 30), (96, 160, 235), dn),
        sky_horizon: lerp((24, 28, 58), (178, 212, 242), dn),
        torches,
        target,
        peers,
    };

    // Camera basis (y-up).
    let fwd = g.forward();
    let (sy, cy) = g.yaw.sin_cos();
    let right = (-sy, 0.0f32, cy);
    let up = (
        right.1 * fwd.2 - right.2 * fwd.1,
        right.2 * fwd.0 - right.0 * fwd.2,
        right.0 * fwd.1 - right.1 * fwd.0,
    );

    let pw = view.width as i32;
    let ph = view.height as i32 * 2; // half-block doubling
    let tan_h = (HFOV / 2.0).tan();
    let tan_v = tan_h * ph as f32 / pw as f32;

    let mut pixels: Vec<Rgb> = vec![(0, 0, 0); (pw * ph) as usize];
    for py in 0..ph {
        let v = (1.0 - 2.0 * (py as f32 + 0.5) / ph as f32) * tan_v;
        for px in 0..pw {
            let u = (2.0 * (px as f32 + 0.5) / pw as f32 - 1.0) * tan_h;
            let dir = (
                fwd.0 + right.0 * u + up.0 * v,
                fwd.1 + right.1 * u + up.1 * v,
                fwd.2 + right.2 * u + up.2 * v,
            );
            let len = (dir.0 * dir.0 + dir.1 * dir.1 + dir.2 * dir.2).sqrt();
            let dir = (dir.0 / len, dir.1 / len, dir.2 / len);
            pixels[(py * pw + px) as usize] = scene.cast(eye, dir);
        }
    }

    let buf = f.buffer_mut();
    for cy in 0..view.height {
        for cx in 0..view.width {
            let top = pixels[((cy as i32 * 2) * pw + cx as i32) as usize];
            let bot = pixels[((cy as i32 * 2 + 1) * pw + cx as i32) as usize];
            if let Some(cell) = buf.cell_mut((view.x + cx, view.y + cy)) {
                cell.set_char('▀')
                    .set_fg(Color::Rgb(top.0, top.1, top.2))
                    .set_bg(Color::Rgb(bot.0, bot.1, bot.2));
            }
        }
    }

    // Crosshair. Turns red when it's on another player.
    let (ccx, ccy) = (view.x + view.width / 2, view.y + view.height / 2);
    let on_peer = g.peer_in_crosshair().is_some();
    if let Some(cell) = buf.cell_mut((ccx, ccy)) {
        cell.set_char(if on_peer { '✕' } else { '┼' })
            .set_fg(if on_peer {
                Color::Rgb(255, 90, 90)
            } else {
                Color::White
            });
    }

    draw_nametags(
        f,
        g,
        view,
        Basis {
            eye,
            fwd,
            right,
            up,
            tan_h,
            tan_v,
        },
    );
    draw_chat(f, g, view);
    draw_hud(f, g, area, hud_h);
    if g.players_open {
        draw_players(f, g, area);
    }
    if g.crafting_open {
        draw_crafting(f, g, area);
    }
    if g.help_open {
        // Creative rewrites the movement bindings; keep survival text otherwise.
        let movement: &[(&str, &str)] = if g.creative {
            &[
                ("Movement", ""),
                ("  w / a / s / d", "fly where you look (includes pitch)"),
                ("  arrow keys", "look around (or drag the mouse)"),
                ("  space", "rise (hold to keep rising)"),
                ("  f", "descend (hold to keep descending)"),
            ]
        } else {
            &[
                ("Movement", ""),
                ("  w / a / s / d", "move (relative to where you look)"),
                ("  arrow keys", "look around (or drag the mouse)"),
                ("  space", "jump / swim up (hold to keep jumping)"),
            ]
        };
        let rest: &[(&str, &str)] = &[
            ("Actions", ""),
            (
                "  x / Enter / left-click",
                "mine the block under the crosshair",
            ),
            ("  z / right-click", "place selected block on targeted face"),
            ("  1-9", "select hotbar slot"),
            ("  c", "crafting menu"),
            ("Multiplayer", ""),
            ("  --seed <N>", "same seed = same shared world"),
            ("  t", "chat with everyone in the world"),
            ("  Tab", "who's online"),
            ("  x on a player", "punch them"),
            ("Game", ""),
            ("  F5 / Ctrl+S", "save"),
            ("  h / ?", "toggle this help"),
            ("  q / Esc", "quit (autosaves)"),
            ("", ""),
            (
                "Tip",
                if g.creative {
                    "creative: no gravity or fall damage - still collides with blocks"
                } else {
                    "craft torches before nightfall - caves are dark!"
                },
            ),
        ];
        let mut entries = Vec::with_capacity(movement.len() + rest.len());
        entries.extend_from_slice(movement);
        entries.extend_from_slice(rest);
        draw_help(f, area, &entries);
    }
    if g.game_over {
        draw_game_over(f, area);
    }
}

/// Camera basis, enough to project a world point back onto the screen.
struct Basis {
    eye: (f32, f32, f32),
    fwd: (f32, f32, f32),
    right: (f32, f32, f32),
    up: (f32, f32, f32),
    tan_h: f32,
    tan_v: f32,
}

fn dot(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}

/// Floating names (and a health bar) over every visible player.
fn draw_nametags(f: &mut Frame, g: &Game3, view: Rect, b: Basis) {
    if g.peers.is_empty() {
        return;
    }
    let (pw, ph) = (view.width as f32, view.height as f32 * 2.0);
    for p in g.peers.values() {
        // Aim the label just above the head.
        let head = (p.st.x, p.st.y + PLAYER_H + 0.25, p.st.z);
        let rel = (head.0 - b.eye.0, head.1 - b.eye.1, head.2 - b.eye.2);
        let z = dot(rel, b.fwd);
        if z < 0.4 {
            continue; // behind us
        }
        let dist = (rel.0 * rel.0 + rel.1 * rel.1 + rel.2 * rel.2).sqrt();
        if dist > MAX_DIST {
            continue;
        }
        // Hide the tag when the player is behind a wall.
        let dir = (rel.0 / dist, rel.1 / dist, rel.2 / dist);
        if let Some(hit) = raycast(&g.world, b.eye, dir, dist) {
            if hit.t < dist - 0.6 {
                continue;
            }
        }
        let sx = (dot(rel, b.right) / z / b.tan_h + 1.0) * 0.5 * pw;
        let sy = (1.0 - dot(rel, b.up) / z / b.tan_v) * 0.5 * ph;
        if !(0.0..pw).contains(&sx) || !(0.0..ph).contains(&sy) {
            continue;
        }
        let hearts =
            (p.st.hp.clamp(0, PLAYER_MAX_HP) as f32 / PLAYER_MAX_HP as f32 * 5.0).round() as usize;
        let label = format!(
            "{} {}{}",
            p.name,
            "♥".repeat(hearts),
            "♡".repeat(5 - hearts.min(5))
        );
        let width = label.chars().count() as u16;
        let col = (sx as u16).saturating_sub(width / 2);
        let row = (sy as u16 / 2).min(view.height.saturating_sub(1));
        let col = col.min(view.width.saturating_sub(width.min(view.width)));
        let c = p.tint();
        let rect = Rect::new(view.x + col, view.y + row, width.min(view.width - col), 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                label,
                Style::default()
                    .fg(Color::Rgb(c.0, c.1, c.2))
                    .bg(Color::Rgb(18, 18, 24))
                    .add_modifier(Modifier::BOLD),
            ))),
            rect,
        );
    }
}

/// Recent chat lines, bottom-left of the viewport.
fn draw_chat(f: &mut Frame, g: &Game3, view: Rect) {
    let lines = g.recent_chat();
    if lines.is_empty() {
        return;
    }
    let h = (lines.len() as u16).min(view.height.saturating_sub(2));
    if h == 0 {
        return;
    }
    let rendered: Vec<Line> = lines
        .iter()
        .rev()
        .take(h as usize)
        .rev()
        .map(|(from, text, _)| {
            let (fg, prefix) = if from == "*" {
                (Color::Rgb(180, 200, 255), String::new())
            } else {
                (Color::Rgb(255, 240, 160), format!("<{from}> "))
            };
            Line::from(vec![
                Span::styled(prefix, Style::default().fg(fg).add_modifier(Modifier::BOLD)),
                Span::styled(text.clone(), Style::default().fg(Color::Rgb(230, 230, 235))),
            ])
        })
        .collect();
    let w = view.width.min(70);
    let rect = Rect::new(view.x, view.y + view.height - h, w, h);
    f.render_widget(
        Paragraph::new(rendered).style(Style::default().bg(Color::Rgb(16, 16, 20))),
        rect,
    );
}

/// Tab overlay: who's in the world, how healthy, and how far away.
fn draw_players(f: &mut Frame, g: &Game3, area: Rect) {
    let rect = centered(area, 46, g.peers.len() as u16 + 4);
    f.render_widget(Clear, rect);
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{:<18}", format!("{} (you)", g.name)),
            Style::default()
                .fg(Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:>3} hp", g.hp.max(0)),
            Style::default().fg(Color::Rgb(230, 120, 120)),
        ),
    ])];
    for p in g.peers.values() {
        let d =
            ((p.st.x - g.px).powi(2) + (p.st.y - g.py).powi(2) + (p.st.z - g.pz).powi(2)).sqrt();
        let c = p.tint();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<18}", p.name),
                Style::default().fg(Color::Rgb(c.0, c.1, c.2)),
            ),
            Span::styled(
                format!("{:>3} hp", p.st.hp.max(0)),
                Style::default().fg(Color::Rgb(230, 120, 120)),
            ),
            Span::styled(
                format!("   {d:>5.1}m away"),
                Style::default().fg(Color::Rgb(170, 170, 180)),
            ),
        ]));
    }
    if g.peers.is_empty() {
        lines.push(Line::from(Span::styled(
            if g.is_multiplayer() {
                "Nobody else here yet."
            } else {
                "Single player - start with --seed <N> to share a world."
            },
            Style::default().fg(Color::Rgb(150, 150, 160)),
        )));
    }
    let title = match g.net_badge() {
        Some(b) => format!(" Players ({b}) - Tab to close "),
        None => " Players - Tab to close ".to_string(),
    };
    let block = WBlock::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Rgb(25, 25, 32)));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

pub fn draw_help(f: &mut Frame, area: Rect, entries: &[(&str, &str)]) {
    let rect = centered(area, 64, entries.len() as u16 + 2);
    f.render_widget(Clear, rect);
    let lines: Vec<Line> = entries
        .iter()
        .map(|(key, desc)| {
            if desc.is_empty() {
                Line::from(Span::styled(
                    key.to_string(),
                    Style::default()
                        .fg(Color::Rgb(255, 220, 100))
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(vec![
                    Span::styled(
                        format!("{key:<26}"),
                        Style::default().fg(Color::Rgb(150, 220, 150)),
                    ),
                    Span::styled(
                        desc.to_string(),
                        Style::default().fg(Color::Rgb(210, 210, 220)),
                    ),
                ])
            }
        })
        .collect();
    let block = WBlock::default()
        .title(" Controls - h or Esc to close ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Rgb(25, 25, 32)));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

fn draw_hud(f: &mut Frame, g: &Game3, area: Rect, hud_h: u16) {
    let y0 = area.y + area.height - hud_h;

    let full = (g.hp.max(0) / 2) as usize;
    let empty = 10usize.saturating_sub(full);
    let target_name = g.target().map(|h| h.block.name()).unwrap_or("-");
    let mut spans = vec![
        Span::styled(
            "♥".repeat(full),
            Style::default().fg(Color::Rgb(230, 60, 60)),
        ),
        Span::styled(
            "♡".repeat(empty),
            Style::default().fg(Color::Rgb(120, 60, 60)),
        ),
        Span::raw("  "),
        Span::styled(
            format!(
                "{} Day {}",
                if g.is_night() { "☾" } else { "☀" },
                g.day_number()
            ),
            Style::default().fg(if g.is_night() {
                Color::Rgb(170, 180, 255)
            } else {
                Color::Rgb(255, 220, 100)
            }),
        ),
        Span::raw(format!(
            "  x:{} y:{} z:{}  ",
            g.px as i32, g.py as i32, g.pz as i32
        )),
        Span::styled(
            format!("[{target_name}]"),
            Style::default().fg(Color::Rgb(200, 200, 255)),
        ),
    ];
    if g.creative {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "✈ creative",
            Style::default()
                .fg(Color::Rgb(160, 210, 255))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(badge) = g.net_badge() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("⚭ {badge}"),
            Style::default()
                .fg(Color::Rgb(140, 230, 180))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some((m, _)) = &g.msg {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            m.clone(),
            Style::default()
                .fg(Color::Rgb(255, 240, 160))
                .add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(20, 20, 24))),
        Rect::new(area.x, y0, area.width, 1),
    );

    // Hotbar.
    let mut spans: Vec<Span> = Vec::new();
    for (i, slot) in g.hotbar.iter().enumerate() {
        let sel = i == g.selected;
        let bracket = if sel {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(110, 110, 110))
        };
        let inner = match slot {
            Some(b) => {
                let n = g.count(*b);
                let c = b.color3d();
                let st = if n == 0 {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::Rgb(c.0, c.1, c.2))
                };
                Span::styled(format!("{}{:<3}", b.glyph(), n.min(999)), st)
            }
            None => Span::raw("    "),
        };
        spans.push(Span::styled(format!("{}[", i + 1), bracket));
        spans.push(inner);
        spans.push(Span::styled("] ", bracket));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(20, 20, 24))),
        Rect::new(area.x, y0 + 1, area.width, 1),
    );

    // While typing, the hint row becomes the chat prompt.
    if g.chat_open {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "say> ",
                    Style::default()
                        .fg(Color::Rgb(255, 240, 160))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{}█", g.chat_input),
                    Style::default().fg(Color::White),
                ),
            ]))
            .style(Style::default().bg(Color::Rgb(30, 30, 40))),
            Rect::new(area.x, y0 + 2, area.width, 1),
        );
        return;
    }

    let help = if g.creative {
        if g.is_multiplayer() {
            "h help  w/a/s/d fly  space up  f down  x mine  z place  c craft  t chat  Tab players  q quit"
        } else {
            "h help  w/a/s/d fly  ←↑→↓ look  space up  f down  x mine  z place  c craft  q quit"
        }
    } else if g.is_multiplayer() {
        "h help  w/a/s/d move  x mine  z place  c craft  t chat  Tab players  q quit"
    } else {
        "h help  w/a/s/d move  ←↑→↓ look  space jump  x mine  z place  c craft  q quit"
    };
    f.render_widget(
        Paragraph::new(help).style(
            Style::default()
                .fg(Color::Rgb(140, 140, 150))
                .bg(Color::Rgb(20, 20, 24)),
        ),
        Rect::new(area.x, y0 + 2, area.width, 1),
    );
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn draw_crafting(f: &mut Frame, g: &Game3, area: Rect) {
    let rect = centered(area, 56, RECIPES.len() as u16 + 6);
    f.render_widget(Clear, rect);

    let mut lines: Vec<Line> = Vec::new();
    for (i, r) in RECIPES.iter().enumerate() {
        let sel = i == g.craft_sel;
        let craftable = g.can_craft(i);
        let style = if craftable {
            Style::default().fg(Color::Rgb(140, 230, 140))
        } else {
            Style::default().fg(Color::Rgb(110, 110, 110))
        };
        let style = if sel {
            style
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(50, 50, 60))
        } else {
            style
        };
        let prefix = if sel { "> " } else { "  " };
        lines.push(Line::from(Span::styled(
            format!("{prefix}{}", r.label),
            style,
        )));
    }
    lines.push(Line::raw(""));
    let inv_str = if g.inv.is_empty() {
        "Inventory: (empty - go mine something!)".to_string()
    } else {
        let items: Vec<String> = g
            .inv
            .iter()
            .filter(|(_, n)| **n > 0)
            .map(|(b, n)| format!("{} x{}", b.name(), n))
            .collect();
        format!("Inventory: {}", items.join(", "))
    };
    lines.push(Line::from(Span::styled(
        inv_str,
        Style::default().fg(Color::Rgb(200, 200, 210)),
    )));

    let block = WBlock::default()
        .title(" Crafting - ↑/↓ select, Enter craft, Esc close ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Rgb(25, 25, 32)));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

fn draw_game_over(f: &mut Frame, area: Rect) {
    let rect = centered(area, 44, 7);
    f.render_widget(Clear, rect);
    let lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            "Y O U   D I E D",
            Style::default()
                .fg(Color::Rgb(255, 70, 70))
                .add_modifier(Modifier::BOLD),
        ))
        .centered(),
        Line::raw(""),
        Line::from("r - respawn    q - quit").centered(),
    ];
    let block = WBlock::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Rgb(40, 12, 12)));
    f.render_widget(Paragraph::new(lines).block(block), rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn rgb_of(c: Color) -> Rgb {
        match c {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (0, 0, 0),
        }
    }

    fn render_frame(g: &mut Game3) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(80, 24);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| draw(f, g)).unwrap();
        term.backend().buffer().clone()
    }

    fn count_blueish(buf: &ratatui::buffer::Buffer, rows: std::ops::Range<u16>) -> (u32, u32) {
        let mut blue = 0u32;
        let mut total = 0u32;
        for y in rows {
            for x in 0..80u16 {
                let c = rgb_of(buf[(x, y)].fg);
                if c.2 > c.0 && c.2 > c.1 {
                    blue += 1;
                }
                total += 1;
            }
        }
        (blue, total)
    }

    #[test]
    fn view_shows_sky_above_and_terrain_below() {
        let mut g = Game3::new(7);
        for _ in 0..100 {
            g.tick(); // settle on the ground, daytime
        }
        // Look straight up: should be dominated by blue daytime sky.
        g.pitch = 1.4;
        let buf = render_frame(&mut g);
        let (blue, total) = count_blueish(&buf, 0..10);
        assert!(
            blue * 2 > total,
            "expected mostly sky when looking up ({blue}/{total} blue-ish)"
        );
        // Crosshair is drawn at the center of the viewport.
        assert_eq!(buf[(40u16, 10u16)].symbol(), "┼");
        // Look straight down: should be dominated by terrain colors.
        g.pitch = -1.4;
        let buf = render_frame(&mut g);
        let (blue, total) = count_blueish(&buf, 5..15);
        assert!(
            blue * 2 < total,
            "expected mostly terrain when looking down ({blue}/{total} blue-ish)"
        );
    }

    #[test]
    fn other_players_are_drawn_with_a_nametag() {
        use crate::net::{Peer, PlayerState};

        let mut g = Game3::new(7);
        for _ in 0..100 {
            g.tick(); // settle on the ground
        }
        g.yaw = 0.0;
        g.pitch = 0.0;
        let before = render_frame(&mut g);

        // Clear a corridor ahead and stand a peer in it, 3 blocks away.
        let (bx, by, bz) = (g.px as i32, g.py as i32, g.pz as i32);
        for x in bx..=bx + 6 {
            for y in by..by + 4 {
                for z in bz - 2..=bz + 2 {
                    g.world.set(x, y, z, Block::Air);
                }
            }
        }
        let st = PlayerState {
            x: g.px + 3.0,
            y: g.py,
            z: g.pz,
            yaw: std::f32::consts::PI, // facing us
            pitch: 0.0,
            hp: PLAYER_MAX_HP,
        };
        g.peers.insert(
            4,
            Peer {
                id: 4,
                name: "buddy".into(),
                st,
                last_seen: g.time,
            },
        );
        let after = render_frame(&mut g);

        // The avatar should change a solid chunk of pixels near the center.
        let mut changed = 0;
        for y in 6..16u16 {
            for x in 30..50u16 {
                if before[(x, y)].fg != after[(x, y)].fg {
                    changed += 1;
                }
            }
        }
        assert!(changed > 20, "peer avatar barely rendered ({changed} px)");

        // ...and their name should appear somewhere on screen.
        let text: String = (0..24u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .map(|p| after[p].symbol().to_string())
            .collect();
        assert!(text.contains("buddy"), "no nametag drawn");
    }

    #[test]
    fn looking_down_changes_the_view() {
        let mut g = Game3::new(7);
        for _ in 0..100 {
            g.tick();
        }
        g.pitch = 0.0;
        let level = render_frame(&mut g);
        g.pitch = -1.3;
        let down = render_frame(&mut g);
        let mut diffs = 0;
        for y in 0..10u16 {
            for x in 0..80u16 {
                if level[(x, y)].fg != down[(x, y)].fg {
                    diffs += 1;
                }
            }
        }
        assert!(
            diffs > 200,
            "view barely changed when pitching down ({diffs} px)"
        );
    }
}
