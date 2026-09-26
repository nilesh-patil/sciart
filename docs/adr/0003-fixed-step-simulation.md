# ADR-0003: Fixed-step simulation, decoupled from frame rate

- **Status**: Accepted
- **Date**: 2026-09-26

## Context
Scenes include physics (pendulum, Brownian motion, orbits). Variable dt makes physics FPS-dependent and unstable after stalls; pure `render(t)` doesn't fit stateful simulations.

## Decision
Generators implement `update(dt)` with a constant `dt = 1/60 s`, driven by a `FixedStep` accumulator (`max_steps = 5` per frame, excess discarded). Rendering happens once per frame at the target FPS (30 default, 5 in low-power mode). Generators never read the wall clock.

## Consequences
+ Deterministic tests: 60 steps is exactly one simulated second.
+ Low-power mode keeps motion speed consistent.
− Heavy generators must fit 5 steps within the frame budget.

## Alternatives considered
Variable `dt`; pure `render(t)`.
