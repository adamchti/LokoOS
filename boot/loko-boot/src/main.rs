//! # The LokoOS bootloader
//!
//! Stages 1 through 4 of the boot sequence in requirement 6: a UEFI application
//! that finds the kernel, loads it, describes the machine, and hands over.
//!
//! The sequence, and why it is in this order:
//!
//! 1. Read the kernel image off the EFI system partition.
//! 2. Parse and validate it. A malformed image is rejected here, where there is
//!    still a console to say so on.
//! 3. Reserve every piece of memory the handover needs — the kernel image, the
//!    page-table arena, the kernel stack, the memory-map array, the boot info —
//!    **while an allocator still exists**. After `ExitBootServices` there is no
//!    way to obtain another byte.
//! 4. Collect what only firmware knows: the framebuffer and the ACPI pointer.
//! 5. Build the page tables.
//! 6. Exit boot services, convert the final memory map, and jump.
//!
//! ## Status
//!
//! **Works.** Verified booting under QEMU with OVMF on every push: firmware
//! loads this image, it loads the kernel, and the kernel comes up on the other
//! side. See `documentation/STATUS.md` for the serial log.
//!
//! Untested on real hardware, and untested on any machine other than the one
//! CI emulates.

#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

mod elf;
mod paging;

use core::ptr::NonNull;

use loko_boot_protocol::{
    BootFlags, BootInfo, Framebuffer, MemoryKind, MemoryRegion, PixelFormat, BOOT_MAGIC,
    DEFAULT_PHYSICAL_MEMORY_OFFSET, PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR,
};
use uefi::boot::{self, AllocateType, MemoryType};
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat as GopPixelFormat};
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, FileType};

use paging::{PageTables, Permissions, ARENA_FRAMES, PAGE_SIZE};

/// Where the kernel lives on the EFI system partition.
///
/// The ESP is FAT, so it cannot carry LKO's structure or permissions. It holds
/// only what firmware must be able to read, and `LKO/Boot` is the LKOFS view of
/// this same directory.
const KERNEL_PATH: &uefi::CStr16 = cstr16!("\\EFI\\LOKO\\loko-kernel");

/// 64 KiB of stack for the kernel's first thread.
const KERNEL_STACK_PAGES: usize = 16;

/// Room for the converted memory map. 8 pages holds 1365 regions; real firmware
/// reports tens.
const MEMORY_MAP_PAGES: usize = 8;

/// How much of the low address space to identity-map so the `cr3` switch
/// survives its own next instruction.
const IDENTITY_MAPPED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// The most physical address space the offset window will ever cover.
///
/// 64 GiB costs 64 page-directory frames to map with 2 MiB pages, which the
/// arena can afford. Memory above this is not mapped and the kernel reports it
/// as unmanaged rather than failing to start.
const MAX_MAPPED_PHYSICAL: u64 = 64 * 1024 * 1024 * 1024;

#[entry]
fn main() -> Status {
    // Installs the logger and the allocator. Both stop working at
    // `ExitBootServices`, which is why nothing after that point logs.
    if uefi::helpers::init().is_err() {
        return Status::ABORTED;
    }

    log::info!("LokoOS bootloader {}", env!("CARGO_PKG_VERSION"));
    log::info!("stage 1: firmware initialised");

    match load_and_start() {
        Ok(never) => never,
        Err(message) => {
            log::error!("{message}");
            // Stall so the message is readable before firmware moves on.
            boot::stall(core::time::Duration::from_secs(10));
            Status::LOAD_ERROR
        }
    }
}

/// The whole handover. Returns only on failure.
fn load_and_start() -> Result<!, &'static str> {
    log::info!("stage 2: reading {KERNEL_PATH}");
    let (kernel_file, kernel_file_pages) = read_kernel()?;

    // SAFETY: `read_kernel` returned a live allocation of this length.
    let kernel_bytes = unsafe {
        core::slice::from_raw_parts(kernel_file.as_ptr(), kernel_file_pages * PAGE_SIZE as usize)
    };

    let image = elf::Image::parse(kernel_bytes).map_err(elf::ElfError::message)?;
    let (kernel_low, kernel_high) = image.virtual_extent();
    let kernel_span = kernel_high - kernel_low;
    log::info!(
        "kernel entry {:#x}, {} KiB across {:#x}..{:#x}",
        image.entry,
        kernel_span / 1024,
        kernel_low,
        kernel_high
    );

    log::info!("stage 3: reserving memory");
    let kernel_pages = (kernel_span.div_ceil(PAGE_SIZE)) as usize;
    let kernel_physical = allocate_pages(kernel_pages, "the kernel image")?;
    let arena = allocate_pages(ARENA_FRAMES, "the page tables")?;
    let stack = allocate_pages(KERNEL_STACK_PAGES, "the kernel stack")?;
    let regions = allocate_pages(MEMORY_MAP_PAGES, "the memory map")?;
    let boot_info_frame = allocate_pages(1, "the boot information")?;

    copy_segments(&image, kernel_low, kernel_physical.as_ptr() as u64);

    log::info!("stage 4: querying hardware");
    let framebuffer = find_framebuffer();
    let rsdp = find_acpi_rsdp();
    let highest_ram = highest_ram_address()?;
    log::info!(
        "{} MiB of RAM, framebuffer {}, ACPI at {:#x}",
        highest_ram / (1024 * 1024),
        if framebuffer.is_present() {
            "present"
        } else {
            "absent"
        },
        rsdp
    );

    log::info!("stage 5: building page tables");
    // SAFETY: the arena was just allocated, is page-aligned, and while boot
    // services are active UEFI identity-maps it.
    let mut tables = unsafe { PageTables::new(arena.as_ptr() as u64, ARENA_FRAMES) }
        .map_err(paging::PagingError::message)?;

    map_kernel(
        &mut tables,
        &image,
        kernel_low,
        kernel_physical.as_ptr() as u64,
    )?;

    // All of RAM, so the kernel can reach any frame without first building the
    // page tables it would need in order to build page tables.
    tables
        .map_huge_range(
            DEFAULT_PHYSICAL_MEMORY_OFFSET,
            0,
            highest_ram,
            Permissions::READ_WRITE,
        )
        .map_err(paging::PagingError::message)?;

    // The framebuffer, which is MMIO and therefore sits outside the RAM range
    // just mapped. Without this the kernel could only reach it through the
    // identity map, which is supposed to be temporary.
    //
    // Lenient, because on a machine whose RAM is remapped above the PCI hole
    // the framebuffer address can fall inside the range already mapped. A
    // conflict there is benign by construction: both mappings are the same
    // physical address at the same offset.
    if framebuffer.is_present() {
        let base = framebuffer.base & !(paging::HUGE_PAGE_SIZE - 1);
        let length = framebuffer.size + (framebuffer.base - base);
        tables
            .map_huge_range_lenient(
                DEFAULT_PHYSICAL_MEMORY_OFFSET + base,
                base,
                length,
                Permissions::READ_WRITE,
            )
            .map_err(paging::PagingError::message)?;
    }

    // The low identity map. This exists for exactly one instruction: the one
    // fetched immediately after `mov cr3`, which still comes from the address
    // this code is executing at. Without it the switch faults on its own next
    // instruction.
    //
    // READ_EXECUTE, not READ_WRITE. `Permissions::READ_WRITE` sets the
    // no-execute bit, and EFER.NXE is enabled a few lines below, so mapping
    // this range non-executable makes the `cr3` load page-fault on the next
    // instruction fetch with no handler installed, which is a triple fault and
    // a silently reset machine. Nothing writes through an identity address
    // between here and the jump, so read-execute keeps W^X intact.
    tables
        .map_huge_range(0, 0, IDENTITY_MAPPED_BYTES, Permissions::READ_EXECUTE)
        .map_err(paging::PagingError::message)?;

    log::info!(
        "page tables used {} of {ARENA_FRAMES} frames",
        tables.frames_used()
    );

    let boot_info = boot_info_frame.as_ptr() as *mut BootInfo;
    let region_array = regions.as_ptr() as *mut MemoryRegion;
    let region_capacity = MEMORY_MAP_PAGES * PAGE_SIZE as usize / size_of::<MemoryRegion>();

    let stack_top_physical = stack.as_ptr() as u64 + KERNEL_STACK_PAGES as u64 * PAGE_SIZE;
    let entry = image.entry;
    let cr3 = tables.cr3_value();

    log::info!("stage 6: exiting boot services");

    // Nothing below this line may allocate, log, or call a boot service.
    //
    // SAFETY: every allocation the kernel needs has been made, the page tables
    // are complete, and control is never returned to firmware.
    let memory_map = unsafe { boot::exit_boot_services(None) };

    let region_count = convert_memory_map(&memory_map, region_array, region_capacity);

    // SAFETY: `boot_info_frame` is a live, exclusively owned page, and
    // `BootInfo` is smaller than a page.
    unsafe {
        boot_info.write(BootInfo {
            magic: BOOT_MAGIC,
            version_major: PROTOCOL_VERSION_MAJOR,
            version_minor: PROTOCOL_VERSION_MINOR,
            size: size_of::<BootInfo>() as u32,
            // The kernel runs with the identity map gone, so it is given the
            // higher-half address of the array, not its physical address.
            memory_map: (DEFAULT_PHYSICAL_MEMORY_OFFSET + region_array as u64)
                as *const MemoryRegion,
            memory_map_len: region_count as u64,
            physical_memory_offset: DEFAULT_PHYSICAL_MEMORY_OFFSET,
            rsdp_addr: rsdp,
            kernel_physical_base: kernel_physical.as_ptr() as u64,
            kernel_virtual_base: kernel_low,
            kernel_size: kernel_span,
            framebuffer,
            flags: BootFlags::empty(),
            _reserved: 0,
        });
    }

    // SAFETY: ring 0, and no mapping with the no-execute bit has been used yet.
    unsafe { paging::enable_no_execute() };

    let boot_info_virtual = DEFAULT_PHYSICAL_MEMORY_OFFSET + boot_info as u64;
    let stack_top_virtual = DEFAULT_PHYSICAL_MEMORY_OFFSET + stack_top_physical;

    // SAFETY: `cr3` is a complete four-level table that maps the kernel at
    // `entry`, all of physical memory at the offset window, and the low 4 GiB
    // identically so that this very code stays mapped across the switch. The
    // stack pointer and the boot-info pointer are both expressed in the offset
    // window, so they stay valid after the identity map goes away.
    unsafe {
        core::arch::asm!(
            "mov cr3, {cr3}",
            "mov rsp, {stack}",
            // Terminate the call chain so a stack walker stops here rather than
            // wandering into whatever the firmware left behind.
            "xor rbp, rbp",
            "jmp {entry}",
            cr3 = in(reg) cr3,
            stack = in(reg) stack_top_virtual,
            entry = in(reg) entry,
            in("rdi") boot_info_virtual,
            options(noreturn)
        )
    }
}

/// Reads the kernel image off the EFI system partition.
///
/// Returns the buffer and its size in pages.
fn read_kernel() -> Result<(NonNull<u8>, usize), &'static str> {
    let mut fs = boot::get_image_file_system(boot::image_handle())
        .map_err(|_| "LokoOS couldn't read the drive it started from.")?;
    let mut volume = fs
        .open_volume()
        .map_err(|_| "LokoOS couldn't open the drive it started from.")?;

    let handle = volume
        .open(KERNEL_PATH, FileMode::Read, FileAttribute::empty())
        .map_err(|_| "The LokoOS system file is missing from this device.")?;

    let FileType::Regular(mut file) = handle
        .into_type()
        .map_err(|_| "The LokoOS system file on this device isn't valid.")?
    else {
        return Err("The LokoOS system file on this device isn't valid.");
    };

    let info = file
        .get_boxed_info::<FileInfo>()
        .map_err(|_| "LokoOS couldn't read the size of its own system file.")?;
    let size = info.file_size() as usize;
    if size == 0 {
        return Err("The LokoOS system file on this device is empty.");
    }

    let pages = size.div_ceil(PAGE_SIZE as usize);
    let buffer = allocate_pages(pages, "the kernel image")?;

    // SAFETY: `buffer` is a live allocation of `pages` pages, which is at least
    // `size` bytes.
    let slice = unsafe { core::slice::from_raw_parts_mut(buffer.as_ptr(), size) };
    let read = file
        .read(slice)
        .map_err(|_| "LokoOS couldn't read its own system file from this device.")?;
    if read != size {
        return Err("LokoOS could only read part of its own system file.");
    }

    Ok((buffer, pages))
}

/// Allocates zeroed pages, or explains which step ran out of memory.
fn allocate_pages(pages: usize, purpose: &'static str) -> Result<NonNull<u8>, &'static str> {
    let pointer = boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages)
        .map_err(|_| {
            log::error!("could not allocate {pages} pages for {purpose}");
            "LokoOS couldn't start: there isn't enough memory in this device."
        })?;
    // UEFI does not promise zeroed pages, and the kernel's `.bss` depends on
    // getting them.
    // SAFETY: the allocation is live and exactly this large.
    unsafe { core::ptr::write_bytes(pointer.as_ptr(), 0, pages * PAGE_SIZE as usize) };
    Ok(pointer)
}

/// Copies each loadable segment into the kernel's physical allocation and zeroes
/// the `.bss` tail.
fn copy_segments(image: &elf::Image<'_>, kernel_low: u64, kernel_physical: u64) {
    for segment in image.segments() {
        let offset = segment.virtual_address - kernel_low;
        let destination = (kernel_physical + offset) as *mut u8;
        let source = image.segment_data(&segment);

        // SAFETY: the destination lies inside the allocation sized from
        // `virtual_extent`, and the source is a validated slice of the file.
        unsafe {
            core::ptr::copy_nonoverlapping(source.as_ptr(), destination, source.len());
            // Anything the segment claims in memory but not in the file is
            // `.bss` and must read as zero.
            let tail = segment.memory_size as usize - source.len();
            if tail > 0 {
                core::ptr::write_bytes(destination.add(source.len()), 0, tail);
            }
        }
    }
}

/// Maps each kernel segment at its linked address with the permissions it
/// declared, so that W^X holds from the first instruction the kernel executes.
fn map_kernel(
    tables: &mut PageTables,
    image: &elf::Image<'_>,
    kernel_low: u64,
    kernel_physical: u64,
) -> Result<(), &'static str> {
    for segment in image.segments() {
        let permissions = match (segment.is_writable(), segment.is_executable()) {
            (false, true) => Permissions::READ_EXECUTE,
            (true, false) => Permissions::READ_WRITE,
            (false, false) => Permissions::READ_ONLY,
            // A segment asking to be both writable and executable is a build
            // error, not something to honour.
            (true, true) => {
                return Err("The LokoOS system file on this device was built incorrectly.")
            }
        };

        let offset = segment.virtual_address - kernel_low;
        tables
            .map_range(
                segment.virtual_address,
                kernel_physical + offset,
                segment.memory_size,
                permissions,
            )
            .map_err(paging::PagingError::message)?;
    }
    Ok(())
}

/// Whether a firmware memory type describes installed RAM.
///
/// An allow-list, not a deny-list. The first attempt excluded only the two
/// MMIO types and still came back with 64 GiB on a machine with 512 MiB,
/// because firmware describes plenty of address space under other types.
/// Naming what RAM *is* leaves no room for that.
///
/// `RESERVED` is excluded: it covers both firmware-reserved RAM and decorative
/// holes, and the kernel has no reason to read either. `UNUSABLE` is excluded
/// because it is memory that failed.
const fn is_installed_ram(ty: MemoryType) -> bool {
    matches!(
        ty,
        MemoryType::CONVENTIONAL
            | MemoryType::LOADER_CODE
            | MemoryType::LOADER_DATA
            | MemoryType::BOOT_SERVICES_CODE
            | MemoryType::BOOT_SERVICES_DATA
            | MemoryType::RUNTIME_SERVICES_CODE
            | MemoryType::RUNTIME_SERVICES_DATA
            | MemoryType::ACPI_RECLAIM
            | MemoryType::ACPI_NON_VOLATILE
            | MemoryType::PERSISTENT_MEMORY
    )
}

/// The highest physical address that installed memory reaches.
///
/// Deliberately not the highest address in the firmware memory map. Firmware
/// describes apertures far above installed RAM: on QEMU the 64-bit PCI hole
/// sits at 1 TiB. Mapping all of that would need a thousand page-directory
/// frames to describe address space holding nothing, which is what the first
/// boot of this bootloader tried to do before running out of arena.
fn highest_ram_address() -> Result<u64, &'static str> {
    let map = boot::memory_map(MemoryType::LOADER_DATA)
        .map_err(|_| "LokoOS couldn't read this device's memory layout.")?;

    let highest = map
        .entries()
        .filter(|d| is_installed_ram(d.ty))
        .map(|d| d.phys_start + d.page_count * PAGE_SIZE)
        .max()
        .unwrap_or(0);

    // A second line of defence, in case some firmware reports something
    // enormous under a type on the allow-list. The kernel reports memory above
    // the limit as unmanaged rather than failing to start.
    Ok(highest.min(MAX_MAPPED_PHYSICAL))
}
/// Asks the firmware for a linear framebuffer, or reports that there is none.
fn find_framebuffer() -> Framebuffer {
    let Ok(handle) = boot::get_handle_for_protocol::<GraphicsOutput>() else {
        log::warn!("no graphics output protocol; booting serial-only");
        return Framebuffer::NONE;
    };
    let Ok(mut gop) = boot::open_protocol_exclusive::<GraphicsOutput>(handle) else {
        log::warn!("graphics output protocol is busy; booting serial-only");
        return Framebuffer::NONE;
    };

    let info = gop.current_mode_info();
    let (width, height) = info.resolution();
    let format = match info.pixel_format() {
        GopPixelFormat::Rgb => PixelFormat::Rgbx8888,
        GopPixelFormat::Bgr => PixelFormat::Bgrx8888,
        // BltOnly has no linear framebuffer at all, and the bitmask formats
        // need a channel-shuffling blit that the early console does not have.
        // Reporting no framebuffer is honest; claiming one would paint garbage.
        other => {
            log::warn!("unsupported pixel format {other:?}; booting serial-only");
            return Framebuffer::NONE;
        }
    };

    let mut buffer = gop.frame_buffer();
    let framebuffer = Framebuffer {
        base: buffer.as_mut_ptr() as u64,
        size: buffer.size() as u64,
        width: width as u32,
        height: height as u32,
        stride_bytes: (info.stride() * 4) as u32,
        format,
    };
    log::info!(
        "framebuffer {}x{} at {:#x}",
        width,
        height,
        framebuffer.base
    );
    framebuffer
}

/// Finds the ACPI 2.0 RSDP, falling back to the 1.0 table.
fn find_acpi_rsdp() -> u64 {
    use uefi::table::cfg::ConfigTableEntry;

    uefi::system::with_config_table(|entries| {
        let mut fallback = 0u64;
        for entry in entries {
            // ACPI 2.0 first: its XSDT carries 64-bit table pointers, which the
            // 1.0 RSDT cannot express on a machine with tables above 4 GiB.
            if entry.guid == ConfigTableEntry::ACPI2_GUID {
                return entry.address as u64;
            }
            if entry.guid == ConfigTableEntry::ACPI_GUID {
                fallback = entry.address as u64;
            }
        }
        fallback
    })
}

/// Translates the firmware memory map into the boot protocol's form: sorted by
/// address, with UEFI's memory types collapsed onto LokoOS's.
///
/// Runs after `ExitBootServices`, so it allocates nothing and writes into the
/// array reserved earlier.
fn convert_memory_map(map: &impl MemoryMap, out: *mut MemoryRegion, capacity: usize) -> usize {
    let mut count = 0usize;

    for descriptor in map.entries() {
        if count >= capacity {
            // Dropping regions is bad, but writing past the array is worse, and
            // the kernel will notice the total is short of what the firmware
            // reported.
            break;
        }
        let kind = match descriptor.ty {
            MemoryType::CONVENTIONAL => MemoryKind::Usable,
            // Boot-services memory is free after this call, but it is also
            // where the firmware's own leftovers live. LokoOS treats it as
            // reclaimable rather than immediately usable, and releases it
            // deliberately later.
            MemoryType::LOADER_CODE
            | MemoryType::LOADER_DATA
            | MemoryType::BOOT_SERVICES_CODE
            | MemoryType::BOOT_SERVICES_DATA => MemoryKind::BootloaderReclaimable,
            MemoryType::ACPI_RECLAIM => MemoryKind::AcpiReclaimable,
            MemoryType::ACPI_NON_VOLATILE => MemoryKind::AcpiNvs,
            MemoryType::UNUSABLE => MemoryKind::BadMemory,
            _ => MemoryKind::Reserved,
        };

        let region = MemoryRegion {
            start: descriptor.phys_start,
            len: descriptor.page_count * PAGE_SIZE,
            kind,
            _reserved: 0,
        };
        if region.len == 0 {
            continue;
        }

        // SAFETY: `count` is below `capacity`, so this element is inside the
        // reserved array.
        unsafe { out.add(count).write(region) };
        count += 1;
    }

    // The protocol requires ascending, non-overlapping regions, and UEFI does
    // not promise an ordered map. Insertion sort: no allocation, and firmware
    // maps are both small and nearly sorted already, which is the case this
    // algorithm is best at.
    for i in 1..count {
        // SAFETY: every index below `count` was written above.
        let current = unsafe { out.add(i).read() };
        let mut j = i;
        while j > 0 {
            // SAFETY: as above.
            let previous = unsafe { out.add(j - 1).read() };
            if previous.start <= current.start {
                break;
            }
            // SAFETY: as above.
            unsafe { out.add(j).write(previous) };
            j -= 1;
        }
        // SAFETY: as above.
        unsafe { out.add(j).write(current) };
    }

    count
}
