mod block;
mod entity;
mod game;
mod game3;
mod render;
mod render3;
mod world;
mod world3;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use game::Game;
use game3::Game3;

const TICK: Duration = Duration::from_millis(50); // 20 TPS

const HELP: &str = "termcraft - a Minecraft-inspired sandbox for your terminal

USAGE:
  termcraft [OPTIONS]

OPTIONS:
  --2d          Play the classic 2D side-view mode
  --new         Start a fresh world (ignores the saved one)
  --seed <N>    World seed for a fresh world
  --help        Show this help
  --version     Show version

The default mode is first-person 3D. Worlds autosave to
~/.termcraft/save3d.json (3D) and ~/.termcraft/save.json (2D) on quit.

3D CONTROLS:
  w / a / s / d  move (relative to where you're looking)
  arrow keys     look around (or drag the mouse)
  space          jump (swim up in water)
  x / Enter      mine the block under the crosshair (or left-click)
  z              place selected block against the targeted face
  1-9            select hotbar slot
  c              crafting menu
  F5 / Ctrl+S    save
  q / Esc        quit

2D CONTROLS (--2d):
  a / d          move left / right
  w / space      jump (swim up in water)
  arrow keys     aim the target cursor
  x / Enter      mine block / attack zombie (or left-click)
  z              place selected block (or right-click)";

fn restore_terminal() {
    let _ = execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen,
        DisableMouseCapture
    );
    let _ = disable_raw_mode();
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut force_new = false;
    let mut mode_2d = false;
    let mut seed: Option<u64> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(());
            }
            "--version" | "-V" => {
                println!("termcraft {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--2d" => mode_2d = true,
            "--new" => force_new = true,
            "--seed" => {
                i += 1;
                let v = args.get(i).and_then(|s| s.parse::<u64>().ok());
                match v {
                    Some(v) => seed = Some(v),
                    None => {
                        eprintln!("error: --seed requires a number");
                        std::process::exit(2);
                    }
                }
                force_new = true;
            }
            other => {
                eprintln!("error: unknown option '{other}' (try --help)");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    // Always restore the terminal, even if we panic mid-frame.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    if mode_2d {
        let mut game = if force_new {
            Game::new(seed.unwrap_or_else(default_seed))
        } else {
            Game::load().unwrap_or_else(|| Game::new(default_seed()))
        };
        let (mut terminal, hold_keys) = setup_terminal()?;
        game.set_hold_mode(hold_keys);
        let result = run(&mut terminal, &mut game);
        restore_terminal();
        match game.save() {
            Ok(()) => println!("World saved to {}", game::save_path().display()),
            Err(e) => eprintln!("Failed to save world: {e}"),
        }
        println!("Thanks for playing TermCraft!");
        result
    } else {
        let mut game = if force_new {
            Game3::new(seed.unwrap_or_else(default_seed))
        } else {
            Game3::load().unwrap_or_else(|| Game3::new(default_seed()))
        };
        let (mut terminal, hold_keys) = setup_terminal()?;
        game.set_hold_mode(hold_keys);
        let result = run3(&mut terminal, &mut game);
        restore_terminal();
        match game.save() {
            Ok(()) => println!("World saved to {}", game3::save3_path().display()),
            Err(e) => eprintln!("Failed to save world: {e}"),
        }
        println!("Thanks for playing TermCraft!");
        result
    }
}

/// Returns the terminal plus whether key release events are available
/// (kitty keyboard protocol), enabling true hold-to-move.
fn setup_terminal() -> io::Result<(Terminal<CrosstermBackend<io::Stdout>>, bool)> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
        )?;
    }
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok((terminal, enhanced))
}

fn default_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(42)
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    game: &mut Game,
) -> io::Result<()> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|f| render::draw(f, game))?;

        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            // Drain everything that's queued so multi-key input stays responsive.
            loop {
                match event::read()? {
                    Event::Key(k) => game.on_key(k),
                    Event::Mouse(m) => game.on_mouse(m),
                    _ => {}
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }

        if last_tick.elapsed() >= TICK {
            game.tick();
            last_tick = Instant::now();
        }

        if game.should_quit {
            return Ok(());
        }
    }
}

fn run3(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    game: &mut Game3,
) -> io::Result<()> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|f| render3::draw(f, game))?;

        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            loop {
                match event::read()? {
                    Event::Key(k) => game.on_key(k),
                    Event::Mouse(m) => game.on_mouse(m),
                    _ => {}
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }

        if last_tick.elapsed() >= TICK {
            game.tick();
            last_tick = Instant::now();
        }

        if game.should_quit {
            return Ok(());
        }
    }
}
