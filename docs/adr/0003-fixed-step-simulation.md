# ADR-0003: Fixed-step simulation, decoupled from frame rate

- **Status**: Accepted (amended 2026-09-26, M3: `max_steps` 5 → 15)
- **Date**: 2026-09-26

## Context
Scenes include physics (pendulum, Brownian motion, orbits). Variable dt makes physics FPS-dependent and unstable after stalls; pure `render(t)` doesn't fit stateful simulations.

## Decision
Generators implement `update(dt)` with a constant `dt = 1/60 s`, driven by a `FixedStep` accumulator (`max_steps = 15` per frame, excess discarded). Rendering happens once per frame at the target FPS (30 default, 5 in low-power mode). Generators never read the wall clock.

**Amendment (M3).** The original `max_steps = 5` covered 30 FPS (2 steps/frame) with headroom, but low-power mode renders at 5 FPS, which needs 12 steps per frame. With a cap of 5, motion would have run at 42 % speed, contradicting spec F5. `15` keeps full speed down to 4 FPS (RT-R11). Stalls longer than 0.25 s are still dropped rather than replayed.

## Consequences
+ Deterministic tests: 60 steps is exactly one simulated second.
+ Low-power mode keeps motion speed consistent.
− Catching up after a hiccup at 30 FPS can run up to 15 steps in one frame; with the GEN-R8 budget (< 5 ms per step+render, typically µs) that is acceptable.

## Alternatives considered
Variable `dt`; pure `render(t)`; a per-FPS `max_steps` setter on `FixedStep` (more API for no practical gain).
