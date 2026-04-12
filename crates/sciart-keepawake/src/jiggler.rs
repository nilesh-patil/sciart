//! Periodic mouse movement, independent of any real mouse API (KA-R4).

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::Backend;

/// Seconds between moves.
pub const INTERVAL: f64 = 15.0;
/// Distance kept from screen edges, in pixels (shrinks on tiny screens).
pub const MARGIN: i32 = 50;

/// The OS-facing part of the mouse backend.
pub trait Mover {
    fn screen_size(&mut self) -> Result<(i32, i32), String>;
    fn move_to(&mut self, x: i32, y: i32) -> Result<(), String>;
}

#[derive(Debug)]
pub struct Jiggler<M: Mover> {
    mover: M,
    rng: ChaCha8Rng,
    interval: f64,
    since_move: f64,
    errors: u32,
}

impl<M: Mover> Jiggler<M> {
    pub fn new(mover: M, seed: u64, interval: f64) -> Self {
        Self {
            mover,
            rng: ChaCha8Rng::seed_from_u64(seed),
            interval,
            since_move: 0.0,
            errors: 0,
        }
    }

    pub fn mover(&self) -> &M {
        &self.mover
    }

    fn jiggle(&mut self) -> Result<(), String> {
        let (w, h) = self.mover.screen_size()?;
        if w < 1 || h < 1 {
            return Err(format!("unusable screen size {w}x{h}"));
        }
        // Inclusive pixel range [m, dim-1-m]; m shrinks so the range is never empty.
        let mut pick = |dim: i32| {
            let m = MARGIN.min(dim / 4);
            self.rng.random_range(m..=dim - 1 - m)
        };
        let (x, y) = (pick(w), pick(h));
        self.mover.move_to(x, y)
    }
}

impl<M: Mover> Backend for Jiggler<M> {
    fn status(&self) -> String {
        if self.errors == 0 {
            "mouse:on".into()
        } else {
            format!("mouse:err({})", self.errors)
        }
    }

    fn poll(&mut self, elapsed: f64) {
        if elapsed.is_finite() && elapsed > 0.0 {
            self.since_move += elapsed;
        }
        if self.since_move >= self.interval {
            self.since_move = 0.0;
            if self.jiggle().is_err() {
                self.errors += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct FakeMover {
        size: (i32, i32),
        fail: bool,
        moves: Vec<(i32, i32)>,
    }

    impl Mover for FakeMover {
        fn screen_size(&mut self) -> Result<(i32, i32), String> {
            if self.fail {
                Err("no accessibility permission".into())
            } else {
                Ok(self.size)
            }
        }
        fn move_to(&mut self, x: i32, y: i32) -> Result<(), String> {
            self.moves.push((x, y));
            Ok(())
        }
    }

    fn jiggler(size: (i32, i32)) -> Jiggler<FakeMover> {
        let mover = FakeMover {
            size,
            ..FakeMover::default()
        };
        Jiggler::new(mover, 7, INTERVAL)
    }

    #[test]
    fn ka_r4_moves_once_per_interval() {
        let mut j = jiggler((1920, 1080));
        j.poll(10.0);
        assert!(j.mover().moves.is_empty());
        j.poll(6.0);
        assert_eq!(j.mover().moves.len(), 1);
        j.poll(15.0);
        assert_eq!(j.mover().moves.len(), 2);
        assert_eq!(j.status(), "mouse:on");
    }

    #[test]
    fn ka_r4_targets_keep_the_margin() {
        let mut j = jiggler((1920, 1080));
        for _ in 0..300 {
            j.poll(INTERVAL);
        }
        for &(x, y) in &j.mover().moves {
            assert!((MARGIN..=1920 - 1 - MARGIN).contains(&x), "x={x}");
            assert!((MARGIN..=1080 - 1 - MARGIN).contains(&y), "y={y}");
        }
    }

    #[test]
    fn ka_r4_margin_shrinks_on_tiny_screens() {
        let mut j = jiggler((100, 60));
        for _ in 0..200 {
            j.poll(INTERVAL);
        }
        for &(x, y) in &j.mover().moves {
            assert!((25..=74).contains(&x), "x={x}");
            assert!((15..=44).contains(&y), "y={y}");
        }
        let mut one = jiggler((1, 1));
        one.poll(INTERVAL);
        assert_eq!(one.mover().moves, vec![(0, 0)]);
    }

    #[test]
    fn ka_r4_errors_are_counted_never_fatal() {
        let mut j = Jiggler::new(
            FakeMover {
                fail: true,
                ..FakeMover::default()
            },
            1,
            INTERVAL,
        );
        j.poll(INTERVAL);
        assert_eq!(j.status(), "mouse:err(1)");
        j.poll(INTERVAL);
        assert_eq!(j.status(), "mouse:err(2)");
        let mut degenerate = jiggler((0, 900));
        degenerate.poll(INTERVAL);
        assert_eq!(
            degenerate.status(),
            "mouse:err(1)",
            "empty screen is an error"
        );
    }

    #[test]
    fn ka_r4_same_seed_same_path() {
        let path = |seed| {
            let mut j = Jiggler::new(
                FakeMover {
                    size: (800, 600),
                    ..FakeMover::default()
                },
                seed,
                INTERVAL,
            );
            for _ in 0..5 {
                j.poll(INTERVAL);
            }
            j.mover().moves.clone()
        };
        assert_eq!(path(3), path(3));
    }
}
