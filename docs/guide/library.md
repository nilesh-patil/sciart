# The library

The `sciart` crate is also a Rust library. It runs any scene without a terminal, lists the
scenes, turns a frame into text, ANSI or a PNG, and draws a scene inside your own ratatui
application.

## Adding it

Turn the default features off, or you also get the command line's dependencies:

```toml
[dependencies]
sciart = { git = "https://github.com/nilesh-patil/sciart", default-features = false, features = ["png", "widget"] }
```

| Feature | Adds |
| --- | --- |
| `png` | `Stage::render_png` |
| `widget` | `SceneWidget`, a ratatui 0.30 widget |
| `unstable-generator` | writing your own scene, with no stability promise |

## Running a scene

```rust
use sciart::{Options, Stage, Theme};

let mut stage = Stage::new("aurora", 80, 24, Options::default().seed(7))?;
stage.advance(5.0)?;
print!("{}", stage.render_ansi(Theme::Astro)?);
```

A `Stage` runs one scene at a fixed step of 1/60 s. `advance` takes simulated seconds and
takes the whole steps they add up to, so `advance(1.0)` is the same as sixty calls of
`advance(1.0 / 60.0)`. `render_text` gives plain text, one line per row; `render_ansi`
gives the coloured text `sciart frame` prints; `render_png` (with `png`) writes a PNG at
8×16 pixels per cell.

## Listing scenes

```rust
for scene in sciart::scenes() {
    println!("{}\t{}\t{}", scene.name(), scene.category().id(), scene.title());
}
let renamed = sciart::find("chemical_reaction").expect("an old name still works");
assert_eq!(renamed.name(), "gray_scott");
```

## What is promised

Only the `sciart` crate's library has a stability promise; the crates behind it are
internal. The same build, scene, seed and size give the same frames on the same platform,
not across platforms or versions. The examples `headless.rs` and `widget.rs` in
`crates/sciart/examples` show both uses end to end.
