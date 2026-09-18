//! # The LokoOS boot protocol
//!
//! Exactly one structure crosses the boundary between the bootloader and the
//! kernel: [`BootInfo`]. It is `#[repr(C)]`, versioned, and self-describing, so
//! that a kernel and a bootloader from different builds either work together or
//! fail immediately and legibly — never subtly.
//!
//! Boot is the one place in an operating system where a mismatch cannot produce
//! a diagnostic, because the diagnostic machinery is what is being set up. So
//! the checks happen in the order that needs the least machinery: magic number
//! first (needs nothing), then version, then size, then contents.
//!
//! ## Status
//!
//! **Defined and compiling. Not yet exercised on hardware or in a virtual
//! machine**, because this development environment has no emulator installed.
//! See `documentation/STATUS.md`.

// `no_std` everywhere except under `cargo test`, where the test harness itself
// needs std. The code under test is identical either way.
#![cfg_attr(not(test), no_std)]

use bitflags::bitflags;

/// Identifies a structure as a LokoOS boot info block.
///
/// Spells "LOKOBOOT" in ASCII, little-endian, so it is recognisable in a memory
/// dump without a decoder ring.
pub const BOOT_MAGIC: u64 = u64::from_le_bytes(*b"LOKOBOOT");

/// Where the bootloader maps all of physical memory.
///
/// The first address of the canonical higher half. Every physical frame is
/// reachable at `DEFAULT_PHYSICAL_MEMORY_OFFSET + physical_address`, which lets
/// the kernel touch any frame without first building the page tables it would
/// need in order to build page tables.
///
/// The kernel reads the actual value from [`BootInfo::physical_memory_offset`]
/// rather than assuming this constant, so that the window can be randomised
/// later without changing the kernel.
pub const DEFAULT_PHYSICAL_MEMORY_OFFSET: u64 = 0xFFFF_8000_0000_0000;

/// Where the kernel image is linked to run. Must match `kernel/linker.ld`.
pub const KERNEL_VIRTUAL_BASE: u64 = 0xFFFF_FFFF_8000_0000;

/// The boot protocol version this crate defines.
///
/// The kernel refuses to start if the bootloader's version differs in the major
/// component. Minor increments are additive: new fields are appended and the
/// `size` field tells an older kernel where its knowledge ends.
pub const PROTOCOL_VERSION_MAJOR: u16 = 0;
/// See [`PROTOCOL_VERSION_MAJOR`].
pub const PROTOCOL_VERSION_MINOR: u16 = 1;

/// How the firmware described a region of physical memory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum MemoryKind {
    /// Free for the kernel to allocate.
    Usable = 0,
    /// Firmware-reserved. Never touch.
    Reserved = 1,
    /// Holds bootloader structures, including this one. Reclaimable once the
    /// kernel has copied out everything it needs.
    BootloaderReclaimable = 2,
    /// The loaded kernel image and any modules.
    KernelAndModules = 3,
    /// ACPI tables. Reclaimable after they are parsed.
    AcpiReclaimable = 4,
    /// ACPI non-volatile storage. Must be preserved across sleep.
    AcpiNvs = 5,
    /// Reported faulty by the firmware.
    BadMemory = 6,
    /// The graphics framebuffer.
    Framebuffer = 7,
}

impl MemoryKind {
    /// Whether the kernel's frame allocator may hand out frames from a region
    /// of this kind **at the moment the kernel starts**.
    ///
    /// `BootloaderReclaimable` is deliberately excluded here even though it
    /// becomes usable later: it still holds the [`BootInfo`] being read. It is
    /// released explicitly, once, after the kernel has taken its own copy.
    #[must_use]
    pub const fn usable_at_handoff(self) -> bool {
        matches!(self, MemoryKind::Usable)
    }

    /// Whether this region may ever be used for general allocation.
    #[must_use]
    pub const fn eventually_reclaimable(self) -> bool {
        matches!(
            self,
            MemoryKind::Usable | MemoryKind::BootloaderReclaimable | MemoryKind::AcpiReclaimable
        )
    }
}

/// A contiguous run of physical memory of a single kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct MemoryRegion {
    /// Physical start address. Always 4 KiB aligned.
    pub start: u64,
    /// Length in bytes. Always a multiple of 4 KiB.
    pub len: u64,
    /// What the firmware said this memory is.
    pub kind: MemoryKind,
    /// Padding to keep the structure 8-byte aligned and its layout explicit.
    pub _reserved: u32,
}

impl MemoryRegion {
    /// One past the last byte of the region.
    ///
    /// Saturating, so a bogus firmware entry that runs off the end of the
    /// address space produces a clamped value rather than a wrapped one that
    /// would look like a region starting at zero.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.start.saturating_add(self.len)
    }

    /// The number of 4 KiB frames the region contains.
    #[must_use]
    pub const fn frame_count(&self) -> u64 {
        self.len / 4096
    }

    /// Whether the region contains `address`.
    #[must_use]
    pub const fn contains(&self, address: u64) -> bool {
        address >= self.start && address < self.end()
    }
}

/// The pixel layout of the framebuffer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum PixelFormat {
    /// Four bytes per pixel, blue in the lowest address.
    Bgrx8888 = 0,
    /// Four bytes per pixel, red in the lowest address.
    Rgbx8888 = 1,
}

impl PixelFormat {
    /// Bytes occupied by one pixel.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        4
    }
}

/// A linear framebuffer the firmware left us in.
///
/// LokoOS's compositor targets the GPU, but the very first thing the kernel
/// must be able to do is put a legible message on the screen when something has
/// gone wrong before any driver exists. That is what this is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Framebuffer {
    /// Physical base address, or `0` if the firmware gave us no framebuffer.
    pub base: u64,
    /// Total size in bytes.
    pub size: u64,
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in pixels.
    pub height: u32,
    /// Bytes per row, which may exceed `width * bytes_per_pixel`.
    pub stride_bytes: u32,
    /// Pixel layout.
    pub format: PixelFormat,
}

impl Framebuffer {
    /// An explicitly absent framebuffer, for headless and serial-only boots.
    pub const NONE: Framebuffer = Framebuffer {
        base: 0,
        size: 0,
        width: 0,
        height: 0,
        stride_bytes: 0,
        format: PixelFormat::Bgrx8888,
    };

    /// Whether the firmware actually gave us a framebuffer.
    #[must_use]
    pub const fn is_present(&self) -> bool {
        self.base != 0 && self.size != 0
    }

    /// The byte offset of a pixel, or `None` if it is outside the visible area
    /// or would run past the end of the buffer.
    ///
    /// Bounds-checked in the protocol crate rather than at each call site,
    /// because the alternative is every early-boot painting routine doing its
    /// own arithmetic against a firmware-supplied stride.
    pub const fn pixel_offset(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = y as u64 * self.stride_bytes as u64 + x as u64 * 4;
        if offset.saturating_add(4) > self.size {
            return None;
        }
        Some(offset as usize)
    }
}

bitflags! {
    /// Choices made before the kernel started, which it cannot re-derive.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    #[repr(transparent)]
    pub struct BootFlags: u32 {
        /// Boot into Loko Recovery instead of the normal session.
        const RECOVERY        = 1 << 0;
        /// Skip GPU driver loading and stay on the firmware framebuffer.
        const SAFE_GRAPHICS   = 1 << 1;
        /// Print every stage to the serial port.
        const VERBOSE         = 1 << 2;
        /// Firmware reported that Secure Boot is enabled and the kernel image
        /// signature was verified.
        const SECURE_BOOT     = 1 << 3;
        /// The previous boot did not reach a user session. The kernel uses this
        /// to offer recovery rather than looping.
        const PREVIOUS_FAILED = 1 << 4;
    }
}

/// Everything the bootloader tells the kernel.
///
/// Passed by pointer in `rdi` per the SysV C ABI. The kernel must treat every
/// field as untrusted until [`BootInfo::validate`] has returned `Ok`: on a real
/// machine these values come from firmware, and firmware is not always right.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct BootInfo {
    /// Must equal [`BOOT_MAGIC`].
    pub magic: u64,
    /// Protocol major version.
    pub version_major: u16,
    /// Protocol minor version.
    pub version_minor: u16,
    /// `size_of::<BootInfo>()` as the bootloader saw it.
    pub size: u32,
    /// Pointer to an array of [`MemoryRegion`], in the kernel's address space.
    pub memory_map: *const MemoryRegion,
    /// Number of entries in `memory_map`.
    pub memory_map_len: u64,
    /// Virtual address at which all of physical memory is mapped.
    ///
    /// The bootloader sets up this mapping so the kernel can reach any physical
    /// frame without having to bootstrap paging before it can allocate the page
    /// tables it needs in order to bootstrap paging.
    pub physical_memory_offset: u64,
    /// Physical address of the ACPI RSDP, or `0` if the firmware had none.
    pub rsdp_addr: u64,
    /// Physical address where the kernel image was loaded.
    pub kernel_physical_base: u64,
    /// Virtual address the kernel image was linked for.
    pub kernel_virtual_base: u64,
    /// Size of the loaded kernel image in bytes.
    pub kernel_size: u64,
    /// The framebuffer, or [`Framebuffer::NONE`].
    pub framebuffer: Framebuffer,
    /// Boot-time choices.
    pub flags: BootFlags,
    /// Padding to an 8-byte boundary.
    pub _reserved: u32,
}

/// Why a [`BootInfo`] was rejected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BootInfoError {
    /// The magic number did not match. Either this is not a `BootInfo`, or the
    /// pointer the kernel was given is wrong.
    BadMagic,
    /// The bootloader implements an incompatible major version.
    VersionMismatch {
        /// What the bootloader provided.
        found: u16,
        /// What this kernel requires.
        expected: u16,
    },
    /// The structure is smaller than this kernel's own definition, so fields
    /// this kernel expects were never written.
    TooSmall,
    /// The memory map is empty or its pointer is null.
    NoMemoryMap,
    /// A region was not 4 KiB aligned, had zero length, or wrapped the address
    /// space.
    MalformedRegion,
    /// Regions overlap or are not sorted by ascending start address.
    UnsortedMemoryMap,
    /// No usable memory was reported at all.
    NoUsableMemory,
    /// `physical_memory_offset` is not aligned to a 4 KiB boundary.
    MisalignedPhysicalOffset,
}

impl BootInfoError {
    /// A message short enough to fit on an early-boot screen, written for
    /// whoever is standing in front of the machine.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            BootInfoError::BadMagic => {
                "LokoOS could not start: the bootloader and the system don't match."
            }
            BootInfoError::VersionMismatch { .. } => {
                "LokoOS could not start: the bootloader is a different version from the system. Run Loko Recovery and choose Repair Bootloader."
            }
            BootInfoError::TooSmall => {
                "LokoOS could not start: the bootloader is older than the system. Run Loko Recovery and choose Repair Bootloader."
            }
            BootInfoError::NoMemoryMap | BootInfoError::NoUsableMemory => {
                "LokoOS could not start: this device's firmware did not report any usable memory."
            }
            BootInfoError::MalformedRegion | BootInfoError::UnsortedMemoryMap => {
                "LokoOS could not start: this device's firmware reported its memory incorrectly."
            }
            BootInfoError::MisalignedPhysicalOffset => {
                "LokoOS could not start: the bootloader set up memory incorrectly."
            }
        }
    }
}

impl BootInfo {
    /// Checks everything that can be checked before trusting any field.
    ///
    /// Ordered from the check that needs least machinery to the one that needs
    /// most, so that the most likely catastrophic case — a wrong pointer —
    /// is caught by a single comparison.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that `self.memory_map` points to
    /// `self.memory_map_len` readable, correctly aligned `MemoryRegion` values.
    /// Everything else about the contents may be arbitrary.
    pub unsafe fn validate(&self) -> Result<(), BootInfoError> {
        if self.magic != BOOT_MAGIC {
            return Err(BootInfoError::BadMagic);
        }
        if self.version_major != PROTOCOL_VERSION_MAJOR {
            return Err(BootInfoError::VersionMismatch {
                found: self.version_major,
                expected: PROTOCOL_VERSION_MAJOR,
            });
        }
        if (self.size as usize) < core::mem::size_of::<BootInfo>() {
            return Err(BootInfoError::TooSmall);
        }
        if self.memory_map.is_null() || self.memory_map_len == 0 {
            return Err(BootInfoError::NoMemoryMap);
        }
        if self.physical_memory_offset % 4096 != 0 {
            return Err(BootInfoError::MisalignedPhysicalOffset);
        }

        // SAFETY: the caller guarantees the pointer and length describe a
        // readable, correctly aligned array.
        let regions =
            unsafe { core::slice::from_raw_parts(self.memory_map, self.memory_map_len as usize) };
        validate_memory_map(regions)
    }

    /// Total bytes of memory usable at handoff.
    ///
    /// # Safety
    ///
    /// Same requirement as [`BootInfo::validate`], which should have been
    /// called first.
    pub unsafe fn usable_bytes(&self) -> u64 {
        // SAFETY: guaranteed by the caller.
        let regions =
            unsafe { core::slice::from_raw_parts(self.memory_map, self.memory_map_len as usize) };
        regions
            .iter()
            .filter(|r| r.kind.usable_at_handoff())
            .map(|r| r.len)
            .sum()
    }
}

/// Validates a memory map independently of any pointer, so it can be tested.
///
/// Split out from [`BootInfo::validate`] precisely so that the interesting
/// logic is reachable from a safe unit test on a host machine.
pub fn validate_memory_map(regions: &[MemoryRegion]) -> Result<(), BootInfoError> {
    if regions.is_empty() {
        return Err(BootInfoError::NoMemoryMap);
    }

    let mut previous_end = 0u64;
    let mut any_usable = false;

    for region in regions {
        if region.len == 0 || region.start % 4096 != 0 || region.len % 4096 != 0 {
            return Err(BootInfoError::MalformedRegion);
        }
        // An overflowing region would make `end()` saturate and silently
        // compare as if it were shorter than it is.
        if region.start.checked_add(region.len).is_none() {
            return Err(BootInfoError::MalformedRegion);
        }
        if region.start < previous_end {
            return Err(BootInfoError::UnsortedMemoryMap);
        }
        previous_end = region.end();
        if region.kind.usable_at_handoff() {
            any_usable = true;
        }
    }

    if !any_usable {
        return Err(BootInfoError::NoUsableMemory);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(start: u64, len: u64, kind: MemoryKind) -> MemoryRegion {
        MemoryRegion {
            start,
            len,
            kind,
            _reserved: 0,
        }
    }

    #[test]
    fn magic_is_readable_in_a_hex_dump() {
        assert_eq!(BOOT_MAGIC.to_le_bytes(), *b"LOKOBOOT");
    }

    #[test]
    fn a_plausible_memory_map_validates() {
        let map = [
            region(0, 0x1000 * 16, MemoryKind::Reserved),
            region(0x10000, 0x1000 * 256, MemoryKind::Usable),
            region(0x110000, 0x1000 * 8, MemoryKind::KernelAndModules),
        ];
        assert_eq!(validate_memory_map(&map), Ok(()));
    }

    #[test]
    fn an_empty_map_is_rejected() {
        assert_eq!(validate_memory_map(&[]), Err(BootInfoError::NoMemoryMap));
    }

    #[test]
    fn a_map_with_no_usable_memory_is_rejected() {
        let map = [region(0, 0x1000, MemoryKind::Reserved)];
        assert_eq!(
            validate_memory_map(&map),
            Err(BootInfoError::NoUsableMemory)
        );
    }

    #[test]
    fn unaligned_or_empty_regions_are_rejected() {
        for bad in [
            region(0x1001, 0x1000, MemoryKind::Usable),
            region(0x1000, 0x999, MemoryKind::Usable),
            region(0x1000, 0, MemoryKind::Usable),
        ] {
            assert_eq!(
                validate_memory_map(&[bad]),
                Err(BootInfoError::MalformedRegion),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn overlapping_or_unsorted_regions_are_rejected() {
        let overlapping = [
            region(0x1000, 0x2000, MemoryKind::Usable),
            region(0x2000, 0x1000, MemoryKind::Usable),
        ];
        assert_eq!(
            validate_memory_map(&overlapping),
            Err(BootInfoError::UnsortedMemoryMap)
        );

        let unsorted = [
            region(0x10000, 0x1000, MemoryKind::Usable),
            region(0x1000, 0x1000, MemoryKind::Usable),
        ];
        assert_eq!(
            validate_memory_map(&unsorted),
            Err(BootInfoError::UnsortedMemoryMap)
        );
    }

    #[test]
    fn a_region_that_wraps_the_address_space_is_rejected() {
        // Without the checked_add, `end()` would saturate to u64::MAX and this
        // region would look shorter than it claims, so a later region would
        // appear to be beyond it.
        let wrapping = region(0xFFFF_FFFF_FFFF_F000, 0x2000, MemoryKind::Usable);
        assert_eq!(
            validate_memory_map(&[wrapping]),
            Err(BootInfoError::MalformedRegion)
        );
    }

    #[test]
    fn bootloader_memory_is_not_allocatable_at_handoff() {
        // The BootInfo being read lives in it.
        assert!(!MemoryKind::BootloaderReclaimable.usable_at_handoff());
        assert!(MemoryKind::BootloaderReclaimable.eventually_reclaimable());
        assert!(!MemoryKind::AcpiNvs.eventually_reclaimable());
        assert!(!MemoryKind::BadMemory.eventually_reclaimable());
    }

    #[test]
    fn framebuffer_bounds_are_checked() {
        let fb = Framebuffer {
            base: 0xE000_0000,
            size: 1920 * 1080 * 4,
            width: 1920,
            height: 1080,
            stride_bytes: 1920 * 4,
            format: PixelFormat::Bgrx8888,
        };
        assert!(fb.is_present());
        assert_eq!(fb.pixel_offset(0, 0), Some(0));
        assert_eq!(fb.pixel_offset(1919, 0), Some(1919 * 4));
        assert_eq!(fb.pixel_offset(1920, 0), None, "x is out of bounds");
        assert_eq!(fb.pixel_offset(0, 1080), None, "y is out of bounds");
    }

    #[test]
    fn a_framebuffer_with_a_lying_stride_cannot_overrun() {
        // Firmware that reports a size smaller than height * stride must not
        // lead to a write past the end of the mapping.
        let fb = Framebuffer {
            base: 0xE000_0000,
            size: 4096,
            width: 1920,
            height: 1080,
            stride_bytes: 1920 * 4,
            format: PixelFormat::Bgrx8888,
        };
        assert_eq!(fb.pixel_offset(0, 0), Some(0));
        assert_eq!(fb.pixel_offset(0, 1), None, "row 1 is past the real size");
    }

    #[test]
    fn an_absent_framebuffer_reports_itself_absent() {
        assert!(!Framebuffer::NONE.is_present());
        assert_eq!(Framebuffer::NONE.pixel_offset(0, 0), None);
    }

    #[test]
    fn every_boot_error_says_what_to_do_or_what_is_wrong() {
        for e in [
            BootInfoError::BadMagic,
            BootInfoError::VersionMismatch {
                found: 1,
                expected: 0,
            },
            BootInfoError::TooSmall,
            BootInfoError::NoMemoryMap,
            BootInfoError::MalformedRegion,
            BootInfoError::UnsortedMemoryMap,
            BootInfoError::NoUsableMemory,
            BootInfoError::MisalignedPhysicalOffset,
        ] {
            let m = e.message();
            assert!(m.starts_with("LokoOS could not start:"), "{e:?}: {m}");
            assert!(!m.contains("0x"), "{e:?} leaks a raw value at the user");
        }
    }

    #[test]
    fn the_structure_layout_is_what_the_bootloader_will_write() {
        // If this changes, the protocol version must change with it.
        assert_eq!(core::mem::align_of::<BootInfo>(), 8);
        assert_eq!(core::mem::size_of::<MemoryRegion>(), 24);
        assert_eq!(core::mem::size_of::<Framebuffer>(), 32);
    }
}
