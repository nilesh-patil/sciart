# Scripting

## Ending by itself

```sh
sciart --for 25m
sciart run aurora --for 90s
```

`--for` ends the run after that much wall time and exits with status 0. It takes a number
with `s`, `m` or `h` (`90s`, `25m`, `1.5h`), or a bare number of seconds, up to 24 hours.
It is a simple timer for a break or a talk.

## One frame for a greeting

```sh
sciart frame aurora --t 5 --size 80x24
```

`sciart frame` prints one frame of a scene as ANSI text and exits: a picture for a shell
greeting, a login message or a log. `--t` is how many simulated seconds to run before the
frame (default 5), and `--size` the size in cells; without it the frame fills the terminal,
or 80×24 when the output is not a terminal. `--seed` and `--theme` work as for `run`.

A greeting in `~/.zshrc` or `~/.bashrc`:

```sh
sciart frame solar_system --size 60x16 --theme aurora
```

## The art while a command runs

```sh
sciart while -- make -j8
sciart while --for 10m -- ./download-everything.sh
```

`sciart while -- <command>` runs the command and shows the art until it finishes. The
command gets no terminal: its input is empty, and its output is held back and printed,
standard output first and then standard error, once the art closes. That suits builds,
downloads and batch jobs, not editors or password prompts.

sciart then exits with the command's status, so `while` fits in scripts and `&&` chains.
Pressing `q` closes the art early and keeps waiting for the command; `Ctrl-C` stops the
command and exits with status 130; a `--for` deadline stops it with status 124. A command
that cannot be started gives status 127. The held output is not limited, so a command that
writes a great deal needs that much temporary disk space. `sciart man while` has every
detail.

## The catalogue as JSON

```sh
sciart list --json
```

prints every scene as one JSON object, `{"schema": 1, "scenes": [...]}`, sorted by name.
Each scene has its `name`, `category`, `category_label`, `title`, `description`,
`captions` and the old `aliases` it still answers to. New fields may be added; a change
that removes or redefines one raises `schema`. `sciart list --plain` prints tab-separated
lines instead.

## Completions and manual pages

```sh
sciart completions zsh > ~/.zfunc/_sciart
sciart completions bash > ~/.local/share/bash-completion/completions/sciart
sciart man > ~/.local/share/man/man1/sciart.1
sciart man while > ~/.local/share/man/man1/sciart-while.1
```

`sciart completions` prints a completion script for `bash`, `zsh`, `fish`, `elvish` or
`powershell`. `sciart man` prints the manual page; `sciart man <subcommand>` prints one
subcommand's page.
