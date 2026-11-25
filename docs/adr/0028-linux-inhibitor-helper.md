# ADR-0028: Linux keep-awake through an inhibitor helper process

- **Status**: Accepted
- **Date**: 2026-11-25

## Context

Keep-awake is half of what sciart is for, and it only works on macOS: the default mode off
macOS is `Off`, so on Linux the art runs and the machine sleeps anyway. Linux has no single
API for this. logind takes inhibitor locks over D-Bus; desktops add their own screen-saver
interfaces; Wayland's idle-inhibit protocol and `xdg-screensaver` need a window, which a
terminal program does not have. The workspace forbids `unsafe` and keeps dependencies few
(a D-Bus client such as `zbus` brings about forty crates).

macOS already has the shape that fits: `caffeinate -w <pid>` is a child process that holds
the assertion and exits when sciart does, wrapped by `ChildGuard` (ADR-0005).

## Decision

On Linux, `Mode::Inhibit` runs `systemd-inhibit` (or `elogind-inhibit`) as a child process
holding an `idle:sleep` block lock for as long as its command runs. The command is `cat`
reading a pipe whose write end sciart keeps: when sciart exits for any reason, including
`SIGKILL`, `cat` reads end-of-file, exits, and the lock is released. No new dependency, no
`unsafe`, and the same guard and status model as `caffeinate`.

It is the default on Linux for a local session and off over SSH, where keeping the remote
host awake is not what the viewer wants.

## Consequences

- Suspend and logind's idle action are blocked. A desktop that blanks the screen on its own
  timer without consulting logind may still blank; that is documented, not worked around.
- Hosts without logind (containers, some minimal distributions) report `inhibit:unavailable`
  or `inhibit:exited` in the HUD and carry on.
- Whether polkit lets a given session take the lock can only be known on that host; a
  refused lock shows as `inhibit:exited`.
- `ChildGuard` gains a piped-stdin form; the helper choice is a pure function, testable on
  macOS with a stand-in helper.

## Alternatives considered

- **D-Bus (`org.freedesktop.login1`, `org.freedesktop.ScreenSaver`, the portal) through
  `zbus`**: the correct interfaces, and the only way to reach desktop screen-saver
  inhibition, at the cost of a large dependency tree. Revisit after 1.0 if blanking on
  GNOME or KDE turns out to matter in practice.
- **`gnome-session-inhibit` / `kde-inhibit` helpers first**: desktop-specific helpers of the
  same child-process shape; can be added to the helper list later without changing the design.
- **Wayland idle-inhibit, `xdg-screensaver`**: need a surface or window id.
- **Leave Linux at `Off`**: ships a keep-awake tool that does not keep Linux awake.
