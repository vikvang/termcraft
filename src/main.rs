mod block;
mod entity;
mod game;
mod game3;
mod net;
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
use net::{Net, NetOpts};
use world3::{D3, H3, W3};

const TICK: Duration = Duration::from_millis(50); // 20 TPS

const HELP: &str = "termcraft - a Minecraft-inspired sandbox for your terminal

USAGE:
  termcraft [OPTIONS]

OPTIONS:
  --2d           Play the classic 2D side-view mode
  --creative     3D creative mode: always fly, no gravity or fall damage
  --new          Start a fresh world (ignores the saved one)
  --seed <N>     Shared world seed - see MULTIPLAYER below
  --name <NAME>  Name other players see (default: $USER)
  --solo         Stay single player even with --seed
  --open         Let other machines on your LAN join this world
  --join <ADDR>  Join a world on another machine (host[:port])
  --help         Show this help
  --version      Show version

The default mode is first-person 3D. Worlds autosave to
~/.termcraft/save3d.json (3D), ~/.termcraft/world-<seed>.json (shared
seeds) and ~/.termcraft/save.json (2D) on quit.

MULTIPLAYER:
  Run `termcraft --seed 7` in two terminals and you're in the same world:
  the first one hosts it, the second joins and gets the host's copy. You
  see each other's avatars, share every block that gets mined or placed,
  chat with `t`, list players with Tab, and can punch each other with `x`.
  Only the host writes the save file. `--open` plus `--join <ip>` does the
  same across a LAN.

3D CONTROLS:
  w / a / s / d  move (relative to where you're looking)
  arrow keys     look around (or drag the mouse)
  space          jump (swim up in water); rise in --creative
  f              descend (creative mode only)
  x / Enter      mine the block under the crosshair (or left-click),
                 or punch the player you're aiming at
  z              place selected block against the targeted face
  1-9            select hotbar slot
  c              crafting menu
  t              chat (multiplayer)
  Tab            player list
  F5 / Ctrl+S    save
  q / Esc        quit

CREATIVE (--creative, 3D only):
  Always flying: no gravity, no fall damage, still collides with blocks.
  w/a/s/d fly relative to look (including pitch). space rises, f descends.

2D CONTROLS (--2d):
  a / d          move left / right
  w / space      jump (swim up in water)
  arrow keys     aim the target cursor
  x / Enter      mine block / attack zombie (or left-click)
  z              place selected block (or right-click)";

/// Reads the value that follows a flag, or exits with a usage error.
fn need_value(args: &[String], i: &mut usize, flag: &str) -> String {
    *i += 1;
    match args.get(*i) {
        Some(v) => v.clone(),
        None => {
            eprintln!("error: {flag} requires a value");
            std::process::exit(2);
        }
    }
}

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
    let mut creative = false;
    let mut seed: Option<u64> = None;
    let mut solo = false;
    let mut net_opts = NetOpts::default();
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
            "--creative" => creative = true,
            "--new" => force_new = true,
            "--solo" => solo = true,
            "--open" => net_opts.open = true,
            "--name" => net_opts.name = need_value(&args, &mut i, "--name"),
            "--join" => net_opts.join = Some(need_value(&args, &mut i, "--join")),
            "--seed" => {
                let raw = need_value(&args, &mut i, "--seed");
                match raw.parse::<u64>() {
                    Ok(v) => seed = Some(v),
                    Err(_) => {
                        eprintln!("error: --seed requires a number");
                        std::process::exit(2);
                    }
                }
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
        if net_opts.join.is_some() && seed.is_none() {
            eprintln!("error: --join also needs --seed <N> (the world you're joining)");
            std::process::exit(2);
        }
        // An explicit seed means "share this world"; --solo opts out.
        let shared = if solo { None } else { seed };
        let mut game = match shared {
            Some(seed) => match start_multiplayer(seed, &net_opts, force_new) {
                Ok(game) => game,
                Err(e) => {
                    eprintln!("Multiplayer unavailable ({e}); starting single player.");
                    Game3::new(seed)
                }
            },
            None => match seed {
                Some(seed) => Game3::new(seed),
                None if force_new => Game3::new(default_seed()),
                None => Game3::load().unwrap_or_else(|| Game3::new(default_seed())),
            },
        };
        if creative {
            game.set_creative(true);
        }
        let (mut terminal, hold_keys) = setup_terminal()?;
        game.set_hold_mode(hold_keys);
        let result = run3(&mut terminal, &mut game);
        restore_terminal();
        if game.owns_save() {
            match game.save() {
                Ok(()) => println!("World saved to {}", game.save_file().display()),
                Err(e) => eprintln!("Failed to save world: {e}"),
            }
        } else {
            println!("The host keeps this world - nothing saved locally.");
        }
        println!("Thanks for playing TermCraft!");
        result
    }
}

/// Hosts or joins the session for `seed` and returns a game bound to it.
///
/// Whoever gets there first hosts (and owns the save file); everyone else
/// downloads that host's world so all edits line up.
fn start_multiplayer(seed: u64, opts: &NetOpts, force_new: bool) -> Result<Game3, String> {
    let mut net = Net::start(seed, opts).map_err(|e| e.to_string())?;
    let path = game3::seed_save_path(seed);
    let mut game = if net.is_authority() {
        // Shared seeds are sticky: pick up where the world left off.
        let saved = if force_new {
            None
        } else {
            Game3::load_from(&path)
        };
        saved.unwrap_or_else(|| Game3::new(seed))
    } else {
        let (bytes, spawn, time) = net.await_world((W3 * H3 * D3) as usize)?;
        let world = world3::World3::from_bytes(&bytes, spawn)
            .ok_or_else(|| "the host sent a world we can't read".to_string())?;
        let mut game = Game3::from_world(seed, world);
        game.time = time;
        game
    };
    let owns = net.is_authority();
    game.set_save(path, owns);
    game.attach_net(net);
    Ok(game)
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

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, game: &mut Game) -> io::Result<()> {
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

fn run3(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, game: &mut Game3) -> io::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn opts(name: &str) -> NetOpts {
        NetOpts {
            name: name.to_string(),
            join: None,
            open: false,
        }
    }

    /// Full stack: two games start on the same seed, one hosts, the other
    /// downloads that world, and edits and positions flow between them.
    #[test]
    fn two_terminals_on_one_seed_share_a_world() {
        let seed = 0xD00D_1234_5678_9ABC;
        let mut host = start_multiplayer(seed, &opts("host"), true).expect("hosts the seed");
        assert!(host.owns_save(), "the host owns the world file");

        // The guest blocks on the world snapshot, which only arrives while the
        // host is ticking, so join from another thread.
        let joining = std::thread::spawn(move || start_multiplayer(seed, &opts("guest"), true));
        let mut guest = loop {
            host.tick();
            if joining.is_finished() {
                break joining.join().unwrap().expect("guest joins");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(!guest.owns_save(), "guests must not own the save file");

        // Same world: spot-check a column of blocks.
        for y in 0..H3 {
            assert_eq!(
                host.world.get(W3 / 2, y, D3 / 2),
                guest.world.get(W3 / 2, y, D3 / 2),
                "worlds diverge at y={y}"
            );
        }

        // Let both settle and exchange positions.
        for _ in 0..40 {
            host.tick();
            guest.tick();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(host.peers.len(), 1, "host should see the guest");
        assert_eq!(guest.peers.len(), 1, "guest should see the host");

        // Step the guest a few blocks clear of the host, otherwise looking
        // down aims at the host's avatar (which is a punch, not a dig).
        let (gx, gz) = (guest.px + 4.0, guest.pz);
        let ground = guest.world.height_at(gx as i32, gz as i32);
        guest.px = gx;
        guest.pz = gz;
        guest.py = ground as f32 + 1.0;
        for _ in 0..20 {
            host.tick();
            guest.tick();
        }

        // The guest mines a block; the host's world must follow.
        guest.pitch = -1.4;
        assert!(
            guest.peer_in_crosshair().is_none(),
            "expected clear line of sight to the ground"
        );
        let dug = guest.target().expect("ground under the guest");
        guest.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(guest.world.get(dug.x, dug.y, dug.z), block::Block::Air);
        let mut synced = false;
        for _ in 0..60 {
            host.tick();
            guest.tick();
            if host.world.get(dug.x, dug.y, dug.z) == block::Block::Air {
                synced = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(synced, "the guest's dig never reached the host");

        // And chat travels the other way.
        host.chat_open = true;
        for c in "hello".chars() {
            host.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        host.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let mut heard = false;
        for _ in 0..60 {
            host.tick();
            guest.tick();
            if guest
                .recent_chat()
                .iter()
                .any(|(from, text, _)| from == "host" && text == "hello")
            {
                heard = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(heard, "the guest never received the host's chat line");
    }
}
