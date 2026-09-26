# SPEC-003: Runtime & rendering

**Crates**: `crates/sciart` (runtime), `crates/sciart-render` · **Milestones**: M1 (minimal), M2 (full) · **Status**: M2 implemented (RT-R8 is M3)

## Runtime (pure state machine, no terminal access)
- **RT-R1** `Runtime` owns the active generator list, the current index, a `Canvas`, a `Ctx`, and a `FixedStep`. It is driven only by `tick(elapsed)`, `handle(Action)` and `resize(Size)` — fully unit-testable.
- **RT-R2** `tick` runs the generator's `update` for the steps returned by `FixedStep`, then `render`s into the canvas. Both calls are wrapped in `std::panic::catch_unwind`.
- **RT-R3** On panic: the generator is added to the quarantine set, the canvas keeps the last good frame, and a `Notice` is recorded. In `sim` mode the runtime switches to the next healthy generator.
- **RT-R4** If every generator is quarantined, `Runtime::status()` is `AllFailed`; `tick` becomes a no-op; the app keeps running until quit.
- **RT-R5** (M2) With `with_interval(secs)`, auto-cycle once `secs` of unpaused *wall* time have elapsed since the last switch, to a random healthy generator ≠ current. With fewer than two healthy generators there is no auto-cycle. Every switch (auto, `Next`, `Prev`, after a panic) restarts the timer; `Reset` does too.
- **RT-R5a** (M2) History: each newly shown generator is pushed (bounded to 64). `Prev` returns to the most recent earlier, non-quarantined entry in history; with none it is a no-op.
- **RT-R6** (M2) `handle(Action) -> Flow`. Actions: `Next`, `Prev`, `TogglePause`, `Reset`, `ToggleHud`, `CycleTheme`, `Quit` (⇒ `Flow::Quit`, all others `Flow::Continue`). Pause freezes simulation and the auto-cycle timer; `Next`/`Prev`/`Reset` work while paused. The runtime owns view state (`hud()`, `theme()`, `paused()`). With `AllFailed`, generator actions are no-ops.
- **RT-R7** Switching generator (or `Reset`) builds a fresh instance, calls `resize` before its first `update`, and renders one frame immediately so the screen never goes blank (also while paused).
- **RT-R9** (M2) `keys::action_for(KeyEvent) -> Option<Action>` maps only key *presses*: `n`/`→` Next, `p`/`←` Prev, `space` TogglePause, `r` Reset, `h` ToggleHud, `t` CycleTheme, `q`/`Esc`/`Ctrl-C` Quit; letters are case-insensitive; everything else is ignored.
- **RT-R10** (M2) `FpsMeter::record(frame_secs)` keeps an exponential moving average (α = 0.1) of `1/frame_secs`; the first sample seeds it; non-positive or non-finite samples are ignored; `fps()` is 0 before any sample.
- **RT-R8** (M3) Low-power: target FPS drops to `low_power.fps` after `idle_after` without input or on `FocusLost`; restores on any key or `FocusGained`.

## Rendering (`sciart-render`)
- **RND-R1** `Theme::color(ink, x, y) -> Rgb` is a pure function (no RNG, no time): same inputs ⇒ same color, so frames never strobe.
- **RND-R2** Built-in themes: `astro` (default, role-based palette), `rainbow` (hue hashed from `(x, y)` + level as value), `mono` (grey ramp by level). `Theme::from_name` parses case-insensitively.
- **RND-R3** `paint(canvas, theme, buf: &mut ratatui::buffer::Buffer)` writes every canvas cell into the buffer area; `Role::Background` cells are written as a space with default colors.
- **RND-R4** (M2) `Hud { generator, fps, theme, seed, paused, quarantined }` renders as a ratatui widget on the **last row** of the area (reversed style), clipped to the width, touching no other row; the canvas size is unchanged. Text: `<name> | <fps> fps | <theme> | seed <n>` then ` | PAUSED` and ` | <k> quarantined` when applicable, then ` | n p space r h t q`. ASCII only.
- **RND-R5** `TerminalGuard` enters raw mode + alt screen + hidden cursor, restores on `Drop`, and installs a panic hook that restores the terminal before the default hook prints.
