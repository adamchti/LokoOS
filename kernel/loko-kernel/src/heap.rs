//! The kernel heap.
//!
//! Small and fixed-size on purpose. Almost nothing in the LokoOS kernel should
//! need dynamic allocation: the frame allocator uses a bitmap it is handed, the
//! handle tables are fixed arrays, and the interesting data structures live in
//! userland services. The heap exists for the few places that genuinely need
//! it, and its size is a budget rather than a floor to grow from.

use core::alloc::Layout;
use linked_list_allocator::LockedHeap;

/// 1 MiB. If this ever needs raising, the right question is which subsystem
/// started allocating and whether it should have.
pub const HEAP_SIZE: usize = 1024 * 1024;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Brings the heap up over an already-mapped region.
///
/// LokoOS does not build a dedicated virtual mapping for the heap. The
/// bootloader has already mapped all of physical memory at a known offset, so
/// the kernel allocates physically contiguous frames and uses their addresses
/// in that window directly. That removes page-table manipulation from the
/// earliest and least debuggable part of boot, at the cost of the heap being
/// wherever the frame allocator put it — which nothing depends on.
///
/// # Safety
///
/// `[start, start + size)` must be mapped, writable, and owned by nobody else.
/// Calling this twice corrupts the allocator.
pub unsafe fn init(start: usize, size: usize) {
    // SAFETY: guaranteed by the caller.
    unsafe {
        ALLOCATOR.lock().init(start as *mut u8, size);
    }
}

/// Bytes currently in use.
#[must_use]
pub fn used() -> usize {
    ALLOCATOR.lock().used()
}

/// Bytes still available.
#[must_use]
pub fn free() -> usize {
    ALLOCATOR.lock().free()
}

/// Called when an allocation cannot be satisfied.
///
/// A kernel that cannot allocate cannot reliably do anything else, including
/// report the problem through any path that itself allocates. So this reports
/// through the serial port, which does not, and stops.
#[alloc_error_handler]
fn on_allocation_failure(layout: Layout) -> ! {
    crate::error!(
        "heap",
        "allocation of {} bytes (align {}) failed; {} of {} bytes free",
        layout.size(),
        layout.align(),
        free(),
        HEAP_SIZE
    );
    crate::panic_screen("LokoOS ran out of memory and had to stop.")
}
