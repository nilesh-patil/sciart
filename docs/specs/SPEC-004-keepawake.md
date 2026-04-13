# SPEC-004: Keep-awake

**Crate**: `crates/sciart-keepawake` · **Milestone**: M3 · **Status**: Implemented (M3)

- **KA-R1** `Mode = Caffeinate | Mouse | Off`; parsed from `caffeinate|mouse|off`. Default: `Caffeinate` on macOS, `Off` elsewhere.
- **KA-R2** `start(mode, seed) -> Result<Guard, KeepAwakeError>`; the guard releases the assertion on `Drop`. Errors are reported, never fatal: the binary prints a warning and continues with `Guard::failed(mode)` (status `<mode>:unavailable`). `Guard::off()` is the explicit no-op guard.
- **KA-R3** `ChildGuard` (unix) owns a helper process: killed and reaped on `Drop`; `poll()` notices if it exited on its own. `Caffeinate` (macOS only, `cfg(target_os = "macos")`) is a `ChildGuard` running `caffeinate -dimsu -w <own pid>` — the `-w` makes it exit by itself if sciart dies without cleanup.
- **KA-R4** `Jiggler<M: Mover>` (always compiled, pure logic): after every `interval` (15 s) of `poll(elapsed)` time it asks the `Mover` for the screen size and moves to a seeded-random point with a 50 px margin (margin shrinks to `size/4` on screens smaller than 200 px). Any `Mover` error is swallowed and counted (Python R2.1 equivalent). The real `Mover` (enigo) exists only with cargo feature `mouse`; without it `start(Mouse)` returns `Unsupported`.
- **KA-R5** `Guard::status()` is a short HUD string: `caffeinate:on`, `caffeinate:exited`, `mouse:on`, `mouse:err(<n>)`, `<mode>:unavailable`, `off`.
- **KA-R6** Backends implement `trait Backend { fn status(&self) -> String; fn poll(&mut self, elapsed: f64) }`; `Guard::new(Box<dyn Backend>)` lets tests inject fakes.
