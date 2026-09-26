//! The counter itself must be trustworthy before any "zero allocations" claim built on
//! it is (ADR-0009): it sees allocations, reallocations and frees on this thread, and
//! nothing from other threads.

use std::hint::black_box;

use sciart_testalloc::{Allocs, CountingAlloc, measure};

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn counts_nothing_for_pure_arithmetic() {
    let (sum, stats) = measure(|| (1..=black_box(100u64)).sum::<u64>());
    assert_eq!(sum, 5050);
    assert_eq!(stats, Allocs::default());
}

#[test]
fn counts_an_allocation_and_its_bytes() {
    let (v, stats) = measure(|| Vec::<u64>::with_capacity(black_box(16)));
    assert_eq!(stats.allocs, 1);
    assert_eq!(stats.bytes, 16 * 8);
    assert_eq!(stats.total(), 1);
    assert_eq!(
        stats.deallocs, 0,
        "the returned value is dropped outside the window"
    );
    drop(v);
}

#[test]
fn counts_growth_as_realloc_and_drop_as_dealloc() {
    let mut v = Vec::<u8>::with_capacity(1);
    v.push(1);
    let ((), stats) = measure(|| v.extend_from_slice(black_box(&[0; 64])));
    assert_eq!((stats.allocs, stats.reallocs), (0, 1));
    let ((), stats) = measure(|| drop(std::mem::take(&mut v)));
    assert_eq!((stats.total(), stats.deallocs), (0, 1));
}

#[test]
fn push_within_capacity_is_free() {
    let mut v = Vec::<u32>::with_capacity(8);
    let ((), stats) = measure(|| {
        for i in 0..8 {
            v.push(black_box(i));
        }
    });
    assert_eq!(stats.total(), 0);
}

#[test]
fn ignores_allocations_on_other_threads() {
    let (handle, _) = measure(|| {
        std::thread::spawn(|| {
            // Allocate a lot on the other thread while this one is being measured.
            let v: Vec<Vec<u8>> = (0..1000).map(|i| vec![0; i]).collect();
            black_box(v.len())
        })
    });
    let n = handle.join().expect("worker");
    assert_eq!(n, 1000);
    // Only the local window matters: an empty closure must read zero even while
    // other test threads allocate concurrently.
    let ((), stats) = measure(|| {});
    assert_eq!(stats, Allocs::default());
}

#[test]
fn reports_installed() {
    assert!(sciart_testalloc::is_installed());
}
