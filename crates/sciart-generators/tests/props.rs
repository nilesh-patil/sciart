//! Property tests: random op sequences never break the contract (GEN-R4, TEST-R4).

use proptest::prelude::*;
use sciart_core::prelude::*;
use sciart_generators::registry;

#[derive(Debug, Clone)]
enum Op {
    Resize(u16, u16),
    Step(f64),
    Render,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        1 => (0u16..=260, 0u16..=80).prop_map(|(w, h)| Op::Resize(w, h)),
        4 => (0.0f64..=0.25).prop_map(Op::Step),
        2 => Just(Op::Render),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn gen_r4_random_op_sequences_hold_the_contract(
        index in 0..registry().len(),
        seed in any::<u64>(),
        (w, h) in (0u16..=260, 0u16..=80),
        ops in proptest::collection::vec(op(), 1..60),
    ) {
        let entry = &registry()[index];
        let mut stage = Stage::new((entry.build)(), seed, Size::new(w, h), GlyphMode::Unicode);
        for op in ops {
            match op {
                Op::Resize(w, h) => stage.resize(Size::new(w, h)),
                Op::Step(dt) => stage.step(dt),
                Op::Render => {
                    let size = stage.ctx().size();
                    let frame = stage.render();
                    prop_assert_eq!(frame.size(), size);
                    prop_assert_eq!(frame.cells().len(), size.area());
                }
            }
        }
    }
}
