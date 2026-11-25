# ADR-0032: The `sciart` library facade

- **Status**: Accepted
- **Date**: 2026-11-25

## Context

M25 makes sciart a library as well as a binary. The workspace has twelve crates, and
`sciart-core` alone has about 340 public items written for the scenes, not for outside
users. Several leak dependency types: `Ctx.rng` is a `ChaCha8Rng`, the renderer takes
ratatui's `Rect` and `Buffer`, and the one module the `sciart` lib exports today, the art
metrics, takes a `ratatui::Buffer`. Freezing that surface would tie every `rand` or
`ratatui` upgrade, and every refactor of the scene toolkit, to a major version. A `rand`
upgrade also changes every seeded frame.

## Decision

One crate carries the promise: the `sciart` package, library and binary together.

- The library owns a small surface: `Stage`, `scenes` and `find`, `SceneInfo`, `Category`,
  `Theme`, `Options`, `Error`, output as text, ANSI and PNG, and a `SceneWidget` for ratatui
  behind the `widget` feature.
- Every public enum is `#[non_exhaustive]` and every struct is opaque. Dependency types
  appear only in the widget.
- Writing a `Generator` is possible only under `unstable-generator`, with no promise.
- The binary's dependencies sit behind a default `cli` feature; a library user turns the
  defaults off.
- The internal crates are published because cargo needs them, with no semver promise.
- The surface is pinned by a reviewed snapshot; cargo-semver-checks joins once 1.0.0 exists
  to compare against.

## Consequences

- The internals stay free to change, including `rand` and the scene toolkit.
- Library users must write `default-features = false`, or they pull in the CLI's
  dependencies; the README says so first.
- A ratatui major release breaks the widget's signature: it needs a sciart major or a
  versioned feature, and the widget's docs name the ratatui version.
- A small mapping layer (`Category`, `Theme`, `SceneInfo`) has to track the internal types.
- Panic isolation needs unwinding; an embedder built with `panic = "abort"` loses it.
- The library offers less than the binary at 1.0: no GIF or asciicast output, no speed,
  pause or theme cycling, no 256-colour or background options. They can be added.

## Alternatives considered

- **Publish `sciart-core` as a scene SDK**: the largest ecosystem, and it freezes `rand`,
  the helper modules and the `Generator` trait; months of work to do and to keep.
- **A separate `sciart-lib` crate**: two names for one product.
- **Re-export the core `Stage`**: leaks `Ctx` and `rand`.
- **`cargo public-api` for pinning**: needs a nightly toolchain.
- **Binary 1.0 with every crate published as unstable 0.x and no facade**: fastest, and no
  library a user could rely on.
