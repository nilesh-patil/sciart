//! GEN-R8: one simulated step + one render per generator must stay < 5 ms at 200x50.
//! Run with `pixi run rs-bench`.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use sciart_core::prelude::*;
use sciart_generators::registry;

fn frame(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame");
    for entry in registry() {
        for (w, h) in [(200u16, 50u16), (400, 100)] {
            let mut stage = Stage::new((entry.build)(), 42, Size::new(w, h), GlyphMode::Unicode);
            // Warm the scene past its initial state.
            for _ in 0..120 {
                stage.step(SIM_DT);
            }
            let id = BenchmarkId::new(entry.meta.name, format!("{w}x{h}"));
            group.bench_function(id, |b| {
                b.iter(|| {
                    stage.step(SIM_DT);
                    std::hint::black_box(stage.render());
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, frame);
criterion_main!(benches);
