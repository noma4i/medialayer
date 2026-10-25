use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static COUNT: Cell<usize> = const { Cell::new(0) };
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
    static MARK: Cell<isize> = const { Cell::new(0) };
}

fn grow(bytes: isize) {
    let _ = LIVE.try_with(|live| {
        live.set(live.get() + bytes);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(live.get())));
    });
}

pub struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = COUNT.try_with(|count| count.set(count.get() + 1));
        grow(layout.size() as isize);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        grow(-(layout.size() as isize));
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = COUNT.try_with(|count| count.set(count.get() + 1));
        grow(new_size as isize - layout.size() as isize);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

pub fn allocations_of<T>(work: impl FnOnce() -> T) -> usize {
    let before = COUNT.with(Cell::get);
    let result = work();
    let used = COUNT.with(Cell::get) - before;
    drop(result);
    used
}

pub fn mark_peak() {
    let live = LIVE.with(Cell::get);
    MARK.with(|mark| mark.set(live));
    PEAK.with(|peak| peak.set(live));
}

pub fn peak_since_mark() -> usize {
    (PEAK.with(Cell::get) - MARK.with(Cell::get)).max(0) as usize
}
