//! Allocation-freedom of the shared hot-path helpers, measured with a counting global
//! allocator (ADR-0009). Own test binary: a binary has exactly one global allocator.
//!
//! SPEC-001 promises no heap traffic after construction for `ParticlePool` (CORE-R28),
//! `Trail` (CORE-R29) and `Grid::step` after its first call (CORE-R36). Counters are
//! per thread, so libtest's parallel threads do not pollute each other.

use std::hint::black_box;

use sciart_core::prelude::*;
use sciart_testalloc::{Allocs, CountingAlloc, measure};

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[derive(Debug, Clone, Copy, PartialEq)]
struct P {
    x: f64,
    y: f64,
    life: u32,
}

const fn p(i: u32) -> P {
    P {
        x: i as f64,
        y: 0.0,
        life: i,
    }
}

fn assert_free(what: &str, stats: Allocs) {
    assert_eq!(stats.total(), 0, "{what} allocated: {stats:?}");
}

#[test]
fn core_r28_pool_spawn_until_full_and_when_full_does_not_allocate() {
    let mut pool = ParticlePool::with_capacity(256);
    let (accepted, stats) = measure(|| (0..256).filter(|&i| pool.spawn(p(i))).count());
    assert_eq!(accepted, 256);
    assert_free("spawn until full", stats);

    let (rejected, stats) = measure(|| (0..1000).filter(|&i| !pool.spawn(p(i))).count());
    assert_eq!(rejected, 1000);
    assert_free("spawn when full", stats);
}

#[test]
fn core_r28_pool_retain_iter_clear_do_not_allocate() {
    let mut pool = ParticlePool::with_capacity(512);
    for i in 0..512 {
        pool.spawn(p(i));
    }
    let ((), stats) = measure(|| {
        // Update in place and kill every third particle (swap-remove compaction).
        pool.retain(|q| {
            q.x += 1.0;
            q.life % 3 != 0
        });
    });
    assert_free("retain", stats);
    assert!(pool.len() < 512);

    let (sum, stats) = measure(|| {
        let s: f64 = pool.iter().map(|q| q.x + q.y).sum();
        for q in &mut pool {
            q.y += 1.0;
        }
        s + (&pool).into_iter().count() as f64
    });
    assert_free("iter / iter_mut", stats);
    black_box(sum);

    let ((), stats) = measure(|| pool.clear());
    assert_free("clear", stats);
    assert!(pool.is_empty());

    // Refilling after clear reuses the buffer.
    let (n, stats) = measure(|| (0..512).filter(|&i| pool.spawn(p(i))).count());
    assert_eq!(n, 512);
    assert_free("respawn after clear", stats);
}

#[test]
fn core_r28_pool_shrink_and_regrow_within_original_buffer_does_not_allocate() {
    // CORE-R28: "no operation other than set_capacity growing may allocate"; growing
    // back within the buffer reserved at construction must not either.
    let mut pool = ParticlePool::with_capacity(128);
    for i in 0..128 {
        pool.spawn(p(i));
    }
    let ((), stats) = measure(|| {
        pool.set_capacity(16);
        pool.set_capacity(128);
    });
    assert_free("set_capacity shrink + regrow", stats);
    assert_eq!(pool.capacity(), 128);
}

#[test]
fn core_r28_pool_many_frames_of_churn_do_not_allocate() {
    // A particle scene's steady state: every frame some die and emitters refill.
    let mut pool: ParticlePool<P> = ParticlePool::with_capacity(300);
    let ((), stats) = measure(|| {
        for frame in 0..600u32 {
            pool.retain(|q| {
                q.life = q.life.wrapping_sub(1);
                q.life > 0 && q.life % 7 != 0
            });
            for k in 0..20 {
                pool.spawn(p(frame.wrapping_mul(31).wrapping_add(k) % 90 + 1));
            }
        }
    });
    assert_free("600 frames of retain + spawn", stats);
}

#[test]
fn core_r29_trail_push_and_iter_do_not_allocate_after_new() {
    let mut trail = Trail::new(32);
    let ((), stats) = measure(|| {
        // Filling, then wrapping many times (overwrite-oldest path).
        for i in 0..1000 {
            trail.push((i, i * 2));
        }
    });
    assert_free("Trail::push", stats);
    assert_eq!(trail.len(), 32);

    let (sum, stats) = measure(|| {
        trail.iter().map(|&(a, _)| a).sum::<i32>() + trail.newest().map_or(0, |(_, b)| b)
    });
    assert_free("Trail::iter / newest", stats);
    black_box(sum);

    let ((), stats) = measure(|| {
        trail.clear();
        trail.push((1, 1));
    });
    assert_free("Trail::clear + push", stats);

    let mut empty: Trail<u8> = Trail::new(0);
    let ((), stats) = measure(|| empty.push(1));
    assert_free("Trail::new(0).push", stats);
}

#[test]
fn core_r36_grid_step_does_not_allocate_after_first_call() {
    let mut grid = Grid::new(Size::new(200, 50), 0u8);
    grid.set(100, 25, 1);
    let mut scratch = Grid::default();
    // Game-of-life-ish rule over the Moore neighbourhood.
    let rule = |g: &Grid<u8>, x: i32, y: i32| {
        let n = g
            .neighbors(x, y, Hood::Moore, Edges::Wrap)
            .filter(|&(nx, ny)| g.get(nx, ny) == Some(&1))
            .count();
        u8::from(n == 3 || (n == 2 && g.get(x, y) == Some(&1)) || (x + y) % 17 == 0)
    };

    // The first call may size the (empty) scratch buffer: that is the one allowed allocation.
    let ((), first) = measure(|| grid.step(&mut scratch, rule));
    assert!(first.total() <= 1, "first step: {first:?}");

    let ((), stats) = measure(|| {
        for _ in 0..20 {
            grid.step(&mut scratch, rule);
        }
    });
    assert_free("Grid::step after the first call", stats);
    black_box(grid.as_slice());
}

#[test]
fn core_r36_grid_step_with_presized_scratch_never_allocates() {
    // CORE-R36: "scratch is ... otherwise not allocated" — a scratch already the right
    // size must not trigger even the first-call allocation.
    let size = Size::new(80, 24);
    let mut grid = Grid::new(size, 0u16);
    let mut scratch = Grid::new(size, 0u16);
    let ((), stats) = measure(|| {
        for _ in 0..10 {
            grid.step(&mut scratch, |g, x, y| {
                g.get(x, y).map_or(0, |v| v.wrapping_add(1))
            });
        }
    });
    assert_free("Grid::step with pre-sized scratch", stats);
}
