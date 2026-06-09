use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WBlock, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::block::{Block, Rgb};
use crate::game::{Game, RECIPES};
use crate::world::{WORLD_H, WORLD_W};

const SKY_DAY: Rgb = (118, 176, 240);
const SKY_NIGHT: Rgb = (10, 12, 36);
const CAVE_BG: Rgb = (26, 22, 20);
const MIN_LIGHT: f32 = 0.07;
const TORCH_RADIUS: f32 = 7.0;

fn scale(c: Rgb, l: f32) -> Color {
    Color::Rgb(
        (c.0 as f32 * l) as u8,
        (c.1 as f32 * l) as u8,
        (c.2 as f32 * l) as u8,
    )
}

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

fn blend(a: Rgb, b: Rgb, t: f32) -> Rgb {
    lerp(a, b, t)
}

pub fn draw(f: &mut Frame, game: &mut Game) {
    let area = f.area();
    if area.width < 40 || area.height < 12 {
        let p = Paragraph::new("Terminal too small for TermCraft - resize to at least 40x12.");
        f.render_widget(p, area);
        return;
    }

    let hud_h = 3u16;
    let map = Rect::new(area.x, area.y, area.width, area.height - hud_h);
    game.map_area = map;

    // ---- camera ----------------------------------------------------------
    let (pcx, pcy) = game.player.center();
    let cam_x = (pcx as i32 - map.width as i32 / 2).clamp(0, (WORLD_W - map.width as i32).max(0));
    let cam_y =
        (pcy as i32 - map.height as i32 / 2).clamp(0, (WORLD_H - map.height as i32).max(0));
    game.camera = (cam_x, cam_y);

    // ---- lighting precomputation ------------------------------------------
    let day = game.daylight();
    let sky_bg = lerp(SKY_NIGHT, SKY_DAY, (day - 0.15) / 0.85);

    let w = map.width as i32;
    let h = map.height as i32;
    let mut surf: Vec<i32> = Vec::with_capacity(w as usize);
    for x in cam_x..cam_x + w {
        surf.push(game.world.surface_at(x));
    }
    let mut torches: Vec<(i32, i32)> = Vec::new();
    let margin = TORCH_RADIUS as i32 + 1;
    for y in (cam_y - margin).max(0)..(cam_y + h + margin).min(WORLD_H) {
        for x in (cam_x - margin).max(0)..(cam_x + w + margin).min(WORLD_W) {
            if game.world.get(x, y) == Block::Torch {
                torches.push((x, y));
            }
        }
    }

    let light_at = |x: i32, y: i32| -> f32 {
        let s = surf
            .get((x - cam_x) as usize)
            .copied()
            .unwrap_or(WORLD_H);
        let sky = if y <= s {
            day
        } else {
            let depth = (y - s) as f32;
            day * (1.0 - (depth / 9.0).min(1.0))
        };
        let mut l = sky;
        for &(tx, ty) in &torches {
            let dx = (tx - x) as f32;
            let dy = (ty - y) as f32;
            let d = (dx * dx + dy * dy).sqrt();
            if d < TORCH_RADIUS {
                l = l.max(1.0 - d / TORCH_RADIUS);
            }
        }
        // faint glow around the player so caves are never pitch black
        let dx = pcx - x as f32;
        let dy = pcy - y as f32;
        let d = (dx * dx + dy * dy).sqrt();
        if d < 5.0 {
            l = l.max(0.4 * (1.0 - d / 5.0) + 0.1);
        }
        l.clamp(MIN_LIGHT, 1.0)
    };

    // ---- tiles --------------------------------------------------------------
    let buf = f.buffer_mut();
    for sy in 0..map.height {
        for sx in 0..map.width {
            let wx = cam_x + sx as i32;
            let wy = cam_y + sy as i32;
            let b = game.world.get(wx, wy);
            let l = light_at(wx, wy);
            let s = surf.get(sx as usize).copied().unwrap_or(WORLD_H);
            let air_bg = if wy <= s { sky_bg } else { CAVE_BG };

            let (mut glyph, fg, bg) = match b {
                Block::Air => (' ', (0, 0, 0), air_bg),
                Block::Torch => (b.glyph(), b.colors().0, air_bg),
                _ => {
                    let (fg, bg) = b.colors();
                    (b.glyph(), fg, bg.unwrap_or(air_bg))
                }
            };
            // gentle water shimmer
            if b == Block::Water && (wx + (game.time / 5) as i32) % 5 == 0 {
                glyph = '≈';
            }
            let (fg_c, bg_c) = if b == Block::Torch {
                // torches glow at full brightness
                (scale(fg, 1.0), scale(bg, l.max(0.6)))
            } else {
                (scale(fg, l), scale(bg, l))
            };
            if let Some(cell) = buf.cell_mut((map.x + sx, map.y + sy)) {
                cell.set_char(glyph).set_fg(fg_c).set_bg(bg_c);
            }
        }
    }

    // ---- entities -------------------------------------------------------------
    let draw_entity = |buf: &mut ratatui::buffer::Buffer,
                           e: &crate::entity::Entity,
                           head: char,
                           body: char,
                           color: Rgb| {
        let tx = (e.x + e.w / 2.0).floor() as i32;
        let head_y = e.y.floor() as i32;
        let body_y = (e.y + 1.0).floor() as i32;
        for (ty, ch) in [(head_y, head), (body_y, body)] {
            if tx >= cam_x && tx < cam_x + w && ty >= cam_y && ty < cam_y + h {
                let l = light_at(tx, ty).max(0.35);
                if let Some(cell) =
                    buf.cell_mut(((map.x as i32 + tx - cam_x) as u16, (map.y as i32 + ty - cam_y) as u16))
                {
                    cell.set_char(ch).set_fg(scale(color, l));
                }
            }
        }
    };
    let zombies = game.zombies.clone();
    for z in &zombies {
        draw_entity(buf, z, 'Z', '╨', (95, 200, 95));
    }
    draw_entity(buf, &game.player, '@', '╨', (255, 230, 170));

    // ---- target cursor -----------------------------------------------------------
    let (cx, cy) = game.cursor;
    if cx >= cam_x && cx < cam_x + w && cy >= cam_y && cy < cam_y + h {
        let ok = game.cursor_in_reach();
        let tint: Rgb = if ok { (255, 255, 140) } else { (255, 90, 90) };
        if let Some(cell) =
            buf.cell_mut(((map.x as i32 + cx - cam_x) as u16, (map.y as i32 + cy - cam_y) as u16))
        {
            let cur_bg = match cell.bg {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (0, 0, 0),
            };
            cell.set_bg(scale(blend(cur_bg, tint, 0.45), 1.0));
            if cell.symbol() == " " {
                cell.set_char('┼').set_fg(scale(tint, 0.9));
            }
        }
    }

    // ---- HUD ------------------------------------------------------------------------
    draw_hud(f, game, area, hud_h);

    if game.crafting_open {
        draw_crafting(f, game, area);
    }
    if game.help_open {
        crate::render3::draw_help(
            f,
            area,
            &[
                ("Movement", ""),
                ("  a / d", "move left / right"),
                ("  w / space", "jump (swim up in water)"),
                ("Actions", ""),
                ("  arrow keys", "aim the target cursor"),
                ("  x / Enter / left-click", "mine block / attack zombie"),
                ("  z / right-click", "place selected block"),
                ("  1-9", "select hotbar slot"),
                ("  c", "crafting menu"),
                ("Game", ""),
                ("  F5 / Ctrl+S", "save"),
                ("  h / ?", "toggle this help"),
                ("  q / Esc", "quit (autosaves)"),
                ("", ""),
                ("Tip", "zombies come out at night - build shelter and torches!"),
            ],
        );
    }
    if game.game_over {
        draw_game_over(f, area);
    }
}

fn draw_hud(f: &mut Frame, game: &Game, area: Rect, hud_h: u16) {
    let y0 = area.y + area.height - hud_h;

    // Line 1: hearts | time | position | message
    let full = (game.player.hp.max(0) / 2) as usize;
    let empty = 10usize.saturating_sub(full);
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
                if game.is_night() { "☾" } else { "☀" },
                game.day_number()
            ),
            Style::default().fg(if game.is_night() {
                Color::Rgb(170, 180, 255)
            } else {
                Color::Rgb(255, 220, 100)
            }),
        ),
        Span::raw(format!(
            "  x:{} y:{}",
            game.player.x as i32, game.player.y as i32
        )),
    ];
    if let Some((m, _)) = &game.msg {
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

    // Line 2: hotbar
    let mut spans: Vec<Span> = Vec::new();
    for (i, slot) in game.hotbar.iter().enumerate() {
        let sel = i == game.selected;
        let (bracket_style, inner): (Style, Vec<Span>) = match slot {
            Some(b) => {
                let n = game.count(*b);
                let fg = b.colors().0;
                let mut st = Style::default().fg(Color::Rgb(fg.0, fg.1, fg.2));
                if n == 0 {
                    st = Style::default().fg(Color::DarkGray);
                }
                (
                    if sel {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(110, 110, 110))
                    },
                    vec![Span::styled(format!("{}{:<3}", b.glyph(), n.min(999)), st)],
                )
            }
            None => (
                if sel {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Rgb(110, 110, 110))
                },
                vec![Span::styled("    ", Style::default())],
            ),
        };
        spans.push(Span::styled(format!("{}", i + 1), bracket_style));
        spans.push(Span::styled("[", bracket_style));
        spans.extend(inner);
        spans.push(Span::styled("] ", bracket_style));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(20, 20, 24))),
        Rect::new(area.x, y0 + 1, area.width, 1),
    );

    // Line 3: help
    let help = "h help  a/d move  w/space jump  ←↑→↓ aim  x mine  z place  c craft  q quit";
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

fn draw_crafting(f: &mut Frame, game: &Game, area: Rect) {
    let inv_lines = 2u16;
    let rect = centered(area, 56, RECIPES.len() as u16 + inv_lines + 4);
    f.render_widget(Clear, rect);

    let mut lines: Vec<Line> = Vec::new();
    for (i, r) in RECIPES.iter().enumerate() {
        let sel = i == game.craft_sel;
        let craftable = game.can_craft(i);
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
    let inv_str = if game.inv.is_empty() {
        "Inventory: (empty - go mine something!)".to_string()
    } else {
        let items: Vec<String> = game
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
