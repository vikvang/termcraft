use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WBlock, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::block::{Block, Rgb};
use crate::game::RECIPES;
use crate::game3::Game3;

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

struct Scene<'a> {
    g: &'a Game3,
    day: f32,
    sky_zenith: Rgb,
    sky_horizon: Rgb,
    torches: Vec<(f32, f32, f32)>,
    target: Option<(i32, i32, i32)>,
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

    /// Casts one ray and returns the final pixel color.
    fn cast(&self, o: (f32, f32, f32), d: (f32, f32, f32)) -> Rgb {
        let w = &self.g.world;
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
        let mut water_seen = false;

        while t <= MAX_DIST {
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
            let (dx, dy, dz) = (
                x as f32 - eye.0,
                y as f32 - eye.1,
                z as f32 - eye.2,
            );
            dx * dx + dy * dy + dz * dz < (MAX_DIST + TORCH_RADIUS3).powi(2)
        })
        .map(|&(x, y, z)| (x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5))
        .collect();
    let target = g.target().map(|h| (h.x, h.y, h.z));

    let scene = Scene {
        g,
        day,
        sky_zenith: lerp((8, 10, 30), (96, 160, 235), dn),
        sky_horizon: lerp((24, 28, 58), (178, 212, 242), dn),
        torches,
        target,
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

    // Crosshair.
    let (ccx, ccy) = (view.x + view.width / 2, view.y + view.height / 2);
    if let Some(cell) = buf.cell_mut((ccx, ccy)) {
        cell.set_char('┼').set_fg(Color::White);
    }

    draw_hud(f, g, area, hud_h);
    if g.crafting_open {
        draw_crafting(f, g, area);
    }
    if g.game_over {
        draw_game_over(f, area);
    }
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

    let help = "w/a/s/d move  ←↑→↓ look  space jump  x/click mine  z/right-click place  1-9 hotbar  c craft  F5 save  q quit";
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
            style.add_modifier(Modifier::BOLD).bg(Color::Rgb(50, 50, 60))
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
        assert!(diffs > 200, "view barely changed when pitching down ({diffs} px)");
    }
}
