# Configuration

sciart reads `~/.config/sciart/config.toml`, or `$XDG_CONFIG_HOME/sciart/config.toml` when
that variable is set. Use another file with `--config`:

```sh
sciart --config ~/sciart-night.toml
```

Every key is optional, and a flag on the command line wins over the file. A key sciart
does not know is an error, so a typo does not go unnoticed. sciart never writes the file.

Here is every key with its default, as shipped in `docs/config.example.toml`:

<!-- include: docs/config.example.toml -->

## A few recipes

A quiet evening: the aurora theme, one scene a minute, a lower frame rate.

```toml
theme = "aurora"
interval = 60
fps = 20
```

Only the space scenes, never the rocket. Categories are chosen on the command line, and
`--exclude` drops a scene from the cycle:

```sh
sciart --category deep-cosmos --category sun-planets --category voyages --exclude rocket_launch
```

To keep the rocket out of every cycle for good, disable it in the file instead; `sciart run
rocket_launch` still works:

```toml
disabled = ["rocket_launch"]
```

A few favourites to come back to; press `v` to show only them:

```toml
favorites = ["solar_system", "saturn_rings", "galaxy_collision"]
```
