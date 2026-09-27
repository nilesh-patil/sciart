//! Allocation-freedom of the shared hot-path helpers, measured with a counting global
//! allocator (ADR-0009). Own test binary: a binary has exactly one global allocator.
//!
//! SPEC-001 promises no heap traffic after construction for `ParticlePool` (CORE-R28),
//! `Trail` (CORE-R29), `Grid::step` after its first call (CORE-R36), the rasters
//! (CORE-R41, CORE-R42), resampling (CORE-R43), the leapfrog integrator (CORE-R44) and
//! the shared M10 scene helpers (CORE-R49..R59) and the post-M10 audit helpers
//! (CORE-R60..R70).
//! Counters are per thread, so libtest's parallel threads do not pollute each other.

use std::fmt::Write as _;
use std::hint::black_box;

use sciart_core::prelude::*;
use sciart_core::raster::{Dots, Pixels};
use sciart_core::{draw, geom, grid, ink, ode, rng};
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

#[test]
fn core_r41_dots_set_plot_clear_blit_do_not_allocate() {
    let size = Size::new(200, 50);
    let mut dots = Dots::new(size);
    let mut canvas = Canvas::new(size, GlyphMode::Unicode);
    let ink = Ink::new(Role::Particle, 200);
    let ((), stats) = measure(|| {
        for frame in 0..3 {
            dots.clear();
            for i in 0..4000 {
                let t = f64::from(i) * 0.01 + f64::from(frame);
                dots.plot(200.0 + 180.0 * t.cos(), 100.0 + 90.0 * t.sin(), ink);
                dots.set(i % 400, i / 20, ink);
            }
            black_box(dots.is_lit(3, 3));
            dots.blit(&mut canvas);
            canvas.set_mode(GlyphMode::Ascii);
            dots.blit(&mut canvas);
            canvas.set_mode(GlyphMode::Unicode);
        }
    });
    assert_free("Dots set/plot/clear/blit", stats);
    black_box(canvas.cells());
}

#[test]
fn core_r42_pixels_set_fill_blit_do_not_allocate() {
    let size = Size::new(200, 50);
    let mut px = Pixels::new(size);
    let mut canvas = Canvas::new(size, GlyphMode::Unicode);
    let ((), stats) = measure(|| {
        px.fill(Ink::new(Role::Seq, 10));
        for y in 0..100 {
            for x in 0..200 {
                px.set(x, y, Ink::new(Role::Div, ((x + y) % 256) as u8));
            }
        }
        black_box(px.get(5, 5));
        px.blit(&mut canvas, |_, _, top, _| Some(('#', top)));
        canvas.set_mode(GlyphMode::Ascii);
        px.blit(&mut canvas, |_, _, top, bottom| {
            (top.level > bottom.level).then_some(('^', top))
        });
        px.clear();
    });
    assert_free("Pixels set/fill/get/blit/clear", stats);
    black_box(canvas.cells());
}

#[test]
fn core_r43_resampling_does_not_allocate() {
    let src = Grid::new(Size::new(120, 60), 0.5f32);
    let mut up = Grid::new(Size::new(200, 100), 0.0f32);
    let mut down = Grid::new(Size::new(37, 11), 0.0f32);
    let spins = Grid::new(Size::new(64, 64), -1i8);
    let mut spins_dst = Grid::new(Size::new(100, 30), 0i8);
    let ((), stats) = measure(|| {
        grid::resample_bilinear(&src, &mut up);
        grid::resample_bilinear(&src, &mut down);
        grid::resample_nearest(&spins, &mut spins_dst);
    });
    assert_free("resample_bilinear / resample_nearest", stats);
    black_box((up.as_slice(), down.as_slice(), spins_dst.as_slice()));
}

#[test]
fn core_r44_verlet_integrate_does_not_allocate() {
    // A small N-body-like system: 16 bodies in 2-D, pairwise softened gravity.
    let mut pos: Vec<f64> = (0..32).map(|i| f64::from(i).sin() * 5.0).collect();
    let mut vel = vec![0.0; 32];
    let mut acc = vec![0.0; 32];
    let gravity = |p: &[f64], a: &mut [f64]| {
        a.fill(0.0);
        for i in 0..16 {
            for j in 0..16 {
                if i != j {
                    let (dx, dy) = (p[2 * j] - p[2 * i], p[2 * j + 1] - p[2 * i + 1]);
                    let r2 = dx * dx + dy * dy + 0.1;
                    let k = 1.0 / (r2 * r2.sqrt());
                    a[2 * i] += dx * k;
                    a[2 * i + 1] += dy * k;
                }
            }
        }
    };
    gravity(&pos, &mut acc);
    let (n, stats) =
        measure(|| ode::verlet_integrate(&mut pos, &mut vel, &mut acc, 0.5, 0.01, gravity));
    assert_eq!(n, 50);
    assert_free("verlet_integrate", stats);
    black_box(pos);
}

#[test]
fn core_r49_stack_text_and_text_mode_do_not_allocate() {
    let mut canvas = Canvas::new(Size::new(80, 24), GlyphMode::Unicode);
    let ink = Ink::new(Role::Text, 150);
    let (end, stats) = measure(|| {
        let mut buf = StackText::<48>::new();
        let _ = write!(buf, "T/Tc {:.2}  N {}  λ₁ {:+.3e}", 2.269_18, 4096, -0.906);
        let _ = write!(buf, " and a tail far too long to fit in forty-eight bytes");
        let x = draw::text_mode(&mut canvas, 0, 0, buf.as_str(), "ascii", ink);
        buf.clear();
        let _ = write!(buf, "{}", black_box(7u8));
        draw::text_mode(&mut canvas, x, 1, "ψ²", buf.as_str(), ink)
    });
    assert_free("StackText write! / text_mode", stats);
    black_box((end, canvas.cells()));
}

#[test]
fn core_r50_projector_does_not_allocate() {
    let cam = Camera {
        yaw: 0.4,
        pitch: -0.3,
        distance: 6.0,
    };
    let (sum, stats) = measure(|| {
        let pr = cam.projector();
        (0..2000)
            .filter_map(|i| {
                let t = f64::from(i) * 0.01;
                pr.project([t.cos(), t.sin(), 0.1 * t], (40.0, 12.0), 8.0)
            })
            .map(|(x, y, z)| x + y + z + pr.rotate([1.0, 2.0, 3.0])[2])
            .sum::<f64>()
    });
    assert_free("Projector::project / rotate", stats);
    black_box(sum);
}

#[test]
fn core_r51_r55_dots_line_and_density_do_not_allocate() {
    let size = Size::new(200, 50);
    let mut dots = Dots::new(size);
    let mut density = Density::new(size);
    let ink = Ink::new(Role::Wave, 180);
    let ((), stats) = measure(|| {
        for i in 0..500 {
            let t = f64::from(i) * 0.013;
            let (x0, y0, x1, y1) = (
                200.0 + 150.0 * t.cos(),
                100.0,
                200.0 + 300.0 * t.sin(),
                1e9 * t,
            );
            dots.line(x0, y0, x1, y1, ink);
            density.deposit(x0, y0, 0.5);
            density.deposit(x1, y1, 0.25);
        }
        density.decay(0.9);
        density.to_dots(&mut dots, 0.3, |v| ink::seq(f64::from(v)));
        black_box(density.get(10, 10));
        density.clear();
        density.resize(Size::new(100, 20)); // shrinking reuses the buffer
        dots.clear();
    });
    assert_free("Dots::line / Density", stats);
}

#[test]
fn core_r52_sample_map_does_not_allocate_after_rebuild() {
    let src = Grid::new(Size::new(120, 60), 0.5f32);
    let mut map = SampleMap::default();
    map.rebuild(src.size(), Size::new(200, 100));
    let (sum, stats) = measure(|| {
        let mut sum = 0.0f32;
        for y in 0..100 {
            for x in 0..200 {
                sum += map.sample(&src, x, y);
                sum += map.nearest(x, y).0 as f32;
            }
        }
        // Rebuilding within the existing tables' capacity reuses them.
        map.rebuild(src.size(), Size::new(150, 40));
        sum
    });
    assert_free("SampleMap::sample / nearest / rebuild (not growing)", stats);
    black_box(sum);
}

#[test]
fn core_r53_laplacian9_with_presized_dst_does_not_allocate() {
    let src = Grid::new(Size::new(200, 100), 0.25f32);
    let mut dst = Grid::new(Size::new(200, 100), 0.0f32);
    let ((), stats) = measure(|| {
        for _ in 0..3 {
            grid::laplacian9(&src, &mut dst, Edges::Wrap);
            grid::laplacian9(&src, &mut dst, Edges::Clip);
        }
    });
    assert_free("laplacian9", stats);
    black_box(dst.as_slice());
}

#[test]
fn core_r54_spatial_hash_does_not_allocate_after_first_rebuild() {
    let mut pts: Vec<[f64; 2]> = (0..500)
        .map(|i| {
            let t = f64::from(i);
            [(t * 7.3) % 200.0, (t * 3.1) % 100.0]
        })
        .collect();
    let mut hash = SpatialHash::new(200.0, 100.0, 5.0);
    hash.rebuild(&pts);
    let (n, stats) = measure(|| {
        let mut n = 0usize;
        for frame in 0..5 {
            for p in &mut pts {
                p[0] = (p[0] + 1.3 + f64::from(frame)) % 200.0;
            }
            hash.rebuild(&pts);
            for p in &pts {
                hash.for_each_within(*p, 5.0, |i| n += i & 1);
            }
        }
        hash.rebuild(&pts[..100]); // fewer points reuse the buffers
        n
    });
    assert_free("SpatialHash::rebuild / for_each_within", stats);
    black_box(n);
}

#[test]
fn core_r56_r57_r58_r59_scalar_helpers_do_not_allocate() {
    let (sum, stats) = measure(|| {
        let mut rng = SplitMix64::new(42);
        let icdf = InverseCdf::<128>::build(0.0, 10.0, |r| r * r * (-r).exp());
        let mut sum = 0.0;
        for _ in 0..1000 {
            let u = rng.next_f64();
            sum += icdf.sample(u) + ink::soft_clip(u * 8.0 - 4.0);
            sum += f64::from(rng.below(17));
        }
        sum += ink::seq_value(ink::seq(0.3)) + ink::div_value(ink::div(-0.4));
        let mut rhs = [1.0; 64];
        let mut scratch = [0.0; 64];
        let ok = ode::tridiag_solve(&[-1.0; 64], &[4.0; 64], &[-1.0; 64], &mut rhs, &mut scratch);
        sum + rhs[0] + f64::from(u8::from(ok))
    });
    assert_free("soft_clip / SplitMix64 / InverseCdf / tridiag_solve", stats);
    black_box(sum);
}

#[test]
fn core_r62_r64_r65_r67_r68_audit_helpers_do_not_allocate() {
    let mut canvas = Canvas::new(Size::new(80, 24), GlyphMode::Unicode);
    let mut dots = Dots::new(Size::new(80, 24));
    let mut pixels = Pixels::new(Size::new(80, 24));
    let mut field = Grid::new(Size::new(60, 40), 0.5f32);
    let mut scratch = Grid::new(Size::new(60, 40), 0.0f32);
    let mut spins = Grid::new(Size::new(60, 40), 1i8);
    let mut spin_scratch = Grid::new(Size::new(60, 40), 1i8);
    let (sum, stats) = measure(|| {
        let mut sum = 0.0;
        geom::stroked_line(0, 0, 70, 20, |k, x, y, (dx, dy)| {
            canvas.put(
                x,
                y,
                draw::step_glyph(dx, dy),
                Ink::new(Role::Wave, k as u8),
            );
        });
        let end = draw::label(&mut canvas, 1, 23, "T/Tc 1.25 λ", Ink::new(Role::Text, 200));
        for i in 0..200 {
            let (x, y) = Dots::from_cells(f64::from(i) * 0.4, f64::from(i % 24));
            dots.plot(x, y, Ink::new(Role::Star, ink::level(f64::from(i))));
            sum += f64::from(dots.cell_mask(i % 80, i % 24).unwrap_or(0));
            sum += rng::hash01(rng::cell_key(i, -i)) + geom::wrap_angle(f64::from(i));
            sum += f64::from(u8::from(draw::disc_covers(40.0, 12.0, 5.0, i % 80, i % 24)));
        }
        pixels.shade(|x, y| ink::seq(f64::from(x + y) / 200.0));
        // Shrinking regrids reuse the scratch buffers.
        grid::regrid_bilinear(&mut field, Size::new(50, 30), &mut scratch);
        grid::regrid_nearest(&mut spins, Size::new(50, 30), &mut spin_scratch);
        sum + f64::from(end)
    });
    assert_free(
        "stroked_line / label / from_cells / cell_mask / shade / regrid",
        stats,
    );
    black_box((sum, field.len(), spins.len()));
}
