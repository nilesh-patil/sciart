# sciart-core

Internal crate of [sciart](https://crates.io/crates/sciart), holding the terminal-agnostic primitives every scene is built on: the canvas, glyphs and inks, the generator trait, the seeded context and the fixed-step clock.

It is published only because `sciart` depends on it. It carries no semver promise: its API
can change in any release, and each `sciart` release depends on exactly the version it was
tested with. Use the [`sciart`](https://docs.rs/sciart) crate, the supported entry point,
instead.

Licensed under the MIT licence ([LICENSE](LICENSE)).
