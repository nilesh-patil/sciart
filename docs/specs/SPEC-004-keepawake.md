# SPEC-004: Keep-awake

**Crate**: `crates/sciart-keepawake` · **Milestone**: M3 · **Status**: KA-R1 implemented (M1); rest planned

- **KA-R1** `Mode = Caffeinate | Mouse | Off`; parsed from `caffeinate|mouse|off`. Default: `Caffeinate` on macOS, `Off` elsewhere.
- **KA-R2** `start(mode) -> Result<Guard, KeepAwakeError>`; the guard releases the assertion on `Drop`. Errors are reported, never fatal (the app continues with `Off`).
- **KA-R3** `Caffeinate` spawns `caffeinate -dimsu -w <own pid>` (exits automatically if sciart dies), killed and reaped on `Drop`. Compiled only on macOS (`cfg(target_os = "macos")`).
- **KA-R4** `Mouse` is behind the cargo feature `mouse` (enigo). Every `interval` (default 15 s) moves the cursor to a random point with a 50 px margin; any backend error (permission, fail-safe) is swallowed and counted (Python R2.1 equivalent).
- **KA-R5** `Guard::status()` returns a short string for the HUD (e.g. `caffeinate:on`, `mouse:err(3)`, `off`).
- **KA-R6** Backends are behind a `KeepAwake` trait so the runtime can be tested with a fake.
