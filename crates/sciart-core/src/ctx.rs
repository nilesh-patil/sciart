use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::Size;

/// Per-generator context: the only source of randomness and time (CORE-R13, ADR-0008).
#[derive(Debug, Clone)]
pub struct Ctx {
    pub rng: ChaCha8Rng,
    size: Size,
    time: f64,
}

impl Ctx {
    pub fn new(seed: u64, size: Size) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed),
            size,
            time: 0.0,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// Called by the runtime before `Generator::resize`.
    pub fn set_size(&mut self, size: Size) {
        self.size = size;
    }

    /// Simulated seconds since the generator was built.
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Called by the runtime once per fixed step.
    pub fn advance(&mut self, dt: f64) {
        self.time += dt;
    }
}
