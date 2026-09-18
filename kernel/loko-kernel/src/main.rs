//! # The LokoOS kernel
//!
//! Boot stages 5 through 9 of the sequence in requirement 6: the kernel takes
//! control from the bootloader, checks what it was handed, brings up the CPU's
//! descriptor tables, takes ownership of physical memory, and starts a heap.
//!
//! ## What this kernel does today
//!
//! Serial diagnostics, boot-info validation, GDT and TSS with dedicated fault
//! stacks, IDT with exception handlers, a bitmap physical frame allocator built
//! from the firmware memory map, and a kernel heap. It then reports what it
//! found and halts.
//!
//! **It does not yet schedule, page, or run userland.** There is no scheduler,
//! no address-space manager, and no storage stack, so there is nothing to hand
//! control to. The kernel says so on the serial console rather than pretending
//! otherwise. `documentation/STATUS.md` tracks what is next.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

mod arch;
mod framebuffer;
mod heap;
mod log;
mod serial;
mod syscall;

use core::panic::PanicInfo;
use core::ptr::addr_of_mut;

use loko_boot_protocol::{BootInfo, MemoryKind};
use loko_memory::{bitmap_words_for, FrameAllocator, FrameNumber, FRAME_SIZE};

use crate::log::Level;

/// The kernel's own version, shown in Settings and stamped into logs.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The most physical memory this build can manage.
///
/// The frame allocator's bitmap has to exist before there is anything to
/// allocate it from, so it is a static. 16 GiB of RAM needs 512 KiB of bitmap,
/// which is BSS and costs nothing on disk.
///
/// A machine with more memory than this still boots; the kernel reports the
/// excess as unmanaged rather than silently ignoring it. Replacing this with a
/// bitmap carved out of the memory map itself is tracked in
/// `documentation/STATUS.md`.
pub const MAX_MANAGED_MEMORY: u64 = 16 * 1024 * 1024 * 1024;

const BITMAP_WORDS: usize = bitmap_words_for(MAX_MANAGED_MEMORY / FRAME_SIZE);

static mut FRAME_BITMAP: [u64; BITMAP_WORDS] = [0; BITMAP_WORDS];

/// The kernel entry point.
///
/// Called by the LokoOS bootloader with a pointer to a [`BootInfo`] in `rdi`,
/// per the SysV C ABI. Nothing about the pointed-to structure is trusted until
/// it has been validated.
///
/// # Safety
///
/// `boot_info` must be either null or a pointer to a live `BootInfo` whose
/// memory map pointer and length are readable.
#[no_mangle]
pub unsafe extern "C" fn _start(boot_info: *const BootInfo) -> ! {
    serial::init();
    log::set_level(Level::Info);

    info!("boot", "LokoOS kernel {VERSION}");
    info!("boot", "stage 5: kernel entered");

    if boot_info.is_null() {
        fail("LokoOS could not start: the bootloader did not describe this device.");
    }

    // SAFETY: the pointer is non-null and, by this function's contract, points
    // to a live BootInfo. `validate` checks everything about its contents.
    let info = unsafe { &*boot_info };

    // SAFETY: same contract; the memory map pointer and length are readable.
    if let Err(error) = unsafe { info.validate() } {
        error!("boot", "boot info rejected: {error:?}");
        fail(error.message());
    }

    if info.flags.contains(loko_boot_protocol::BootFlags::VERBOSE) {
        log::set_level(Level::Trace);
    }

    info!(
        "boot",
        "protocol {}.{}, {} memory regions, physical window at {:#x}",
        info.version_major,
        info.version_minor,
        info.memory_map_len,
        info.physical_memory_offset
    );
    if info
        .flags
        .contains(loko_boot_protocol::BootFlags::SECURE_BOOT)
    {
        info!("security", "Secure Boot verified this kernel image");
    } else {
        warn!(
            "security",
            "Secure Boot is off; this image was not verified"
        );
    }

    info!("boot", "stage 6: descriptor tables");
    arch::init();

    if info.framebuffer.is_present() {
        info!(
            "graphics",
            "framebuffer {}x{} at {:#x}",
            info.framebuffer.width,
            info.framebuffer.height,
            info.framebuffer.base
        );
        // `BootInfo` carries the framebuffer at its physical address, because
        // that is what the frame allocator has to reserve. Drawing needs the
        // address in the physical-memory window instead: the identity map that
        // makes the raw address work is temporary, and code that depends on it
        // would break the moment the address-space manager tears it down.
        let mut surface = info.framebuffer;
        surface.base += info.physical_memory_offset;
        // SAFETY: the bootloader maps the framebuffer into the physical-memory
        // window before handing over, and `validate` has confirmed the
        // structure describing it is well formed.
        unsafe { framebuffer::init(surface) };
    } else {
        info!("graphics", "no framebuffer; serial only");
    }

    info!("boot", "stage 7: physical memory");
    let mut frames = setup_memory(info);

    info!("boot", "stage 8: kernel heap");
    setup_heap(&mut frames, info.physical_memory_offset);

    report(&frames);

    // Stage 9 onward — scheduler, address spaces, storage, services — does not
    // exist yet. Saying so plainly is the whole point of rule 69: there is
    // nothing here pretending to be a running system.
    info!(
        "boot",
        "stage 9: not implemented; no scheduler or storage stack yet"
    );
    info!("boot", "kernel initialised successfully and is halting");
    arch::halt_forever();
}

/// Builds the frame allocator from the firmware memory map and reserves
/// everything the kernel must not hand out.
fn setup_memory(info: &BootInfo) -> FrameAllocator<'static> {
    // SAFETY: `validate` confirmed the pointer and length describe a readable,
    // correctly aligned array of regions.
    let regions =
        unsafe { core::slice::from_raw_parts(info.memory_map, info.memory_map_len as usize) };

    let highest = regions.iter().map(|r| r.end()).max().unwrap_or(0);

    // SAFETY: single-threaded early boot, before any other CPU is started, and
    // this is the only code that ever takes a reference to the bitmap.
    let bitmap: &'static mut [u64] = unsafe { &mut *addr_of_mut!(FRAME_BITMAP) };

    let mut frames = match FrameAllocator::from_memory_map(bitmap, regions) {
        Ok(f) => f,
        Err(e) => {
            error!("memory", "frame allocator refused the memory map: {e:?}");
            fail("LokoOS could not start: this device's memory could not be set up.");
        }
    };

    // The allocator manages what its bitmap can describe, which may be less
    // than the memory map reaches. Most of the difference is MMIO rather than
    // RAM, so this is reported rather than warned about, and only when it
    // actually exceeds what this build supports.
    let managed = frames.total_frames() * FRAME_SIZE;
    if highest > managed && managed >= MAX_MANAGED_MEMORY {
        warn!(
            "memory",
            "this device reports addresses up to {} GiB; this build manages the first {} GiB",
            highest / (1024 * 1024 * 1024),
            managed / (1024 * 1024 * 1024)
        );
    }

    // The kernel image, the boot info, and the framebuffer are in use right
    // now. Handing any of them out would corrupt the running system.
    frames.reserve_range(info.kernel_physical_base, info.kernel_size);
    if info.framebuffer.is_present() {
        frames.reserve_range(info.framebuffer.base, info.framebuffer.size);
    }
    for region in regions {
        if region.kind == MemoryKind::BootloaderReclaimable {
            frames.reserve_range(region.start, region.len);
        }
    }

    // The first frame is never handed out. A null pointer must stay
    // distinguishable from a valid allocation at physical address zero.
    let _ = frames.reserve(FrameNumber(0));

    if !frames.audit() {
        error!("memory", "frame allocator failed its own consistency check");
        fail("LokoOS could not start: this device's memory could not be set up.");
    }

    frames
}

/// Allocates and initialises the kernel heap.
fn setup_heap(frames: &mut FrameAllocator<'static>, physical_memory_offset: u64) {
    let needed = (heap::HEAP_SIZE as u64).div_ceil(FRAME_SIZE);
    let first = match frames.allocate_contiguous(needed) {
        Ok(f) => f,
        Err(e) => {
            error!(
                "heap",
                "could not reserve {needed} frames for the heap: {e:?}"
            );
            fail("LokoOS could not start: there isn't enough memory in this device.");
        }
    };

    // The bootloader mapped all of physical memory at this offset, so the
    // frames just allocated are already reachable without touching page tables.
    let virt = physical_memory_offset + first.start_address();

    // SAFETY: these frames were just allocated and are owned by nobody else,
    // and the physical window is mapped writable by the bootloader.
    unsafe { heap::init(virt as usize, heap::HEAP_SIZE) };

    info!("heap", "{} KiB at {:#x}", heap::HEAP_SIZE / 1024, virt);
}

/// Reports what the kernel found, as a real measurement rather than a
/// hard-coded figure.
fn report(frames: &FrameAllocator<'_>) {
    let total_mib = frames.total_frames() * FRAME_SIZE / (1024 * 1024);
    let free_mib = frames.free_bytes() / (1024 * 1024);
    info!(
        "memory",
        "{} MiB addressable, {} MiB free, {} KiB heap in use",
        total_mib,
        free_mib,
        heap::used() / 1024
    );
}

/// Stops the machine with a message, for failures that happen before there is
/// any recovery path.
fn fail(message: &str) -> ! {
    error!("boot", "{message}");
    panic_screen(message)
}

/// Paints the stop screen, writes the message to the serial log, and halts.
///
/// Public because the exception handlers and the allocation-failure handler all
/// end here.
pub fn panic_screen(message: &str) -> ! {
    // SAFETY: we are stopping. No other context will make progress, so taking
    // the serial port without its lock cannot race — and *not* bypassing the
    // lock would hang if we panicked while holding it.
    unsafe {
        serial::write_fmt_unlocked(format_args!("\n*** LokoOS stopped ***\n{message}\n"));
    }
    framebuffer::stop_screen(message);
    arch::halt_forever();
}

#[panic_handler]
fn on_panic(info: &PanicInfo<'_>) -> ! {
    // SAFETY: as in `panic_screen` — this is the end of the line.
    unsafe {
        serial::write_fmt_unlocked(format_args!("\n*** LokoOS kernel panic ***\n{info}\n"));
    }
    framebuffer::stop_screen("LokoOS ran into a problem and had to stop.");
    arch::halt_forever();
}
