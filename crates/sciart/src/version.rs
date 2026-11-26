//! The text of `sciart --version` (SPEC-008 DIST-R2).
//!
//! `-V` prints the crate version alone. `--version` adds the commit and the build target
//! when the build was given `SCIART_GIT_SHA` (the release workflow and the conda recipe do);
//! a build without it, such as `cargo install sciart`, prints the version alone.

use std::sync::OnceLock;

/// The commit the binary was built from, when the build environment named one. rustc
/// records the variable in the dep-info, so cargo rebuilds when it changes.
const GIT_SHA: Option<&str> = option_env!("SCIART_GIT_SHA");

/// The `--version` text after the program name: `X.Y.Z`, or `X.Y.Z (<commit> <target>)`
/// when `sha` is a full or abbreviated commit of at least 12 lower-case hex digits, cut to
/// 12. Anything else is not a commit and is left out rather than printed.
pub(crate) fn version_text(version: &str, sha: Option<&str>, target: &str) -> String {
    match sha.map(str::trim).and_then(short_commit) {
        Some(commit) => format!("{version} ({commit} {target})"),
        None => version.to_owned(),
    }
}

fn short_commit(sha: &str) -> Option<&str> {
    let hex = sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    (hex && sha.len() >= 12).then(|| &sha[..12])
}

/// The target triple of a build, from the pieces `std::env::consts` and `cfg!` give: the
/// six release targets (TEST-R13) come out exactly; others in the same `arch-vendor-os-env`
/// shape.
pub(crate) fn target_triple(arch: &str, os: &str, env: &str) -> String {
    let (vendor, os) = match os {
        "macos" => ("apple", "darwin"),
        other => ("unknown", other),
    };
    if env.is_empty() {
        format!("{arch}-{vendor}-{os}")
    } else {
        format!("{arch}-{vendor}-{os}-{env}")
    }
}

fn build_env() -> &'static str {
    if cfg!(target_env = "gnu") {
        "gnu"
    } else if cfg!(target_env = "musl") {
        "musl"
    } else {
        ""
    }
}

/// The `--version` text of this binary, built once.
pub(crate) fn long() -> &'static str {
    static LONG: OnceLock<String> = OnceLock::new();
    LONG.get_or_init(|| {
        let target = target_triple(std::env::consts::ARCH, std::env::consts::OS, build_env());
        version_text(env!("CARGO_PKG_VERSION"), GIT_SHA, &target)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn dist_r2_version_alone_without_a_commit() {
        assert_eq!(version_text("1.2.3", None, "aarch64-apple-darwin"), "1.2.3");
    }

    #[test]
    fn dist_r2_commit_cut_to_twelve_hex_digits_and_target() {
        assert_eq!(
            version_text("1.2.3", Some(SHA), "x86_64-unknown-linux-musl"),
            "1.2.3 (0123456789ab x86_64-unknown-linux-musl)"
        );
        assert_eq!(
            version_text("1.2.3", Some("0123456789ab\n"), "t"),
            "1.2.3 (0123456789ab t)"
        );
    }

    #[test]
    fn dist_r2_anything_but_a_commit_is_left_out() {
        for bad in ["", "   ", "0123456789a", "0123456789ABCDEF", "not-a-commit-at-all", "v1.2.3"] {
            assert_eq!(version_text("1.2.3", Some(bad), "t"), "1.2.3", "{bad:?}");
        }
    }

    #[test]
    fn dist_r2_the_six_release_targets() {
        for (arch, os, env, want) in [
            ("aarch64", "macos", "", "aarch64-apple-darwin"),
            ("x86_64", "macos", "", "x86_64-apple-darwin"),
            ("x86_64", "linux", "gnu", "x86_64-unknown-linux-gnu"),
            ("aarch64", "linux", "gnu", "aarch64-unknown-linux-gnu"),
            ("x86_64", "linux", "musl", "x86_64-unknown-linux-musl"),
            ("aarch64", "linux", "musl", "aarch64-unknown-linux-musl"),
        ] {
            assert_eq!(target_triple(arch, os, env), want);
        }
    }

    #[test]
    fn dist_r2_this_build_names_its_version() {
        assert!(long().starts_with(env!("CARGO_PKG_VERSION")), "{}", long());
    }

    #[test]
    fn dist_r2_short_and_long_flags_print_these_texts() {
        let command = <crate::cli::Cli as clap::CommandFactory>::command();
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(command.render_version(), format!("sciart {version}\n"));
        assert_eq!(command.render_long_version(), format!("sciart {}\n", long()));
    }
}
