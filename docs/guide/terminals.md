# Colour and terminals

## Colour modes

sciart draws in 24-bit colour by default. Choose with `--color`:

| Mode | What it does |
| --- | --- |
| `auto` | 24-bit colour, unless `NO_COLOR` is set |
| `truecolor` | always 24-bit colour |
| `256` | the nearest of the 256 xterm colours, for tmux and older terminals |
| `mono` | the grey `mono` theme, whatever `--theme` says |

When the `NO_COLOR` environment variable is set, `auto` switches to `mono` and says so
once; an explicit `--color` wins over it.

## Colour-blind viewing

`--theme colorblind` uses the Okabe–Ito palette: vermillion and sky blue take the place of
the red and green pairs, and the colour ramps run from blue to vermillion. `t` reaches it
after `aurora`.

## The background

By default every cell is drawn on opaque black, which is how the scenes are meant to look.
`--background terminal` leaves the terminal's own background showing instead, for a
transparent or tinted terminal.

## Fonts

The scenes use braille dots and half blocks. Most modern terminal fonts have both. If you
see boxes or question marks instead, use a font with braille characters, or run with
`--ascii`.

## Keeping the machine awake

While sciart runs it keeps the machine from sleeping. On macOS it uses `caffeinate`; on
Linux it holds a `systemd-inhibit` (or `elogind-inhibit`) lock. Over SSH it does nothing by
default, since keeping the remote machine awake is rarely what you want. Choose with
`--keep-awake`:

```sh
sciart --keep-awake off
```

The values are `caffeinate`, `inhibit`, `mouse` and `off`. `mouse` nudges the pointer
instead; it needs a build with the `mouse` feature and, on macOS, Accessibility
permission. If the chosen way cannot start, sciart warns and keeps drawing.

## Platforms

macOS and Linux are supported. The same build, scene, seed and size give the same frames
on the same platform; frames may differ between platforms in the last bits of the
mathematics.
