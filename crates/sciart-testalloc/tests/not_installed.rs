//! Without `#[global_allocator]` a zero count would be vacuous, so `measure` refuses
//! to run (ADR-0009). This binary deliberately does NOT install `CountingAlloc`.

#[test]
#[should_panic(expected = "is not the global allocator")]
fn measure_panics_when_counting_alloc_is_not_installed() {
    let _ = sciart_testalloc::measure(|| ());
}

#[test]
fn is_installed_is_false_without_global_allocator() {
    assert!(!sciart_testalloc::is_installed());
}
