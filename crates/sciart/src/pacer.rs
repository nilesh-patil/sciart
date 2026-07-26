//! Frame-rate policy: full speed while watched and moving, low power when idle or
//! unfocused (SPEC-003 RT-R8, spec F5), and a lower draw rate while nothing changes
//! (RT-R17).

/// Quiet seconds per halving of the draw rate (RT-R17): 30 → 15 after 0.5 s without a
/// changed frame, → 7 after 1 s, → `low_fps` (5) after 1.5 s. A scene whose dots move
/// more often than this never steps down.
const QUIET_STEP_SECS: f64 = 0.5;

#[derive(Debug, Clone)]
pub struct Pacer {
    fps: u16,
    low_fps: u16,
    idle_after: f64,
    /// Seconds since the last key (or focus gain).
    idle: f64,
    /// Seconds since the last drawn frame that changed the picture, a key, a resize or a
    /// scene switch (RT-R17).
    quiet: f64,
    focused: bool,
}

impl Pacer {
    pub fn new(fps: u16, low_fps: u16, idle_after: f64) -> Self {
        Self {
            fps,
            low_fps,
            idle_after,
            idle: 0.0,
            quiet: 0.0,
            focused: true,
        }
    }

    /// Wall time since the last frame.
    pub fn record(&mut self, elapsed: f64) {
        if elapsed.is_finite() && elapsed > 0.0 {
            self.idle += elapsed;
            self.quiet += elapsed;
        }
    }

    /// A frame was drawn; `changed` when it changed the picture (RT-R17).
    pub fn drawn(&mut self, changed: bool) {
        if changed {
            self.quiet = 0.0;
        }
    }

    /// Any key event, bound or not (RT-R8, RT-R17).
    pub fn input(&mut self) {
        self.idle = 0.0;
        self.quiet = 0.0;
    }

    /// A resize or a scene switch: back to the full rate (RT-R17).
    pub fn wake(&mut self) {
        self.quiet = 0.0;
    }

    pub fn focus(&mut self, gained: bool) {
        self.focused = gained;
        if gained {
            self.idle = 0.0;
            self.quiet = 0.0;
        }
    }

    pub fn low_power(&self) -> bool {
        !self.focused || self.idle >= self.idle_after
    }

    /// The draw rate: `fps`, or `min(low_fps, fps)` in low power (RT-R8), halved for every
    /// `QUIET_STEP_SECS` without a change down to that same floor (RT-R17).
    pub fn fps(&self) -> u16 {
        let floor = self.low_fps.min(self.fps);
        if self.low_power() {
            return floor;
        }
        // Truncation is the point: whole steps of quiet time. `quiet` is finite and ≥ 0.
        let halvings = (self.quiet / QUIET_STEP_SECS) as u32;
        self.fps.checked_shr(halvings).unwrap_or(0).max(floor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rt_r8_idle_time_triggers_low_power_and_input_restores() {
        let mut p = Pacer::new(30, 5, 10.0);
        p.record(9.0);
        p.drawn(true); // a moving scene: only idle time counts here (RT-R17 aside)
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

    /// Frames of an app loop that draws nothing new: each lasts `1 / fps()`.
    fn quiet_for(p: &mut Pacer, secs: f64) -> Vec<u16> {
        let mut rates = Vec::new();
        let mut t = 0.0;
        while t < secs {
            let fps = p.fps();
            rates.push(fps);
            let dt = 1.0 / f64::from(fps);
            p.record(dt);
            p.drawn(false);
            t += dt;
        }
        rates
    }

    #[test]
    fn rt_r17_a_run_of_empty_frames_steps_the_rate_down_to_low_power_fps() {
        let mut p = Pacer::new(30, 5, 120.0);
        let rates = quiet_for(&mut p, 3.0);
        assert_eq!(rates[0], 30, "full rate at first");
        assert!(
            rates.windows(2).all(|w| w[1] <= w[0]),
            "steps only down: {rates:?}"
        );
        assert!(
            rates.iter().any(|&r| r > 5 && r < 30),
            "steps, not a jump: {rates:?}"
        );
        assert_eq!(p.fps(), 5, "settles at low_power.fps: {rates:?}");
        assert!(rates.iter().all(|&r| r >= 5), "never below low_power.fps");
        assert!(
            !p.low_power(),
            "quiet is not idle: the HUD shows no LOW-POWER"
        );
        // A short quiet spell (a slow orbit between dot moves) keeps the full rate.
        let mut q = Pacer::new(30, 5, 120.0);
        for _ in 0..10 {
            q.record(1.0 / 30.0);
            q.drawn(false);
        }
        assert_eq!(q.fps(), 30);
    }

    #[test]
    fn rt_r17_a_changed_frame_restores_the_full_rate() {
        let mut p = Pacer::new(30, 5, 120.0);
        quiet_for(&mut p, 3.0);
        p.record(0.2);
        p.drawn(true);
        assert_eq!(p.fps(), 30);
        // Frames that keep changing never step down.
        for _ in 0..300 {
            p.record(1.0 / 30.0);
            p.drawn(true);
            assert_eq!(p.fps(), 30);
        }
    }

    #[test]
    fn rt_r17_a_key_a_resize_or_a_switch_restores_the_full_rate() {
        let mut p = Pacer::new(30, 5, 120.0);
        quiet_for(&mut p, 3.0);
        p.input(); // any key
        assert_eq!(p.fps(), 30);
        quiet_for(&mut p, 3.0);
        p.wake(); // resize or scene switch
        assert_eq!(p.fps(), 30);
        quiet_for(&mut p, 3.0);
        p.focus(true);
        assert_eq!(p.fps(), 30);
    }

    #[test]
    fn rt_r17_quiet_never_raises_a_rate_already_at_or_below_the_floor() {
        // Low power already draws at low_power.fps; quiet frames cannot go below it.
        let mut p = Pacer::new(30, 5, 1.0);
        p.record(2.0);
        assert!(quiet_for(&mut p, 3.0).iter().all(|&r| r == 5));
        // A full rate at or below low_power.fps is never stepped.
        let mut q = Pacer::new(3, 5, 120.0);
        assert!(quiet_for(&mut q, 5.0).iter().all(|&r| r == 3));
        let mut r = Pacer::new(5, 5, 120.0);
        assert!(quiet_for(&mut r, 5.0).iter().all(|&r| r == 5));
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
