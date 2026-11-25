//! Terminal hint (SPEC-003 RND-R21, ADR-0029): one line of advice, printed on exit, when the
//! environment says the default truecolor braille output will not render. Advice only: the
//! colour mode is never switched on a guess (ADR-0026).

/// The environment variables the hint reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TermEnv {
    /// `TERM`.
    pub term: Option<String>,
    /// `TERM_PROGRAM`.
    pub term_program: Option<String>,
    /// `COLORTERM` set to a non-empty value.
    pub colorterm: bool,
}

impl TermEnv {
    /// The real process environment.
    pub fn from_process() -> Self {
        let var = |key| std::env::var(key).ok().filter(|v: &String| !v.is_empty());
        Self {
            term: var("TERM"),
            term_program: var("TERM_PROGRAM"),
            colorterm: var("COLORTERM").is_some(),
        }
    }
}

const LINUX_CONSOLE: &str = "sciart: hint: the Linux console has no braille glyphs and few \
     colours; try --ascii --color 256";
const APPLE_TERMINAL: &str = "sciart: hint: Terminal.app may not show 24-bit colour; if the \
     colours looked wrong, try --color 256";
const MULTIPLEXER: &str = "sciart: hint: screen or tmux may not pass 24-bit colour through; if \
     the colours looked wrong, try --color 256 or --color mono";

/// The advice for `env`, if any (RND-R21): the Linux console (`TERM=linux`), Apple Terminal
/// without `COLORTERM`, and `screen`/`tmux` without `COLORTERM`.
pub fn terminal_hint(env: &TermEnv) -> Option<&'static str> {
    let term = env.term.as_deref().unwrap_or("");
    if term == "linux" {
        return Some(LINUX_CONSOLE);
    }
    if env.colorterm {
        return None;
    }
    if env.term_program.as_deref() == Some("Apple_Terminal") {
        return Some(APPLE_TERMINAL);
    }
    if term.starts_with("screen") || term.starts_with("tmux") {
        return Some(MULTIPLEXER);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(term: Option<&str>, program: Option<&str>, colorterm: bool) -> TermEnv {
        TermEnv {
            term: term.map(String::from),
            term_program: program.map(String::from),
            colorterm,
        }
    }

    #[test]
    fn rnd_r21_linux_console_is_advised_whatever_colorterm_says() {
        for colorterm in [false, true] {
            assert_eq!(
                terminal_hint(&env(Some("linux"), None, colorterm)),
                Some(LINUX_CONSOLE)
            );
        }
    }

    #[test]
    fn rnd_r21_apple_terminal_and_multiplexers_only_without_colorterm() {
        let apple = Some("Apple_Terminal");
        assert_eq!(
            terminal_hint(&env(Some("xterm-256color"), apple, false)),
            Some(APPLE_TERMINAL)
        );
        assert_eq!(
            terminal_hint(&env(Some("xterm-256color"), apple, true)),
            None
        );
        for term in [
            "screen",
            "screen-256color",
            "screen.xterm-256color",
            "tmux-256color",
        ] {
            assert_eq!(
                terminal_hint(&env(Some(term), None, false)),
                Some(MULTIPLEXER),
                "{term}"
            );
            assert_eq!(terminal_hint(&env(Some(term), None, true)), None, "{term}");
        }
    }

    #[test]
    fn rnd_r21_capable_or_unknown_terminals_get_no_hint() {
        for e in [
            env(None, None, false),
            env(Some("xterm-256color"), None, false),
            env(Some("xterm-kitty"), Some("kitty"), false),
            env(Some("xterm-256color"), Some("iTerm.app"), true),
            env(Some("dumb"), None, false),
            env(Some("linux-ish"), None, false),
        ] {
            assert_eq!(terminal_hint(&e), None, "{e:?}");
        }
    }

    #[test]
    fn rnd_r21_hints_are_one_line_and_name_existing_flags_only() {
        for hint in [LINUX_CONSOLE, APPLE_TERMINAL, MULTIPLEXER] {
            assert!(!hint.contains('\n'), "{hint}");
            assert!(hint.starts_with("sciart: hint: "), "{hint}");
            let mut rest = hint;
            while let Some(at) = rest.find("--") {
                rest = &rest[at..];
                assert!(
                    ["--ascii", "--color 256", "--color mono"]
                        .iter()
                        .any(|flag| rest.starts_with(flag)),
                    "{hint}: unknown flag at {rest:?}"
                );
                rest = &rest[2..];
            }
        }
    }
}
