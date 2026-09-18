//! # Physical memory for the LokoOS kernel
//!
//! The frame allocator is the first thing in a kernel that can be wrong in a
//! way nothing else survives: hand out the same frame twice and the damage
//! surfaces somewhere unrelated, minutes later. So it lives here, in a crate
//! with no hardware dependencies, where every branch can be driven from a host
//! unit test.
//!
//! The allocator is a bitmap. A bitmap costs one bit per 4 KiB frame — 32 KiB
//! per gigabyte of RAM — and in exchange gives O(1) free, exact double-free
//! detection, and a structure that can be checked for consistency at any
//! moment. A free-list would be smaller and would have neither property.
//!
//! ## Status
//!
//! **Implemented and tested on the host.** Not yet driven by a real memory map
//! on real hardware; see `documentation/STATUS.md`.

#![cfg_attr(not(test), no_std)]

use loko_boot_protocol::MemoryRegion;

/// The size of a physical frame, in bytes.
pub const FRAME_SIZE: u64 = 4096;

/// A physical frame number: a physical address divided by [`FRAME_SIZE`].
///
/// Using a frame number rather than an address makes the "is this aligned?"
/// question unrepresentable instead of merely checked.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[repr(transparent)]
pub struct FrameNumber(pub u64);

impl FrameNumber {
    /// The frame containing `address`.
    #[must_use]
    pub const fn containing(address: u64) -> Self {
        FrameNumber(address / FRAME_SIZE)
    }

    /// The physical address of the start of this frame.
    #[must_use]
    pub const fn start_address(self) -> u64 {
        self.0 * FRAME_SIZE
    }
}

/// Why an allocation or free failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryError {
    /// No run of free frames long enough was available.
    OutOfMemory,
    /// The frame is outside the range the allocator manages.
    OutOfRange,
    /// The frame was already free. Almost always a double free, which is a bug
    /// worth failing loudly for rather than absorbing.
    NotAllocated,
    /// The bitmap storage passed in is too small for the address range.
    BitmapTooSmall,
}

/// A bitmap physical frame allocator.
///
/// Manages frames in `[base, base + frames)`. A set bit means allocated.
/// Borrows its bitmap storage rather than allocating it, because at the moment
/// this type is constructed there is nothing to allocate from.
#[derive(Debug)]
pub struct FrameAllocator<'a> {
    bitmap: &'a mut [u64],
    /// The first frame this allocator manages.
    base: FrameNumber,
    /// How many frames it manages.
    frames: u64,
    /// Frames currently allocated. Maintained incrementally; checked against a
    /// full recount by [`FrameAllocator::audit`].
    allocated: u64,
    /// Where the last search stopped, so that sequential allocation does not
    /// rescan from zero every time.
    cursor: u64,
}

/// The number of `u64` words a bitmap needs to cover `frames` frames.
#[must_use]
pub const fn bitmap_words_for(frames: u64) -> usize {
    frames.div_ceil(64) as usize
}

impl<'a> FrameAllocator<'a> {
    /// Creates an allocator covering `[base, base + frames)` with everything
    /// marked allocated.
    ///
    /// Starting fully allocated rather than fully free is deliberate: memory
    /// becomes usable only by being explicitly released from a region the
    /// firmware called usable. A bug in the release path then costs us memory,
    /// which is visible, rather than handing out firmware-reserved frames,
    /// which is not.
    pub fn new(bitmap: &'a mut [u64], base: FrameNumber, frames: u64) -> Result<Self, MemoryError> {
        if bitmap.len() < bitmap_words_for(frames) {
            return Err(MemoryError::BitmapTooSmall);
        }
        for word in bitmap.iter_mut() {
            *word = u64::MAX;
        }
        Ok(FrameAllocator {
            bitmap,
            base,
            frames,
            allocated: frames,
            cursor: 0,
        })
    }

    /// Builds an allocator from a firmware memory map, releasing every region
    /// that is usable at handoff.
    ///
    /// Regions are clamped to the allocator's range, so a firmware entry that
    /// extends past the end of managed memory contributes its overlapping part
    /// instead of being dropped or causing an overflow.
    pub fn from_memory_map(
        bitmap: &'a mut [u64],
        regions: &[MemoryRegion],
    ) -> Result<Self, MemoryError> {
        let highest = regions.iter().map(|r| r.end()).max().unwrap_or(0);
        let frames = highest / FRAME_SIZE;
        let mut allocator = FrameAllocator::new(bitmap, FrameNumber(0), frames)?;

        for region in regions.iter().filter(|r| r.kind.usable_at_handoff()) {
            let first = FrameNumber::containing(region.start);
            let count = region.len / FRAME_SIZE;
            for i in 0..count {
                // A region beyond the managed range is skipped rather than
                // treated as an error: firmware maps sometimes describe memory
                // holes above the highest usable address.
                let _ = allocator.release(FrameNumber(first.0 + i));
            }
        }
        Ok(allocator)
    }

    /// The index of the bit for `frame`, or `None` if out of range.
    const fn index_of(&self, frame: FrameNumber) -> Option<u64> {
        if frame.0 < self.base.0 {
            return None;
        }
        let index = frame.0 - self.base.0;
        if index >= self.frames {
            return None;
        }
        Some(index)
    }

    fn get(&self, index: u64) -> bool {
        self.bitmap[(index / 64) as usize] & (1 << (index % 64)) != 0
    }

    fn set(&mut self, index: u64, allocated: bool) {
        let word = &mut self.bitmap[(index / 64) as usize];
        let mask = 1u64 << (index % 64);
        if allocated {
            *word |= mask;
        } else {
            *word &= !mask;
        }
    }

    /// Marks a frame as free and available.
    ///
    /// Returns `Err(OutOfRange)` for a frame this allocator does not manage,
    /// and is idempotent for a frame that is already free — releasing memory
    /// that was never allocated is how a memory map is consumed, not a bug.
    pub fn release(&mut self, frame: FrameNumber) -> Result<(), MemoryError> {
        let index = self.index_of(frame).ok_or(MemoryError::OutOfRange)?;
        if self.get(index) {
            self.set(index, false);
            self.allocated -= 1;
        }
        Ok(())
    }

    /// Marks a frame as permanently unavailable.
    ///
    /// Used for the kernel image, the boot info, and the framebuffer.
    pub fn reserve(&mut self, frame: FrameNumber) -> Result<(), MemoryError> {
        let index = self.index_of(frame).ok_or(MemoryError::OutOfRange)?;
        if !self.get(index) {
            self.set(index, true);
            self.allocated += 1;
        }
        Ok(())
    }

    /// Marks every frame overlapping `[start, start + len)` as unavailable.
    pub fn reserve_range(&mut self, start: u64, len: u64) {
        if len == 0 {
            return;
        }
        let first = FrameNumber::containing(start);
        // The last byte, not one past the end, so a range ending exactly on a
        // frame boundary does not reserve an extra frame.
        let last = FrameNumber::containing(start.saturating_add(len - 1));
        for f in first.0..=last.0 {
            let _ = self.reserve(FrameNumber(f));
        }
    }

    /// Allocates one frame.
    pub fn allocate(&mut self) -> Result<FrameNumber, MemoryError> {
        self.allocate_contiguous(1)
    }

    /// Allocates `count` physically contiguous frames.
    ///
    /// Needed for DMA buffers and for page tables that hardware walks.
    pub fn allocate_contiguous(&mut self, count: u64) -> Result<FrameNumber, MemoryError> {
        if count == 0 || count > self.frames {
            return Err(MemoryError::OutOfMemory);
        }

        // Two passes: a fast one from the cursor, then a complete one from
        // zero. The second pass deliberately rescans what the first already
        // covered rather than stopping at the cursor — a run that begins below
        // the cursor and extends past it belongs to neither half, and stopping
        // early would report out-of-memory while such a run exists. The cost is
        // paid only on the path that would otherwise fail.
        let start_points = [self.cursor, 0];
        for start in start_points {
            let limit = self.frames;
            let mut index = start;
            while index + count <= limit {
                match self.first_allocated_in(index, index + count) {
                    // Every bit in the window is clear: take it.
                    None => {
                        for i in index..index + count {
                            self.set(i, true);
                        }
                        self.allocated += count;
                        self.cursor = index + count;
                        return Ok(FrameNumber(self.base.0 + index));
                    }
                    // Restart just past the blocker rather than one frame on.
                    Some(blocker) => index = blocker + 1,
                }
            }
        }
        Err(MemoryError::OutOfMemory)
    }

    /// The first allocated bit in `[from, to)`, if any.
    fn first_allocated_in(&self, from: u64, to: u64) -> Option<u64> {
        (from..to).find(|&i| self.get(i))
    }

    /// Frees one frame.
    ///
    /// Returns [`MemoryError::NotAllocated`] on a double free rather than
    /// ignoring it.
    pub fn free(&mut self, frame: FrameNumber) -> Result<(), MemoryError> {
        let index = self.index_of(frame).ok_or(MemoryError::OutOfRange)?;
        if !self.get(index) {
            return Err(MemoryError::NotAllocated);
        }
        self.set(index, false);
        self.allocated -= 1;
        // Bias the next search toward reusing what was just freed, which keeps
        // allocations clustered and the working set small.
        if index < self.cursor {
            self.cursor = index;
        }
        Ok(())
    }

    /// Frees `count` frames starting at `frame`.
    pub fn free_contiguous(&mut self, frame: FrameNumber, count: u64) -> Result<(), MemoryError> {
        // Validate the whole run before changing anything, so a partially
        // invalid free does not leave the bitmap half-updated.
        for i in 0..count {
            let index = self
                .index_of(FrameNumber(frame.0 + i))
                .ok_or(MemoryError::OutOfRange)?;
            if !self.get(index) {
                return Err(MemoryError::NotAllocated);
            }
        }
        for i in 0..count {
            self.free(FrameNumber(frame.0 + i))?;
        }
        Ok(())
    }

    /// Whether `frame` is currently allocated.
    pub fn is_allocated(&self, frame: FrameNumber) -> Option<bool> {
        self.index_of(frame).map(|i| self.get(i))
    }

    /// Total frames managed.
    #[must_use]
    pub const fn total_frames(&self) -> u64 {
        self.frames
    }

    /// Frames currently allocated or reserved.
    #[must_use]
    pub const fn allocated_frames(&self) -> u64 {
        self.allocated
    }

    /// Frames available.
    #[must_use]
    pub const fn free_frames(&self) -> u64 {
        self.frames - self.allocated
    }

    /// Bytes available.
    #[must_use]
    pub const fn free_bytes(&self) -> u64 {
        self.free_frames() * FRAME_SIZE
    }

    /// Recounts the bitmap and compares it with the running total.
    ///
    /// Cheap enough to run at every boot and after every test. An allocator
    /// whose count has drifted from its bitmap is corrupt, and it is far better
    /// to find that out here than through a symptom elsewhere.
    #[must_use]
    pub fn audit(&self) -> bool {
        let counted: u64 = (0..self.frames).filter(|&i| self.get(i)).count() as u64;
        counted == self.allocated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loko_boot_protocol::MemoryKind;

    fn allocator(frames: u64) -> (Vec<u64>, FrameNumber, u64) {
        (vec![0u64; bitmap_words_for(frames)], FrameNumber(0), frames)
    }

    fn region(start: u64, len: u64, kind: MemoryKind) -> MemoryRegion {
        MemoryRegion {
            start,
            len,
            kind,
            _reserved: 0,
        }
    }

    #[test]
    fn a_new_allocator_owns_nothing() {
        let (mut bits, base, frames) = allocator(1024);
        let a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        assert_eq!(a.free_frames(), 0, "memory must be released explicitly");
        assert_eq!(a.allocated_frames(), 1024);
        assert!(a.audit());
    }

    #[test]
    fn a_bitmap_that_is_too_small_is_refused() {
        let mut bits = vec![0u64; 1];
        assert_eq!(
            FrameAllocator::new(&mut bits, FrameNumber(0), 1024).unwrap_err(),
            MemoryError::BitmapTooSmall
        );
    }

    #[test]
    fn released_frames_become_allocatable() {
        let (mut bits, base, frames) = allocator(64);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        a.release(FrameNumber(10)).unwrap();
        assert_eq!(a.free_frames(), 1);
        assert_eq!(a.allocate().unwrap(), FrameNumber(10));
        assert_eq!(a.free_frames(), 0);
        assert_eq!(a.allocate().unwrap_err(), MemoryError::OutOfMemory);
        assert!(a.audit());
    }

    #[test]
    fn a_frame_is_never_handed_out_twice() {
        let (mut bits, base, frames) = allocator(256);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        for f in 0..256 {
            a.release(FrameNumber(f)).unwrap();
        }

        let mut seen = std::collections::HashSet::new();
        while let Ok(frame) = a.allocate() {
            assert!(seen.insert(frame), "{frame:?} was handed out twice");
        }
        assert_eq!(seen.len(), 256);
        assert_eq!(a.free_frames(), 0);
        assert!(a.audit());
    }

    #[test]
    fn a_double_free_is_reported_not_absorbed() {
        let (mut bits, base, frames) = allocator(64);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        a.release(FrameNumber(0)).unwrap();
        let f = a.allocate().unwrap();
        a.free(f).unwrap();
        assert_eq!(
            a.free(f).unwrap_err(),
            MemoryError::NotAllocated,
            "a double free must be an error, not a silent corruption"
        );
        assert!(a.audit());
    }

    #[test]
    fn out_of_range_frames_are_rejected() {
        let (mut bits, base, frames) = allocator(64);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        assert_eq!(
            a.release(FrameNumber(64)).unwrap_err(),
            MemoryError::OutOfRange
        );
        assert_eq!(a.is_allocated(FrameNumber(64)), None);
        assert_eq!(a.is_allocated(FrameNumber(63)), Some(true));
    }

    #[test]
    fn contiguous_allocation_returns_a_real_run() {
        let (mut bits, base, frames) = allocator(128);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        for f in 0..128 {
            a.release(FrameNumber(f)).unwrap();
        }
        // Fragment: reserve every eighth frame.
        for f in (0..128).step_by(8) {
            a.reserve(FrameNumber(f)).unwrap();
        }
        let run = a.allocate_contiguous(7).unwrap();
        for i in 0..7 {
            assert_eq!(
                a.is_allocated(FrameNumber(run.0 + i)),
                Some(true),
                "frame {} of the run is not marked allocated",
                run.0 + i
            );
        }
        // An 8-frame run cannot exist in this fragmentation pattern.
        assert_eq!(
            a.allocate_contiguous(8).unwrap_err(),
            MemoryError::OutOfMemory
        );
        assert!(a.audit());
    }

    #[test]
    fn contiguous_allocation_never_spans_an_allocated_frame() {
        let (mut bits, base, frames) = allocator(64);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        for f in 0..64 {
            a.release(FrameNumber(f)).unwrap();
        }
        a.reserve(FrameNumber(32)).unwrap();

        // Exhaust everything in runs of 4 and check none straddles frame 32.
        while let Ok(run) = a.allocate_contiguous(4) {
            assert!(
                !(run.0..run.0 + 4).contains(&32),
                "a run at {} spans the reserved frame 32",
                run.0
            );
        }
        assert!(a.audit());
    }

    #[test]
    fn allocation_wraps_to_reuse_freed_low_frames() {
        let (mut bits, base, frames) = allocator(16);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        for f in 0..16 {
            a.release(FrameNumber(f)).unwrap();
        }
        // Consume everything, pushing the cursor to the end.
        for _ in 0..16 {
            a.allocate().unwrap();
        }
        // Free a low frame; the allocator must find it rather than reporting
        // out-of-memory because its cursor is past it.
        a.free(FrameNumber(2)).unwrap();
        assert_eq!(a.allocate().unwrap(), FrameNumber(2));
        assert!(a.audit());
    }

    #[test]
    fn reserve_range_covers_partial_frames_at_both_ends() {
        let (mut bits, base, frames) = allocator(16);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        for f in 0..16 {
            a.release(FrameNumber(f)).unwrap();
        }
        // One byte into frame 1, running one byte into frame 3.
        a.reserve_range(FRAME_SIZE + 1, FRAME_SIZE * 2);
        assert_eq!(a.is_allocated(FrameNumber(0)), Some(false));
        assert_eq!(a.is_allocated(FrameNumber(1)), Some(true));
        assert_eq!(a.is_allocated(FrameNumber(2)), Some(true));
        assert_eq!(a.is_allocated(FrameNumber(3)), Some(true));
        assert_eq!(a.is_allocated(FrameNumber(4)), Some(false));
        assert!(a.audit());
    }

    #[test]
    fn a_range_ending_on_a_frame_boundary_does_not_reserve_one_extra() {
        let (mut bits, base, frames) = allocator(8);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        for f in 0..8 {
            a.release(FrameNumber(f)).unwrap();
        }
        a.reserve_range(0, FRAME_SIZE * 2);
        assert_eq!(a.is_allocated(FrameNumber(1)), Some(true));
        assert_eq!(
            a.is_allocated(FrameNumber(2)),
            Some(false),
            "an exact two-frame range must not reserve a third frame"
        );
    }

    #[test]
    fn a_firmware_memory_map_yields_exactly_the_usable_frames() {
        let map = [
            region(0, FRAME_SIZE * 16, MemoryKind::Reserved),
            region(FRAME_SIZE * 16, FRAME_SIZE * 100, MemoryKind::Usable),
            region(FRAME_SIZE * 116, FRAME_SIZE * 4, MemoryKind::AcpiNvs),
            region(
                FRAME_SIZE * 120,
                FRAME_SIZE * 8,
                MemoryKind::BootloaderReclaimable,
            ),
        ];
        let total_frames = 128;
        let mut bits = vec![0u64; bitmap_words_for(total_frames)];
        let a = FrameAllocator::from_memory_map(&mut bits, &map).unwrap();

        assert_eq!(a.total_frames(), total_frames);
        assert_eq!(
            a.free_frames(),
            100,
            "only the Usable region should be available at handoff"
        );
        assert_eq!(a.free_bytes(), 100 * FRAME_SIZE);
        // Bootloader memory holds the BootInfo we are reading; it must not be
        // allocatable yet even though it is eventually reclaimable.
        assert_eq!(a.is_allocated(FrameNumber(120)), Some(true));
        assert_eq!(a.is_allocated(FrameNumber(0)), Some(true));
        assert_eq!(a.is_allocated(FrameNumber(16)), Some(false));
        assert!(a.audit());
    }

    #[test]
    fn frame_numbers_and_addresses_agree() {
        assert_eq!(FrameNumber::containing(0).0, 0);
        assert_eq!(FrameNumber::containing(4095).0, 0);
        assert_eq!(FrameNumber::containing(4096).0, 1);
        assert_eq!(FrameNumber(3).start_address(), 3 * FRAME_SIZE);
    }

    #[test]
    fn bitmap_sizing_rounds_up() {
        assert_eq!(bitmap_words_for(0), 0);
        assert_eq!(bitmap_words_for(1), 1);
        assert_eq!(bitmap_words_for(64), 1);
        assert_eq!(bitmap_words_for(65), 2);
        // One gigabyte of RAM costs 32 KiB of bitmap.
        assert_eq!(bitmap_words_for(262_144) * 8, 32 * 1024);
    }

    #[test]
    fn the_audit_catches_a_desynchronised_count() {
        // Sanity check on the check itself: if `audit` cannot fail, it is not
        // testing anything.
        let (mut bits, base, frames) = allocator(64);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        a.release(FrameNumber(0)).unwrap();
        assert!(a.audit());
        a.set(1, false); // corrupt the bitmap behind the counter's back
        assert!(!a.audit());
    }

    #[test]
    fn a_partially_invalid_contiguous_free_changes_nothing() {
        let (mut bits, base, frames) = allocator(16);
        let mut a = FrameAllocator::new(&mut bits, base, frames).unwrap();
        for f in 0..16 {
            a.release(FrameNumber(f)).unwrap();
        }
        let run = a.allocate_contiguous(4).unwrap();
        // Ask to free one frame more than was allocated.
        assert_eq!(
            a.free_contiguous(run, 5).unwrap_err(),
            MemoryError::NotAllocated
        );
        for i in 0..4 {
            assert_eq!(
                a.is_allocated(FrameNumber(run.0 + i)),
                Some(true),
                "the failed free released frame {} anyway",
                run.0 + i
            );
        }
        assert!(a.audit());
    }
}
