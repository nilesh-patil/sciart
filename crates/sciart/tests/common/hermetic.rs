//! The real binary with a hermetic environment (TEST-R12). Whatever the developer's shell or
//! CI sets (`NO_COLOR`, the terminal identity, a config file) must not change what a test
//! sees, so every raw spawn starts from here and sets only what the test is about; a later
//! `.env(..)` on the returned command overrides these defaults.

use std::process::Command;

/// `sciart` with no ambient colour or terminal settings and no config file: `NO_COLOR`,
/// `COLORTERM` and `TERM_PROGRAM` removed, `TERM=xterm-256color`, and `XDG_CONFIG_HOME`
/// pointing at a directory that does not exist (so no `config.toml` is found, CFG-R2).
pub fn sciart() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sciart"));
    command
        .env_remove("NO_COLOR")
        .env_remove("COLORTERM")
        .env_remove("TERM_PROGRAM")
        .env("TERM", "xterm-256color")
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join("sciart-tests-no-config-here"),
        );
    command
}
