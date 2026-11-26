# Recording

```sh
sciart record aurora --out aurora.gif
```

`sciart record` renders a scene without a terminal, at 8×16 pixels per cell, and writes it
to a file. It needs no screen and keeps nothing awake, so it works on a server or in a
script. The output depends on the name you give:

| `--out` | Output |
| --- | --- |
| `aurora.gif` | a looping GIF |
| `frames.png` | a directory of PNG frames, `frame-000.png` and on |
| `aurora.cast` or `.json` | an asciicast, to play with asciinema |

`--format gif|png|asciicast` overrides the guess.

## Timing

```sh
sciart record total_solar_eclipse --start 6 --seconds 8 --fps 20 --out eclipse.gif
```

- `--seconds` is the length of the recording in simulated seconds (default 5);
  `--frames` gives a number of frames instead.
- `--fps` is the frame rate (default 30). It must divide 60 (1, 2, 3, 4, 5, 6, 10, 12, 15,
  20, 30 or 60) so every frame falls on the scene's fixed 1/60 s steps.
- `--start` runs the scene for that many simulated seconds before the first frame, to skip
  a slow opening or to catch a later moment of a story.

A recording is capped at 36,000 frames.

## Size, seed and theme

```sh
sciart record galaxy_collision --size 100x30 --seed 7 --theme aurora --out galaxies.gif
```

`--size` is in cells (default 80×24). Without `--seed` a recording uses seed 42, so it is
the same every time; `--theme` and `--ascii` work as for `run`.

Every clip on this site and in the README was made this way, from a list of scenes and
parameters kept with the project.

## A build without the recorder

The default build includes the recorder. [Installing](installing.md) shows how to build
the program without the image encoders.
