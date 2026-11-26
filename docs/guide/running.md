# Running

## Cycling through the scenes

```sh
sciart
```

With no arguments sciart cycles through every scene, changing every 15 seconds, until you
press `q`. This is the `sim` mode; `sciart sim` is the same thing. Change the pace with
`--interval`:

```sh
sciart --interval 60
```

## One scene

```sh
sciart run aurora
```

`sciart list` names every scene; `sciart list --categories` shows the eleven categories
and how many scenes each has. A few scenes were renamed over time, and their old names
still work.

## Choosing what to cycle

```sh
sciart --category deep-cosmos --category sun-planets
sciart --only solar_system --only saturn_rings --only black_hole
sciart --exclude rocket_launch
```

`--category` takes a category id from `sciart list --categories` and can be repeated, as
can `--only` and `--exclude`. In the configuration file, `disabled` takes scenes out of the
cycle and `favorites` marks the ones `v` shows.

## Themes

Five colour themes: `astro` (the default), `rainbow`, `mono`, `aurora` and the
colour-blind-safe `colorblind`. Pick one with `--theme`, or press `t` to cycle while the art
runs:

```sh
sciart run galaxy_collision --theme mono
```

## Seed, speed and frame rate

```sh
sciart run double_pendulum --seed 7
sciart --speed 0.5
sciart --fps 60
```

Each scene draws its randomness from a seed. Without `--seed` sciart picks one at random;
with the same seed and terminal size a scene plays out the same way every time on the same
machine. `--speed` runs simulated time slower or faster than the clock (0.1 to 4), and
`--fps` sets the frame rate (1 to 120, default 30). When nobody is watching (no key press
for two minutes, or the terminal window is unfocused) sciart draws fewer frames; the
scenes keep their real speed.

## The status line and the title

`--hud` starts with the status line on (`h` toggles it); it shows the scene and the
current settings. `--no-title` hides the science title in the bottom-left corner. `--ascii`
draws with plain ASCII characters, for terminals or fonts without braille and block
glyphs.

## Ending

`q`, `Esc` or `Ctrl-C` quits. `--for` ends the run by itself after a while; see
[Scripting](scripting.md).
