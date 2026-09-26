# SPEC-003: Runtime & rendering

**Crates**: `crates/sciart` (runtime), `crates/sciart-render` · **Milestones**: M1 (minimal), M2 (full) · **Status**: M1 partial

## Runtime (pure state machine, no terminal access)
- **RT-R1** `Runtime` owns the active generator list, the current index, a `Canvas`, a `Ctx`, and a `FixedStep`. It is driven only by `tick(elapsed)`, `handle(Action)` and `resize(Size)` — fully unit-testable.
- **RT-R2** `tick` runs the generator's `update` for the steps returned by `FixedStep`, then `render`s into the canvas. Both calls are wrapped in `std::panic::catch_unwind`.
- **RT-R3** On panic: the generator is added to the quarantine set, the canvas keeps the last good frame, and a `Notice` is recorded. In `sim` mode the runtime switches to the next healthy generator.
- **RT-R4** If every generator is quarantined, `Runtime::status()` is `AllFailed`; `tick` becomes a no-op; the app keeps running until quit.
- **RT-R5** (M2) Auto-cycle every `interval` of *wall* time; random healthy generator ≠ current (when more than one is healthy). History supports `Prev`.
- **RT-R6** (M2) Actions: `Next`, `Prev`, `TogglePause`, `Reset`, `ToggleHud`, `CycleTheme`, `Quit`. Pause freezes simulation and auto-cycle.
- **RT-R7** Switching generator (or `Reset`) builds a fresh instance and calls `resize` before its first `update`.
- **RT-R8** (M3) Low-power: target FPS drops to `low_power.fps` after `idle_after` without input or on `FocusLost`; restores on any key or `FocusGained`.

## Rendering (`sciart-render`)
- **RND-R1** `Theme::color(ink, x, y) -> Rgb` is a pure function (no RNG, no time): same inputs ⇒ same color, so frames never strobe.
- **RND-R2** Built-in themes: `astro` (default, role-based palette), `rainbow` (hue hashed from `(x, y)` + level as value), `mono` (grey ramp by level). `Theme::from_name` parses case-insensitively.
- **RND-R3** `paint(canvas, theme, buf: &mut ratatui::buffer::Buffer)` writes every canvas cell into the buffer area; `Role::Background` cells are written as a space with default colors.
- **RND-R4** (M2) HUD is drawn on the last row over the canvas when enabled; it never changes canvas size.
- **RND-R5** `TerminalGuard` enters raw mode + alt screen + hidden cursor, restores on `Drop`, and installs a panic hook that restores the terminal before the default hook prints.
