# TermCraft

A Minecraft-inspired sandbox game that runs entirely in your terminal — in first-person 3D. Built in Rust with [ratatui](https://ratatui.rs); the 3D view is a software voxel raycaster drawn with half-block characters and truecolor.

## Features

### 3D mode (default)
- First-person voxel rendering: per-pixel DDA raycasting, per-face shading, distance fog, sky gradient
- Procedurally generated 128×64×128 voxel world: hills, oceans, beaches, 3D cave systems, coal & iron ore, trees, bedrock
- Mine the block under the crosshair, place blocks against the targeted face
- Real lighting: dark caves, torch glow, day/night cycle with dusk/dawn sky
- Gravity, jumping, swimming, fall damage, health regen
- Crafting: planks, torches, stone bricks
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
termcraft --seed 42  # fresh world with a specific seed
termcraft --2d       # classic 2D side-view mode
```

## 3D Controls

- `w` / `a` / `s` / `d` — move (relative to where you're looking)
- arrow keys — look around (or drag the mouse)
- `space` — jump (swim up in water)
- `x` / `Enter` / left-click — mine the block under the crosshair
- `z` / right-click — place selected block against the targeted face
- `1`–`9` — select hotbar slot
- `c` — crafting menu
- `F5` / `Ctrl+S` — save
- `q` / `Esc` — quit (autosaves)

In 2D mode the arrow keys aim a target cursor instead, and `a`/`d` move while `w` jumps.

Tips: a wider terminal means a higher-resolution 3D view. Use a truecolor-capable terminal. Mine wood from trees, craft planks, then torches (coal + planks) before nightfall — caves are pitch black without them.
