//! Builds the page tables the kernel starts life on.
//!
//! UEFI hands control over with everything identity-mapped and, on most
//! firmware, everything writable and executable. LokoOS does not start in that
//! state. The bootloader builds a fresh four-level table with three things in
//! it, and switches to it as the last thing it does:
//!
//! 1. **The kernel image**, at its linked higher-half address, mapped at 4 KiB
//!    granularity so that each ELF segment gets the permissions it declared.
//!    Read-only data is not writable; code is not writable; data is not
//!    executable.
//! 2. **All of physical memory**, at [`DEFAULT_PHYSICAL_MEMORY_OFFSET`], using
//!    2 MiB pages. This is how the kernel reaches an arbitrary frame before it
//!    has an address-space manager.
//! 3. **The low 4 GiB, identity-mapped.** This exists for exactly one reason:
//!    the instruction after `mov cr3` executes at the address it was fetched
//!    from, which is still a low identity address. Without this, the switch
//!    faults on its own next instruction. The kernel drops this mapping once it
//!    is running from the higher half.
//!
//! All tables are carved from one contiguous arena allocated before
//! `ExitBootServices`, because afterwards there is no allocator.

use core::ptr;

/// Page-table entry bits.
mod bits {
    /// The mapping is valid.
    pub const PRESENT: u64 = 1 << 0;
    /// Writes are permitted.
    pub const WRITABLE: u64 = 1 << 1;
    /// This entry maps a 2 MiB page rather than pointing at another table.
    pub const HUGE: u64 = 1 << 7;
    /// Instruction fetches are forbidden. Requires `EFER.NXE`.
    pub const NO_EXECUTE: u64 = 1 << 63;
}

/// The physical address bits of a page-table entry.
const ADDRESS_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// Bytes in a 4 KiB page.
pub const PAGE_SIZE: u64 = 4096;
/// Bytes in a 2 MiB page.
pub const HUGE_PAGE_SIZE: u64 = 2 * 1024 * 1024;

/// Permissions for a mapping, in the bootloader's own terms.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Permissions {
    /// Writes permitted.
    pub writable: bool,
    /// Instruction fetch permitted.
    pub executable: bool,
}

impl Permissions {
    /// Read-only data.
    pub const READ_ONLY: Permissions = Permissions {
        writable: false,
        executable: false,
    };
    /// Read-write data.
    pub const READ_WRITE: Permissions = Permissions {
        writable: true,
        executable: false,
    };
    /// Executable code.
    pub const READ_EXECUTE: Permissions = Permissions {
        writable: false,
        executable: true,
    };

    const fn to_bits(self) -> u64 {
        let mut flags = bits::PRESENT;
        if self.writable {
            flags |= bits::WRITABLE;
        }
        if !self.executable {
            flags |= bits::NO_EXECUTE;
        }
        flags
    }
}

/// Why a mapping could not be made.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PagingError {
    /// The arena ran out of frames for intermediate tables.
    ArenaExhausted,
    /// A 2 MiB mapping was requested over an address already covered by 4 KiB
    /// pages, or the reverse. Rejected rather than silently replaced, because
    /// replacing it would orphan the existing mapping.
    Conflict,
    /// The virtual or physical address was not correctly aligned.
    Misaligned,
}

impl PagingError {
    /// A message for whoever is looking at the screen.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            PagingError::ArenaExhausted => {
                "LokoOS couldn't start: there isn't enough memory in this device to set it up."
            }
            PagingError::Conflict | PagingError::Misaligned => {
                "LokoOS couldn't start: it was unable to set up this device's memory."
            }
        }
    }
}

/// A four-level page table under construction.
///
/// While this exists, UEFI's identity mapping is still active, so a table's
/// physical address is also a usable pointer. That stops being true the moment
/// `cr3` is loaded, which is why nothing here outlives the switch.
pub struct PageTables {
    pml4: u64,
    arena_base: u64,
    arena_frames: usize,
    next_frame: usize,
}

impl PageTables {
    /// Takes ownership of a zeroed, page-aligned, identity-mapped arena.
    ///
    /// # Safety
    ///
    /// `arena_base` must point to `arena_frames` contiguous 4 KiB pages that
    /// are writable, identity-mapped, and used by nothing else.
    pub unsafe fn new(arena_base: u64, arena_frames: usize) -> Result<Self, PagingError> {
        if arena_base % PAGE_SIZE != 0 || arena_frames == 0 {
            return Err(PagingError::Misaligned);
        }
        // SAFETY: the caller guarantees the arena is writable and this large.
        unsafe {
            ptr::write_bytes(arena_base as *mut u8, 0, arena_frames * PAGE_SIZE as usize);
        }
        let mut tables = PageTables {
            pml4: 0,
            arena_base,
            arena_frames,
            next_frame: 0,
        };
        tables.pml4 = tables.take_frame()?;
        Ok(tables)
    }

    /// The physical address to load into `cr3`.
    #[must_use]
    pub const fn cr3_value(&self) -> u64 {
        self.pml4
    }

    /// How many arena frames have been used. Reported at boot so that the arena
    /// size is a measured number rather than a guess that silently drifts.
    #[must_use]
    pub const fn frames_used(&self) -> usize {
        self.next_frame
    }

    /// Takes the next zeroed frame from the arena.
    fn take_frame(&mut self) -> Result<u64, PagingError> {
        if self.next_frame >= self.arena_frames {
            return Err(PagingError::ArenaExhausted);
        }
        let address = self.arena_base + self.next_frame as u64 * PAGE_SIZE;
        self.next_frame += 1;
        Ok(address)
    }

    /// Reads a table entry.
    fn read_entry(table: u64, index: usize) -> u64 {
        // SAFETY: `table` came from the arena or from another entry's address
        // field, so it is a live, identity-mapped page-table frame, and `index`
        // is masked to 0..512 by the index helpers.
        unsafe { ptr::read_volatile((table as *const u64).add(index)) }
    }

    /// Writes a table entry.
    fn write_entry(table: u64, index: usize, value: u64) {
        // SAFETY: as `read_entry`.
        unsafe { ptr::write_volatile((table as *mut u64).add(index), value) }
    }

    /// Returns the next-level table for `index`, creating it if absent.
    ///
    /// Intermediate entries are always writable and always executable. On
    /// x86-64 the effective permission is the AND of the writable bits and the
    /// OR of the no-execute bits down the whole walk, so restricting an
    /// intermediate level would restrict every leaf under it. The restriction
    /// belongs on the leaf, where it describes one page.
    fn descend(&mut self, table: u64, index: usize) -> Result<u64, PagingError> {
        let entry = Self::read_entry(table, index);
        if entry & bits::PRESENT != 0 {
            if entry & bits::HUGE != 0 {
                return Err(PagingError::Conflict);
            }
            return Ok(entry & ADDRESS_MASK);
        }
        let frame = self.take_frame()?;
        Self::write_entry(table, index, frame | bits::PRESENT | bits::WRITABLE);
        Ok(frame)
    }

    /// Maps one 4 KiB page.
    pub fn map_page(
        &mut self,
        virtual_address: u64,
        physical_address: u64,
        permissions: Permissions,
    ) -> Result<(), PagingError> {
        if virtual_address % PAGE_SIZE != 0 || physical_address % PAGE_SIZE != 0 {
            return Err(PagingError::Misaligned);
        }
        let pdpt = self.descend(self.pml4, pml4_index(virtual_address))?;
        let pd = self.descend(pdpt, pdpt_index(virtual_address))?;
        let pt = self.descend(pd, pd_index(virtual_address))?;

        let index = pt_index(virtual_address);
        let existing = Self::read_entry(pt, index);
        if existing & bits::PRESENT != 0 {
            return Err(PagingError::Conflict);
        }
        Self::write_entry(pt, index, physical_address | permissions.to_bits());
        Ok(())
    }

    /// Maps one 2 MiB page.
    pub fn map_huge_page(
        &mut self,
        virtual_address: u64,
        physical_address: u64,
        permissions: Permissions,
    ) -> Result<(), PagingError> {
        if virtual_address % HUGE_PAGE_SIZE != 0 || physical_address % HUGE_PAGE_SIZE != 0 {
            return Err(PagingError::Misaligned);
        }
        let pdpt = self.descend(self.pml4, pml4_index(virtual_address))?;
        let pd = self.descend(pdpt, pdpt_index(virtual_address))?;

        let index = pd_index(virtual_address);
        let existing = Self::read_entry(pd, index);
        if existing & bits::PRESENT != 0 {
            return Err(PagingError::Conflict);
        }
        Self::write_entry(
            pd,
            index,
            physical_address | permissions.to_bits() | bits::HUGE,
        );
        Ok(())
    }

    /// Maps `[physical_base, physical_base + length)` at `virtual_base` using
    /// 2 MiB pages, rounding the length up.
    pub fn map_huge_range(
        &mut self,
        virtual_base: u64,
        physical_base: u64,
        length: u64,
        permissions: Permissions,
    ) -> Result<(), PagingError> {
        let pages = length.div_ceil(HUGE_PAGE_SIZE);
        for i in 0..pages {
            let offset = i * HUGE_PAGE_SIZE;
            self.map_huge_page(virtual_base + offset, physical_base + offset, permissions)?;
        }
        Ok(())
    }

    /// Like [`PageTables::map_huge_range`], but treats an existing mapping as
    /// success rather than as a conflict.
    ///
    /// Only for ranges where an overlap is known to be harmless because the
    /// existing entry maps the same physical address. The strict version stays
    /// the default so that an accidental double mapping is still an error.
    pub fn map_huge_range_lenient(
        &mut self,
        virtual_base: u64,
        physical_base: u64,
        length: u64,
        permissions: Permissions,
    ) -> Result<(), PagingError> {
        let pages = length.div_ceil(HUGE_PAGE_SIZE);
        for i in 0..pages {
            let offset = i * HUGE_PAGE_SIZE;
            match self.map_huge_page(virtual_base + offset, physical_base + offset, permissions) {
                Ok(()) | Err(PagingError::Conflict) => {}
                Err(other) => return Err(other),
            }
        }
        Ok(())
    }
    /// Maps `[physical_base, physical_base + length)` at `virtual_base` using
    /// 4 KiB pages, rounding the length up.
    pub fn map_range(
        &mut self,
        virtual_base: u64,
        physical_base: u64,
        length: u64,
        permissions: Permissions,
    ) -> Result<(), PagingError> {
        let pages = length.div_ceil(PAGE_SIZE);
        for i in 0..pages {
            let offset = i * PAGE_SIZE;
            self.map_page(virtual_base + offset, physical_base + offset, permissions)?;
        }
        Ok(())
    }
}

/// The number of arena frames to reserve.
///
/// One PML4, plus the intermediate tables for three regions. Sized with
/// headroom and checked at runtime: [`PageTables::frames_used`] is reported at
/// boot, so if this ever becomes tight it shows up as a number rather than as
/// an out-of-memory failure on an unusual machine.
pub const ARENA_FRAMES: usize = 256;

fn pml4_index(address: u64) -> usize {
    ((address >> 39) & 0x1FF) as usize
}

fn pdpt_index(address: u64) -> usize {
    ((address >> 30) & 0x1FF) as usize
}

fn pd_index(address: u64) -> usize {
    ((address >> 21) & 0x1FF) as usize
}

fn pt_index(address: u64) -> usize {
    ((address >> 12) & 0x1FF) as usize
}

/// Enables `EFER.NXE`, without which the no-execute bit is reserved and any
/// entry that sets it causes a page fault.
///
/// UEFI usually enables this already. "Usually" is not a basis for setting a
/// bit in every data mapping in the system, so LokoOS sets it itself.
///
/// # Safety
///
/// Must run in ring 0 before any mapping with [`bits::NO_EXECUTE`] is used.
pub unsafe fn enable_no_execute() {
    const IA32_EFER: u32 = 0xC000_0080;
    const NXE: u64 = 1 << 11;

    // SAFETY: IA32_EFER exists on every x86-64 processor, and the caller
    // guarantees ring 0.
    unsafe {
        let (low, high): (u32, u32);
        core::arch::asm!("rdmsr", in("ecx") IA32_EFER, out("eax") low, out("edx") high, options(nomem, nostack));
        let value = ((high as u64) << 32) | low as u64;
        let updated = value | NXE;
        core::arch::asm!(
            "wrmsr",
            in("ecx") IA32_EFER,
            in("eax") updated as u32,
            in("edx") (updated >> 32) as u32,
            options(nomem, nostack)
        );
    }
}
