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
command is not found, make sure the directory it was installed to is on your `PATH`:
`~/.local/bin` for the installer script (or the folder in `SCIART_INSTALL_DIR`), `~/.pixi/bin`
for `pixi global`, and Cargo's binary directory (usually `~/.cargo/bin`) for
`cargo install` and `cargo binstall`. Homebrew puts it on the `PATH` itself.

## Shell completions and the manual page

sciart can print completions for bash, zsh, fish, elvish and PowerShell, and its own
manual pages. See [Scripting](scripting.md#completions-and-manual-pages).
