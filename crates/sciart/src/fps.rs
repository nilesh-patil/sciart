//! Measured frame rate for the HUD (SPEC-003 RT-R10).

#[derive(Debug, Default, Clone)]
pub struct FpsMeter {
    ema: f64,
}

impl FpsMeter {
    const ALPHA: f64 = 0.1;

    pub fn record(&mut self, frame_secs: f64) {
        if !(frame_secs.is_finite() && frame_secs > 0.0) {
            return;
        }
        let instant = 1.0 / frame_secs;
        self.ema = if self.ema == 0.0 {
            instant
        } else {
            self.ema * (1.0 - Self::ALPHA) + instant * Self::ALPHA
        };
    }

    pub fn fps(&self) -> f64 {
        self.ema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rt_r10_zero_before_samples_and_first_sample_seeds() {
        let mut m = FpsMeter::default();
        assert_eq!(m.fps(), 0.0);
        m.record(0.02);
        assert!((m.fps() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn rt_r10_moving_average_converges_and_ignores_bad_samples() {
        let mut m = FpsMeter::default();
        m.record(0.1); // 10 fps
        m.record(0.05); // 20 fps -> 10*0.9 + 20*0.1 = 11
        assert!((m.fps() - 11.0).abs() < 1e-9);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            m.record(bad);
        }
        assert!((m.fps() - 11.0).abs() < 1e-9);
        for _ in 0..200 {
            m.record(1.0 / 30.0);
        }
        assert!((m.fps() - 30.0).abs() < 0.01);
    }
}
