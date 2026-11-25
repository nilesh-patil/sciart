# ADR-0031: Scriptable modes — a deadline, one frame, and running beside a command

- **Status**: Accepted
- **Date**: 2026-11-25

## Context

Until 1.0 sciart had one way to be used: start it, watch it, press `q`. People reach for a
keep-awake art program in other shapes too: for a fixed time (a focus timer, a talk break),
for as long as a long command runs (a build, a download, a training job), and as a single
picture in a login message, a tmux status pane or a CI log. The pieces existed (a pure
runtime, a headless stage, an ANSI writer used by the recorder) but no command exposed them,
and the event loop could only end on a key.

## Decision

Three additions, each a thin command over what exists.

- **`--for <duration>`**: a wall-clock deadline held by the pure `Runtime`, so it is tested
  without a terminal and holds while paused.
- **`sciart frame <scene>`**: one deterministic frame as ANSI text on stdout. No terminal
  session, no config file, no keep-awake; available in every build.
- **`sciart while -- <command>`**: the art and keep-awake last as long as the child process.
  The child gets no terminal, because the art owns the alternate screen; its output is
  spooled to temporary files and replayed when the terminal is back. sciart exits with the
  child's status, so it can wrap a command in a script without changing the result.

`list --json` becomes a versioned object, and `completions` and `man` are generated from the
clap definition, so the scriptable surface is described by the binary itself.

## Consequences

- The CLI surface that 1.0 freezes grows by one flag and four subcommands.
- `while` takes on process semantics: exit-status passthrough, a spool that can grow with
  the child's output, and no interactive input for the child. Programs that need a terminal
  are out of scope and the manual says so.
- A child's stdout and stderr are replayed one after the other, so their interleaving is
  lost; each stream keeps its own order.
- `Ctrl-C` ends the child with the standard library's kill (no signal crate, no `unsafe`),
  so the child cannot clean up on interrupt; `q` leaves the child running.

## Alternatives considered

- **A pseudo-terminal for the child, shown in a pane beside the art**: a terminal
  multiplexer inside sciart; far beyond 1.0.
- **Let the child write to the terminal under the art**: corrupts both.
- **Only `--for`, leaving `while` to shell job control** (`cmd & sciart; wait`): the art
  would not end with the command, and the exit status would be sciart's.
- **`frame` as `record --frames 1 --format ansi`**: hides a text feature behind the image
  feature and the recorder's flags.
