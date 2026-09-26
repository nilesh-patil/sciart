//! Key bindings (SPEC-003 RT-R9).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::runtime::Action;

/// Map a key *press* to an action; releases, repeats and unbound keys map to `None`.
pub fn action_for(key: KeyEvent) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return matches!(key.code, KeyCode::Char('c' | 'C')).then_some(Action::Quit);
    }
    Some(match key.code {
        KeyCode::Right => Action::Next,
        KeyCode::Left => Action::Prev,
        KeyCode::Esc => Action::Quit,
        KeyCode::Char(c) => match c.to_ascii_lowercase() {
            'n' => Action::Next,
            'p' => Action::Prev,
            ' ' => Action::TogglePause,
            'r' => Action::Reset,
            'h' => Action::ToggleHud,
            't' => Action::CycleTheme,
            'q' => Action::Quit,
            _ => return None,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn rt_r9_bindings() {
        let cases = [
            (KeyCode::Char('n'), Action::Next),
            (KeyCode::Right, Action::Next),
            (KeyCode::Char('p'), Action::Prev),
            (KeyCode::Left, Action::Prev),
            (KeyCode::Char(' '), Action::TogglePause),
            (KeyCode::Char('r'), Action::Reset),
            (KeyCode::Char('h'), Action::ToggleHud),
            (KeyCode::Char('t'), Action::CycleTheme),
            (KeyCode::Char('q'), Action::Quit),
            (KeyCode::Esc, Action::Quit),
        ];
        for (code, action) in cases {
            assert_eq!(action_for(press(code)), Some(action), "{code:?}");
        }
    }

    #[test]
    fn rt_r9_letters_are_case_insensitive_and_ctrl_c_quits() {
        let shifted = KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT);
        assert_eq!(action_for(shifted), Some(Action::Next));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(action_for(ctrl_c), Some(Action::Quit));
    }

    #[test]
    fn rt_r9_only_presses_of_bound_keys_count() {
        assert_eq!(action_for(press(KeyCode::Char('x'))), None);
        assert_eq!(
            action_for(press(KeyCode::Char('c'))),
            None,
            "plain c is unbound"
        );
        let mut release = press(KeyCode::Char('q'));
        release.kind = KeyEventKind::Release;
        assert_eq!(action_for(release), None);
    }
}
