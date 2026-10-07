//! Test-only allocation probe (#776): how many bytes a piece of code holds at its peak, and how many
//! it still holds when it finishes.
//!
//! It is the unit-test binary's global allocator. It wraps `System`, and counts only on a thread that
//! has switched counting on, so every other test pays one thread-local read per allocation. Counts are
//! per thread, so [`measure`] needs a current-thread runtime, which `#[sqlx::test]` provides, to see
//! every allocation the future makes.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct Probe;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn note(delta: isize) {
    // `try_with`: never panic inside the allocator, even while a thread is being torn down.
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            let live = LIVE.with(|l| {
                l.set(l.get() + delta);
                l.get()
            });
            PEAK.with(|p| p.set(p.get().max(live)));
        }
    });
}

// SAFETY: every call is forwarded unchanged to `System`; the bookkeeping allocates nothing.
unsafe impl GlobalAlloc for Probe {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            note(layout.size() as isize);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            note(layout.size() as isize);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        note(-(layout.size() as isize));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(ptr, layout, new_size) };
        if !moved.is_null() {
            note(new_size as isize - layout.size() as isize);
        }
        moved
    }
}

#[global_allocator]
static PROBE: Probe = Probe;

/// What a future allocated on this thread: its result, the most it held at once, and what it still
/// held when it finished — both in bytes, relative to when it started.
pub(crate) async fn measure<F: Future>(future: F) -> (F::Output, usize, usize) {
    assert_eq!(
        tokio::runtime::Handle::current().runtime_flavor(),
        tokio::runtime::RuntimeFlavor::CurrentThread,
        "the probe counts one thread, so the future must not hop to another"
    );
    LIVE.with(|l| l.set(0));
    PEAK.with(|p| p.set(0));
    COUNTING.with(|c| c.set(true));
    let output = future.await;
    COUNTING.with(|c| c.set(false));
    let held = LIVE.with(Cell::get).max(0) as usize;
    let peak = PEAK.with(Cell::get).max(0) as usize;
    (output, peak, held)
}
