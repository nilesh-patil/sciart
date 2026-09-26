/// Fixed-timestep accumulator (ADR-0003, CORE-R9..R12).
///
/// The runtime feeds real elapsed time in; generators only ever see a constant `dt`.
#[derive(Debug, Clone)]
pub struct FixedStep {
    dt: f64,
    max_steps: u32,
    acc: f64,
}

impl FixedStep {
    /// `dt` must be positive and finite; `max_steps` is clamped to `>= 1`.
    pub fn new(dt: f64, max_steps: u32) -> Self {
        assert!(
            dt.is_finite() && dt > 0.0,
            "FixedStep dt must be > 0, got {dt}"
        );
        Self {
            dt,
            max_steps: max_steps.max(1),
            acc: 0.0,
        }
    }

    pub fn dt(&self) -> f64 {
        self.dt
    }

    /// Add `elapsed` seconds and return how many `dt` steps to simulate now.
    pub fn advance(&mut self, elapsed: f64) -> u32 {
        if elapsed.is_finite() && elapsed > 0.0 {
            self.acc += elapsed;
        }
        // Epsilon: 0.05 + 0.05 accumulates to 0.0999.. and would otherwise lose a step.
        let due = (self.acc / self.dt + 1e-9).floor();
        if due >= f64::from(self.max_steps) {
            // Spiral-of-death guard: drop the backlog instead of catching up.
            self.acc = 0.0;
            return self.max_steps;
        }
        let steps = due as u32;
        self.acc -= f64::from(steps) * self.dt;
        // Guard float drift so alpha stays in [0, 1).
        if self.acc < 0.0 || self.acc >= self.dt {
            self.acc = 0.0;
        }
        steps
    }

    /// Fraction of the next step already accumulated, in `[0, 1)`.
    pub fn alpha(&self) -> f64 {
        self.acc / self.dt
    }

    pub fn reset(&mut self) {
        self.acc = 0.0;
    }
}
