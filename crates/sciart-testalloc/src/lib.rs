//! Test-only counting global allocator (ADR-0009).
//!
//! Lets allocation tests assert "this code performs zero heap allocations" without
//! relaxing `unsafe_code = "forbid"` for product crates: the one `unsafe impl
//! GlobalAlloc` in the workspace lives here, in a `publish = false` crate that is only
//! ever a **dev-dependency**.
//!
//! This crate does not install anything. A test binary opts in explicitly, and must
//! be its own integration-test file because a binary can have only one global
//! allocator:
//!
//! ```ignore
//! use sciart_testalloc::{CountingAlloc, measure};
//!
//! #[global_allocator]
//! static GLOBAL: CountingAlloc = CountingAlloc;
//!
//! #[test]
//! fn no_alloc() {
//!     let (_, stats) = measure(|| 1 + 1);
//!     assert_eq!(stats.total(), 0);
//! }
//! ```
//!
//! Counters are **per thread**. libtest runs tests on parallel threads, and a process-wide
//! counter would see other tests' allocations; a thread-local one sees only the code
//! under test (plus anything *it* does on this thread). Work spawned onto other threads
//! is not counted, which is fine for the synchronous code measured here.
//!
//! Besides the call counters, [`live_bytes`] tracks the net heap footprint of the thread
//! for the leak guards (TEST-R6, TEST-R7).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

/// Heap operations observed on the current thread (see [`measure`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Allocs {
    /// `alloc` + `alloc_zeroed` calls.
    pub allocs: u64,
    /// `realloc` calls (a `Vec` growing in place still counts: it is a heap call).
    pub reallocs: u64,
    /// `dealloc` calls.
    pub deallocs: u64,
    /// Bytes requested by `alloc`/`alloc_zeroed`, plus the new size of each `realloc`.
    pub bytes: u64,
}

impl Allocs {
    /// Calls that acquire memory: `allocs + reallocs`. This is what "does not
    /// allocate" means in SPEC-001/SPEC-002; frees alone are not counted as allocations.
    pub const fn total(&self) -> u64 {
        self.allocs + self.reallocs
    }

    fn since(self, before: Self) -> Self {
        Self {
            allocs: self.allocs - before.allocs,
            reallocs: self.reallocs - before.reallocs,
            deallocs: self.deallocs - before.deallocs,
            bytes: self.bytes - before.bytes,
        }
    }
}

// `const` initialisers and `Copy` payloads: no lazy init and no TLS destructor, so
// touching these from inside the allocator can neither allocate (recursion) nor
// register a destructor. `try_with` tolerates access during thread teardown.
thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static REALLOCS: Cell<u64> = const { Cell::new(0) };
    static DEALLOCS: Cell<u64> = const { Cell::new(0) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
    // Signed: a thread may free blocks another thread allocated.
    static LIVE: Cell<i64> = const { Cell::new(0) };
}

/// Set on the first allocation routed through [`CountingAlloc`], so [`measure`] can
/// refuse to report a vacuous zero when the test binary forgot `#[global_allocator]`.
static INSTALLED: AtomicBool = AtomicBool::new(false);

#[inline]
fn bump(counter: &'static std::thread::LocalKey<Cell<u64>>, by: u64) {
    let _ = counter.try_with(|c| c.set(c.get().wrapping_add(by)));
}

/// Add `by` (possibly negative) to this thread's live-byte count.
#[inline]
fn live_add(by: i64) {
    let _ = LIVE.try_with(|c| c.set(c.get().wrapping_add(by)));
}

/// Count an allocation of `size` bytes that returned `p`. The attempt always counts; the
/// bytes are live only when it succeeded, since a null return allocated nothing.
#[inline]
fn record_alloc(size: usize, p: *mut u8) {
    INSTALLED.store(true, Ordering::Relaxed);
    bump(&ALLOCS, 1);
    bump(&BYTES, size as u64);
    if !p.is_null() {
        live_add(size as i64);
    }
}

fn snapshot() -> Allocs {
    let read = |k: &'static std::thread::LocalKey<Cell<u64>>| k.with(Cell::get);
    Allocs {
        allocs: read(&ALLOCS),
        reallocs: read(&REALLOCS),
        deallocs: read(&DEALLOCS),
        bytes: read(&BYTES),
    }
}

/// [`System`] plus per-thread counters. Install it with `#[global_allocator]` in a
/// dedicated integration-test binary; never in a library or the product binary.
#[derive(Debug, Clone, Copy, Default)]
pub struct CountingAlloc;

// ADR-0009: the workspace's only `unsafe` code. Every method forwards unchanged to
// `System` and only adds counter bumps, which neither allocate nor unwind.
#[allow(unsafe_code)]
// SAFETY: `GlobalAlloc`'s contract is met by delegation: each method passes its
// arguments unmodified to the corresponding `System` method (itself a sound
// `GlobalAlloc`) and returns its result unmodified, so every pointer, layout and
// alignment guarantee is exactly `System`'s. The added bookkeeping touches only
// `const`-initialised thread-locals of `Cell<u64>` and one atomic, so it cannot
// allocate (no re-entrancy), panic, or unwind across the allocator boundary.
unsafe impl GlobalAlloc for CountingAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s preconditions (non-zero
        // size layout), which are exactly `System::alloc`'s.
        let p = unsafe { System.alloc(layout) };
        record_alloc(layout.size(), p);
        p
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: same preconditions as `alloc`, forwarded unchanged.
        let p = unsafe { System.alloc_zeroed(layout) };
        record_alloc(layout.size(), p);
        p
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        bump(&DEALLOCS, 1);
        live_add(-(layout.size() as i64));
        // SAFETY: the caller guarantees `ptr` came from this allocator with `layout`;
        // every block this allocator hands out came from `System`, so that holds there.
        unsafe { System.dealloc(ptr, layout) }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        INSTALLED.store(true, Ordering::Relaxed);
        bump(&REALLOCS, 1);
        bump(&BYTES, new_size as u64);
        // SAFETY: the caller guarantees `ptr`/`layout` describe a live block from this
        // allocator (hence from `System`) and that `new_size` is valid for `layout`.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        // On failure the old block stays live at its old size.
        if !p.is_null() {
            live_add(new_size as i64 - layout.size() as i64);
        }
        p
    }
}

/// Run `f` on the current thread and return its result with the heap operations it
/// performed on this thread.
///
/// The result is returned (not dropped inside the window) so that freeing it is not
/// attributed to `f`; drop it after inspecting the counts.
///
/// # Panics
/// If [`CountingAlloc`] is not the global allocator of this test binary: a zero
/// count would then be meaningless.
pub fn measure<R>(f: impl FnOnce() -> R) -> (R, Allocs) {
    assert_installed();
    let before = snapshot();
    let out = f();
    let after = snapshot();
    (out, after.since(before))
}

/// Bytes allocated minus bytes freed on the current thread since it started, with a
/// `realloc` counted as its size delta (TEST-R6). Only differences are meaningful: take
/// one reading before and one after the code under test. Negative when the thread has
/// freed more than it allocated (blocks handed over from other threads).
///
/// # Panics
/// If [`CountingAlloc`] is not the global allocator of this test binary: the count would
/// then stay at zero and hide every leak.
pub fn live_bytes() -> i64 {
    assert_installed();
    LIVE.with(Cell::get)
}

/// Whether [`CountingAlloc`] is this binary's global allocator.
pub fn is_installed() -> bool {
    // One real allocation to prove the routing; `black_box` stops it being elided.
    drop(std::hint::black_box(Box::new(0u64)));
    INSTALLED.load(Ordering::Relaxed)
}

fn assert_installed() {
    assert!(
        is_installed(),
        "sciart_testalloc::CountingAlloc is not the global allocator: add \
         `#[global_allocator] static GLOBAL: CountingAlloc = CountingAlloc;` to this test binary"
    );
}
