# Product specification: settings panel foundation (#5)

## Summary

TermCraft needs an in-game settings panel that can grow into the home for configurable gameplay options. This first stage delivers a polished, navigable settings page in both 2D and 3D modes, while deliberately leaving settings such as time of day, monsters, and creative mode non-functional for later work.

## Problem

There is no single place in the game where players can discover or eventually change gameplay settings. Adding each option independently would lead to fragmented controls and inconsistent interaction patterns between the 2D and 3D modes.

The first deliverable should establish the user experience and extension point before any setting changes game state.

## Goals

- Provide a clear in-game entry point for settings.
- Provide a consistent settings-page layout and keyboard navigation in both game modes.
- Preview the intended categories without implying that configuration already works.
- Establish a reusable shell so later changes can add values and actions without redesigning navigation.

## Current behavior

- Both game modes have centered modal overlays for help and crafting.
- `h`, `?`, or `F1` opens help; `c` opens crafting.
- Modal input is handled before normal gameplay input, preventing modal keystrokes from triggering game actions.
- Crafting uses `↑`/`↓` or `w`/`s` for wraparound selection and uses a highlighted row with a `>` prefix.
- Help and crafting use bordered, dark-background overlays that clear the underlying cells.
- Neither mode has a settings panel or settings-specific state.
- Time advances in both modes. Monster behavior currently exists in the 2D game, and creative mode currently exists only in 3D.

## Desired experience

### Entry and exit

- Pressing `o` or `O` during normal live gameplay opens the settings panel. `o` is recommended because it is unused in both modes and reads as “options.”
- Opening the panel selects its first settings row every time.
- Pressing `Esc`, `q`, `o`, or `O` while the panel is open closes it and returns to gameplay.
- Closing with `q` or `Esc` must not quit the game.
- The panel opens only from normal gameplay. It does not open while chat, crafting, help, or the game-over screen is handling input.
- If the 3D player list is visible, opening settings closes the player list first so only one primary overlay remains.

### Navigation

- `↑` and `w` move selection to the previous row.
- `↓` and `s` move selection to the next row.
- Navigation wraps from the first row to the last and from the last row to the first, matching crafting.
- All other key presses are consumed while the panel is open. In this stage, `Enter`, `Space`, and left/right keys do not change any value.
- Mouse input does not move, mine, place, look, or activate a setting while the panel is open. Mouse-based settings interaction is deferred.

### Layout and content

- The settings panel is a centered, bordered overlay titled `Settings`.
- It uses the same dark background, border treatment, selected-row emphasis, and terminal-native typography as the existing help and crafting overlays.
- The initial page contains these preview rows:
  - `Time of day`
  - `Monsters`
  - `Creative mode`
- Each row displays `Coming later` instead of a value or toggle. The wording and subdued styling must make the rows visibly non-functional.
- The selected row uses a `>` prefix, bold text, and highlighted background.
- A footer inside the panel states the available controls: `↑/↓ navigate · Esc close · controls coming later`.
- The panel clamps to the available terminal area and remains usable without panicking when the terminal is smaller than its preferred size. Content may be clipped only when the terminal cannot fit the minimum bordered layout.
- The normal game view remains visible around the overlay.

### Runtime behavior

- Opening the settings panel does not pause the simulation, matching existing help and crafting overlays.
- While the panel is open, navigation keys must not initiate new movement or other gameplay actions.
- Keyboard release events continue to update existing held-key tracking so a key held before opening does not remain stuck.
- No preview row changes time, monster behavior, creative mode, or any other game state.
- Opening, navigating, and closing the panel does not save or transmit data.

## Compatibility

- The panel is available in both default 3D mode and `--2d` mode.
- The shell and navigation are consistent across both modes.
- It is available in 3D survival, 3D creative, single-player, hosted multiplayer, and joined multiplayer sessions.
- Preview rows may describe capabilities that are mode-specific today, but they remain non-functional in every mode in this stage.
- Existing saves load unchanged, and saves written after this change remain in the existing format.

## Acceptance criteria

1. From normal gameplay in either 2D or 3D, `o` opens a centered settings panel with `Time of day`, `Monsters`, and `Creative mode` rows.
2. Every row is labeled `Coming later`; no row changes game state.
3. The first row is selected whenever the panel opens.
4. `↑`/`w` and `↓`/`s` move the selected row with wraparound behavior.
5. `Esc`, `q`, and `o` close the panel without quitting the game.
6. Gameplay keys and mouse events do not trigger movement, looking, mining, placing, crafting, chat, saving, or setting changes while the panel is open.
7. The game simulation continues while the panel is open, including the existing 3D multiplayer synchronization loop.
8. The overlay follows the existing TUI visual language and renders without panic at the normal 80×24 test size and at constrained terminal sizes.
9. Existing CLI behavior, controls outside the panel, save formats, world generation, game rules, and network protocol remain unchanged.
10. The in-game help/HUD control hints and `README.md` identify `o` as the settings-panel shortcut in both modes.

## Extension points for later settings

- Each visible row has a stable internal identity independent of its label and position.
- Later changes can replace `Coming later` with a mode-aware current value and add left/right or activation behavior without changing panel entry, exit, or row navigation.
- Later implementations can declare whether a setting applies to both modes or only one mode and can render unavailable choices without removing them from the page.
- Persistence and multiplayer authority rules are defined by the future setting that needs them; the shell itself owns neither.

## Out of scope

- Changing time of day.
- Enabling, disabling, spawning, or removing monsters.
- Toggling creative mode.
- Adding any other functional setting.
- Persisting settings to save files or a new configuration file.
- Synchronizing settings over multiplayer.
- Mouse interaction within the settings panel.
- Multiple settings pages, tabs, scrolling, search, remapping, reset-to-defaults, apply/cancel flows, or command-line settings.
- Pausing the world while settings are open.

## Open questions

- **Entry key:** this specification recommends `o` because it is currently unused. Maintainers may choose another unbound key during review.
- **Preview content:** this specification recommends showing the reporter’s three examples as inert rows so the shell demonstrates its future structure. Maintainers may prefer category-only placeholders, but functional controls remain out of scope either way.
