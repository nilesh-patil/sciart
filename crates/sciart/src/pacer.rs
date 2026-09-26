//! Frame-rate policy: full speed while watched, low power when idle or unfocused
//! (SPEC-003 RT-R8, spec F5).

#[derive(Debug, Clone)]
pub struct Pacer {
    fps: u16,
    low_fps: u16,
    idle_after: f64,
    idle: f64,
    focused: bool,
}

impl Pacer {
    pub fn new(fps: u16, low_fps: u16, idle_after: f64) -> Self {
        Self {
            fps,
            low_fps,
            idle_after,
            idle: 0.0,
            focused: true,
        }
    }

    pub fn record(&mut self, elapsed: f64) {
        if elapsed.is_finite() && elapsed > 0.0 {
            self.idle += elapsed;
        }
    }

    pub fn input(&mut self) {
        self.idle = 0.0;
    }

    pub fn focus(&mut self, gained: bool) {
        self.focused = gained;
        if gained {
            self.idle = 0.0;
        }
    }

    pub fn low_power(&self) -> bool {
        !self.focused || self.idle >= self.idle_after
    }

    pub fn fps(&self) -> u16 {
        if self.low_power() {
            self.low_fps.min(self.fps)
        } else {
            self.fps
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rt_r8_idle_time_triggers_low_power_and_input_restores() {
        let mut p = Pacer::new(30, 5, 10.0);
        p.record(9.0);
        assert!(!p.low_power());
        assert_eq!(p.fps(), 30);
        p.record(1.0);
        assert!(p.low_power());
        assert_eq!(p.fps(), 5);
        p.input();
        assert!(!p.low_power());
        assert_eq!(p.fps(), 30);
    }

    #[test]
    fn rt_r8_focus_loss_triggers_low_power_regardless_of_input() {
        let mut p = Pacer::new(30, 5, 600.0);
        p.focus(false);
        assert!(p.low_power());
        p.input();
        assert!(p.low_power(), "still unfocused");
        p.record(700.0);
        p.focus(true);
        assert!(!p.low_power(), "focus gained also resets idle time");
    }

    #[test]
    fn rt_r8_low_fps_never_exceeds_normal_fps_and_bad_elapsed_is_ignored() {
        let mut p = Pacer::new(3, 5, 1.0);
        p.record(2.0);
        assert_eq!(p.fps(), 3);
        let mut q = Pacer::new(30, 5, 1.0);
        for bad in [f64::NAN, -5.0, f64::NEG_INFINITY] {
            q.record(bad);
        }
        assert!(!q.low_power());
    }
}
