# TermCraft

> [!NOTE]
> ❤️ Built with [warp.dev](https://warp.dev)

A Minecraft-inspired sandbox game that runs entirely in your terminal — in first-person 3D. Built in Rust with [ratatui](https://ratatui.rs); the 3D view is a software voxel raycaster drawn with half-block characters and truecolor.

## Features

### 3D mode (default)
- First-person voxel rendering: per-pixel DDA raycasting, per-face shading, distance fog, sky gradient
- Procedurally generated 128×64×128 voxel world: hills, oceans, beaches, 3D cave systems, coal & iron ore, trees, bedrock
- Mine the block under the crosshair, place blocks against the targeted face
- Real lighting: dark caves, torch glow, day/night cycle with dusk/dawn sky
- Gravity, jumping, swimming, fall damage, health regen
- Crafting: planks, torches, stone bricks
- Multiplayer: two terminals on the same seed play in the same live world
- Autosaves to `~/.termcraft/save3d.json` on quit

### 2D mode (`--2d`)
- The classic side-view 600×160 world with zombies at night, flowing water, and mouse-aim mining
- Autosaves to `~/.termcraft/save.json`

## Install

**Download a prebuilt binary** (no Rust needed) from the
[latest release](https://github.com/vikvang/termcraft/releases/latest) —
available for macOS (Apple Silicon & Intel), Linux (x86_64 & arm64), and Windows.

```sh
# macOS / Linux example:
tar -xzf termcraft-*.tar.gz
sudo mv termcraft /usr/local/bin/
termcraft
```

On macOS you may need to clear the quarantine flag the first time:
`xattr -d com.apple.quarantine ./termcraft`

**Or install with cargo** (Rust required):

```sh
cargo install termcraft-3d          # from crates.io (binary is `termcraft`)
# or:
cargo install --git https://github.com/vikvang/termcraft
# or from a local checkout:
cargo install --path .
```

## Play

```
termcraft            # 3D, resume saved world (or create one)
termcraft --new      # fresh 3D world
termcraft --seed 42  # shared world with a specific seed (multiplayer)
termcraft --creative # 3D creative mode: always flying, no gravity or fall damage
termcraft --2d       # classic 2D side-view mode
```

`--creative` only affects 3D (ignored with `--2d`). It pairs with `--new`, `--seed`, and multiplayer; inventory, mining, and HP stay the same, and you still collide with blocks.

## Multiplayer

Run the same seed in two terminals and you're in the same world:

```sh
termcraft --seed 7           # terminal 1 - hosts the world
termcraft --seed 7           # terminal 2 - joins it
termcraft --seed 7 --name jo # pick the name others see
```

The first process to claim the seed's port hosts the world and is the
authority; everyone else downloads the host's copy on join, so all edits line
up. You'll see each other's avatars (with floating nametags and health), every
mined or placed block is shared instantly, and the day/night clock is synced.

- `t` — chat with everyone in the world
- `Tab` — player list with health and distance
- `x` aimed at a player — punch them

Shared-seed worlds persist in `~/.termcraft/world-<seed>.json`. Only the host
writes that file; guests never touch it. Add `--new` to start the shared world
from scratch, or `--solo` to ignore multiplayer entirely.

Across a LAN, the host adds `--open` and everyone else joins with
`termcraft --seed 7 --join <host-ip>`. Multiplayer is 3D-only.

## 3D Controls

- `w` / `a` / `s` / `d` — move (relative to where you're looking; in creative, fly including pitch)
- arrow keys — look around (or drag the mouse)
- `space` — jump (swim up in water); in creative, rise
- `f` — descend (creative only; hold to keep descending)
- `x` / `Enter` / left-click — mine the block under the crosshair (or punch the player you're aiming at)
- `z` / right-click — place selected block against the targeted face
- `1`–`9` — select hotbar slot
- `c` — crafting menu
- `t` — chat (multiplayer)
- `Tab` — player list
- `F5` / `Ctrl+S` — save
- `q` / `Esc` — quit (autosaves)

In 2D mode the arrow keys aim a target cursor instead, and `a`/`d` move while `w` jumps.

Tips: a wider terminal means a higher-resolution 3D view. Use a truecolor-capable terminal. Mine wood from trees, craft planks, then torches (coal + planks) before nightfall — caves are pitch black without them.
