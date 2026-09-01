# Technical specification: settings panel foundation (#5)

## Approach

Add transient settings-panel state to both game models, route input through the same modal-first pattern used by crafting and help, and render a shared settings overlay from both 2D and 3D render paths. Keep the initial settings catalog declarative and non-functional so later work can attach values and actions without changing the shell.

No setting value, persistence schema, or network message is introduced in this stage.

## Affected components

### `src/main.rs`

- Register any new shared settings module.
- Add `o` to the 2D and 3D control text in the built-in help constant.
- Do not add a command-line option.

### `src/settings.rs` (new)

- Define a small shared settings catalog used by both modes and the renderer.
- Give every row a stable identifier, such as `TimeOfDay`, `Monsters`, and `CreativeMode`, plus its display label.
- Keep row identity separate from label and array position so later implementations can route behavior safely.
- Expose the three initial rows in their displayed order.
- Do not define mutable setting values, defaults, persistence, or mode-specific behavior yet.

### `src/game.rs`

- Add transient state for whether settings are open and which row is selected.
- Initialize settings as closed with the first row selected in both `Game::new` and `Game::load`.
- In `Game::on_key`, preserve key-release handling, then handle existing higher-priority states and settings before normal gameplay input:
  - when settings are open, close on `Esc`, `q`, or `o`;
  - move selection with wrapping on `↑`/`w` and `↓`/`s`;
  - consume all other presses without mutating the game;
  - from normal gameplay, open settings on `o` and reset selection to the first row.
- Include the settings-open state in the early return in `Game::on_mouse`.
- Do not change `SaveData`; panel state is session-only.

### `src/game3.rs`

- Add the same transient open/selection state and initialize it in both `Game3::from_world` and loaded games through that constructor.
- Route settings input after chat, game-over, crafting, and help handling but before normal gameplay keys.
- Preserve global key-release handling before modal handling so existing held-key state can be released while the panel is open.
- Include settings in the `Game3::on_mouse` modal guard.
- Continue calling `Game3::tick` and `net_sync`; the panel is not a pause mechanism.
- Do not change `Save3`, `Net`, `NetEvent`, or the multiplayer wire protocol.

### `src/render.rs`

- Draw the settings overlay when the 2D settings-open state is active.
- Add the `o settings` hint to the 2D HUD.
- Reuse the shared settings renderer rather than creating a visually divergent 2D implementation.

### `src/render3.rs`

- Add a reusable settings-overlay renderer that accepts the selected row and shared catalog.
- Build the overlay with the existing `centered`, `Clear`, `Block`, border, dark background, and highlighted-row patterns.
- Render the title, section content, `Coming later` value text, selection marker, and footer defined in `product.md`.
- Draw it from the 3D path when settings are open.
- Add `o settings` to each applicable 3D HUD hint variant.
- Keep overlay ordering deterministic. Settings are mutually exclusive with chat, player list, crafting, help, and game over through input routing; the renderer must not rely on stacked modals for normal operation.

### `README.md`

- Add the `o` settings-panel control for both modes.
- Describe it as a non-functional settings preview in this stage so users do not expect the listed options to change gameplay.

## State and API shape

The exact Rust types are an implementation choice, but they must preserve these properties:

- Panel visibility and selection are transient UI state.
- The selection is always a valid index into a non-empty shared catalog.
- Opening resets selection to index zero.
- Navigation uses catalog length and wraps in both directions without underflow.
- Rows have stable identities so adding or reordering labels later does not define behavior by index.
- Rendering reads catalog metadata and selection but cannot mutate game state.
- No public file format or network API changes.

A small shared helper may encapsulate open/close/navigation behavior if that avoids duplicating wraparound logic between `Game` and `Game3`. It must not couple the two distinct game-state structs.

## Input precedence and edge cases

- Key releases remain first so movement holds can be cleared regardless of the open overlay.
- In 3D, chat retains precedence; typing `o`, `w`, `s`, or `q` into chat must not open or navigate settings.
- Game-over controls retain precedence; settings cannot open from the death screen.
- Existing crafting and help controls retain precedence; pressing `o` there must not stack settings on top.
- The 3D player list is toggled from normal gameplay. Opening settings while the player list is visible should close the player list before showing settings, ensuring a single primary overlay.
- Settings navigation consumes `w` and `s` before movement handling.
- `Ctrl+S` must not save while settings are open because all non-settings key presses are consumed.
- Mouse events are ignored while settings are open.
- Preferred overlay dimensions must be clamped to `Frame::area()`. Width calculations use character counts and saturating/clamped arithmetic to avoid terminal-size underflow.
- Resizing while the panel is open must not invalidate selection or panic.

## Compatibility and data impact

- Existing save files remain readable because settings UI state is not serialized.
- Save output remains byte-structure-compatible with the current `SaveData` and `Save3` schemas.
- Multiplayer peers do not observe panel state, and no authority decision is needed.
- Time, monsters, creative state, and all other gameplay continue to use their current fields and logic.
- The change adds no dependency.

## Testing strategy

### Game-state tests

Add parallel tests in `src/game.rs` and `src/game3.rs` covering:

- `o` opens settings from live gameplay and selects the first row.
- `↑`/`w` and `↓`/`s` navigate and wrap.
- `Esc`, `q`, and `o` close settings without setting `should_quit`.
- Reopening resets selection.
- Mining/placing, save, movement, help, crafting, and chat keys do not trigger their gameplay action while settings are open.
- Mouse events do not trigger gameplay actions while settings are open.
- In 3D, chat input containing settings keys remains chat input.
- Key-release events still clear held movement state while settings are open.
- No preview row changes time, entities, creative state, save ownership, or networking state.

### Rendering tests

Use Ratatui `TestBackend` buffers to verify:

- Both 2D and 3D render paths show the `Settings` title, all three labels, `Coming later`, the selected-row marker, and footer.
- Changing selection moves the highlighted marker.
- The overlay renders at 80×24 and representative constrained dimensions without panic.
- Opening settings does not render crafting/help/player-list content over the settings panel.

Prefer semantic buffer assertions for text and selection over snapshots tied to unrelated world pixels.

### Manual validation

- Launch default 3D and `--2d` modes in an 80×24 or larger truecolor terminal.
- Verify open, wraparound navigation, close behavior, visual consistency, and suppression of gameplay input.
- Repeat in a constrained terminal and resize while open.
- In a 3D multiplayer session, verify the session continues syncing while the panel is open and that panel state is local only.

## Implementation boundaries

- Do not implement setting values or actions in the shared row identifiers.
- Do not modify save structs, migration behavior, networking messages, world generation, monster logic, day/night calculations, or creative-mode logic.
- Do not refactor unrelated modal state as a prerequisite.
- Do not introduce a general settings framework beyond what the three-row shell needs.
