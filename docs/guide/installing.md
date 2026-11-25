# Installing

sciart is a single program, `sciart`. The commands below are the project README's
installation instructions, copied into this page when the site is built, so the two always
agree.

<!-- include: README.md#Install -->

## Checking the installation

```sh
sciart --version
sciart list
```

`sciart list` prints every scene with its category and a one-line description. If the
command is not found, make sure Cargo's binary directory (usually `~/.cargo/bin`) is on
your `PATH`.

## Shell completions and the manual page

sciart can print completions for bash, zsh, fish, elvish and PowerShell, and its own
manual pages. See [Scripting](scripting.md#completions-and-manual-pages).
