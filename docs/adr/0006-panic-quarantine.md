# ADR-0006: catch_unwind + quarantine for generator panics

- **Status**: Accepted
- **Date**: 2026-09-26

## Context
The app runs unattended for hours. The Python version crashed on single-generator failures until fault isolation was added (issue-01, R2.2).

## Decision
The runtime wraps each generator `update`/`render` in `catch_unwind(AssertUnwindSafe(..))`. A panicking generator is quarantined for the session; the last good frame stays on screen; `sim` moves on. Contract and property suites make any panic a test failure — quarantine is only a safety net. The release profile keeps `panic = "unwind"`.

## Consequences
+ One bad scene can't crash the app; the failure shows in the HUD.
− Under `AssertUnwindSafe` a quarantined generator's state may be inconsistent — so it is never reused.

## Alternatives considered
Reset and retry (loops on deterministic bugs); no catching (a crash also kills keep-awake).
