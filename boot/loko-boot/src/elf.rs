//! A minimal ELF64 reader, enough to load the LokoOS kernel.
//!
//! Deliberately not a general-purpose ELF library. It understands exactly the
//! kind of file `kernel/linker.ld` produces: a static, non-relocatable
//! executable with a handful of `PT_LOAD` segments. Anything else is rejected
//! rather than half-understood, because a bootloader that guesses about a
//! malformed kernel image is a bootloader that jumps somewhere arbitrary.
//!
//! Every field is read with `from_le_bytes` out of a byte slice rather than by
//! casting the buffer to a struct. The file comes off a FAT partition that
//! anything could have written, so it is data, not a `#[repr(C)]` value, and
//! there is no alignment to assume.

/// Flags on a program header.
pub mod flags {
    /// Segment is executable.
    pub const EXECUTE: u32 = 1;
    /// Segment is writable.
    pub const WRITE: u32 = 2;
    /// Segment is readable.
    ///
    /// Nothing branches on this: every mapping LokoOS makes is readable, and a
    /// `PT_LOAD` segment without it would be nonsensical. It is defined anyway,
    /// because a flags module that documents two of three bits documents
    /// nothing.
    #[allow(dead_code)]
    pub const READ: u32 = 4;
}

/// `PT_LOAD`: a segment that must be copied into memory.
const PT_LOAD: u32 = 1;

const EI_NIDENT: usize = 16;
const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;

const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 0x3E;

/// Why an image was rejected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ElfError {
    /// Shorter than an ELF header.
    TooSmall,
    /// Wrong magic number: this is not an ELF file at all.
    NotElf,
    /// Not a 64-bit, little-endian, x86-64 executable.
    WrongFormat,
    /// Not `ET_EXEC`. LokoOS links its kernel non-relocatable, and silently
    /// accepting a `ET_DYN` image would mean loading it without applying the
    /// relocations it needs.
    NotAnExecutable,
    /// A program header table that runs past the end of the file.
    TruncatedHeaders,
    /// A segment whose contents run past the end of the file.
    TruncatedSegment,
    /// A segment whose in-memory size is smaller than its on-disk size.
    ImpossibleSegment,
    /// No loadable segments at all.
    NothingToLoad,
}

impl ElfError {
    /// A message for whoever is looking at the screen.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            ElfError::TooSmall | ElfError::NotElf | ElfError::WrongFormat => {
                "The LokoOS system file on this device isn't valid."
            }
            ElfError::NotAnExecutable => {
                "The LokoOS system file on this device was built incorrectly."
            }
            ElfError::TruncatedHeaders
            | ElfError::TruncatedSegment
            | ElfError::ImpossibleSegment => {
                "The LokoOS system file on this device is damaged or incomplete."
            }
            ElfError::NothingToLoad => "The LokoOS system file on this device is empty.",
        }
    }
}

/// One loadable segment.
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    /// Offset of the segment's contents within the file.
    pub file_offset: usize,
    /// Number of bytes present in the file.
    pub file_size: usize,
    /// Virtual address the segment is linked for.
    pub virtual_address: u64,
    /// Number of bytes the segment occupies in memory. Any excess over
    /// `file_size` is `.bss` and must be zeroed.
    pub memory_size: u64,
    /// `PF_*` permission bits.
    pub flags: u32,
}

impl Segment {
    /// Whether the segment must be executable.
    #[must_use]
    pub const fn is_executable(&self) -> bool {
        self.flags & flags::EXECUTE != 0
    }

    /// Whether the segment must be writable.
    #[must_use]
    pub const fn is_writable(&self) -> bool {
        self.flags & flags::WRITE != 0
    }
}

/// A parsed kernel image.
pub struct Image<'a> {
    data: &'a [u8],
    /// The virtual address to jump to.
    pub entry: u64,
    program_header_offset: usize,
    program_header_count: usize,
    program_header_size: usize,
}

impl<'a> Image<'a> {
    /// Parses and validates an ELF64 executable.
    pub fn parse(data: &'a [u8]) -> Result<Self, ElfError> {
        if data.len() < ELF_HEADER_SIZE {
            return Err(ElfError::TooSmall);
        }
        if data[0..4] != [0x7F, b'E', b'L', b'F'] {
            return Err(ElfError::NotElf);
        }
        if data[4] != ELFCLASS64 || data[5] != ELFDATA2LSB {
            return Err(ElfError::WrongFormat);
        }
        if read_u16(data, 0x12) != EM_X86_64 {
            return Err(ElfError::WrongFormat);
        }
        if read_u16(data, 0x10) != ET_EXEC {
            return Err(ElfError::NotAnExecutable);
        }

        let entry = read_u64(data, 0x18);
        let program_header_offset = read_u64(data, 0x20) as usize;
        let program_header_size = read_u16(data, 0x36) as usize;
        let program_header_count = read_u16(data, 0x38) as usize;

        if program_header_size < PROGRAM_HEADER_SIZE {
            return Err(ElfError::WrongFormat);
        }
        // Checked rather than assumed: a crafted header count could otherwise
        // walk this loader off the end of the buffer.
        let table_end = program_header_offset
            .checked_add(
                program_header_size
                    .checked_mul(program_header_count)
                    .ok_or(ElfError::TruncatedHeaders)?,
            )
            .ok_or(ElfError::TruncatedHeaders)?;
        if table_end > data.len() {
            return Err(ElfError::TruncatedHeaders);
        }

        let image = Image {
            data,
            entry,
            program_header_offset,
            program_header_count,
            program_header_size,
        };

        // Validate every segment up front, so that loading either happens
        // completely or does not start.
        let mut loadable = 0;
        for index in 0..image.program_header_count {
            if let Some(segment) = image.segment_at(index)? {
                loadable += 1;
                let _ = segment;
            }
        }
        if loadable == 0 {
            return Err(ElfError::NothingToLoad);
        }

        Ok(image)
    }

    /// The `index`-th program header, if it is loadable.
    fn segment_at(&self, index: usize) -> Result<Option<Segment>, ElfError> {
        let base = self.program_header_offset + index * self.program_header_size;
        let kind = read_u32(self.data, base);
        if kind != PT_LOAD {
            return Ok(None);
        }

        let flags = read_u32(self.data, base + 0x04);
        let file_offset = read_u64(self.data, base + 0x08) as usize;
        let virtual_address = read_u64(self.data, base + 0x10);
        let file_size = read_u64(self.data, base + 0x20) as usize;
        let memory_size = read_u64(self.data, base + 0x28);

        if memory_size < file_size as u64 {
            return Err(ElfError::ImpossibleSegment);
        }
        let end = file_offset
            .checked_add(file_size)
            .ok_or(ElfError::TruncatedSegment)?;
        if end > self.data.len() {
            return Err(ElfError::TruncatedSegment);
        }

        Ok(Some(Segment {
            file_offset,
            file_size,
            virtual_address,
            memory_size,
            flags,
        }))
    }

    /// Every loadable segment.
    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        (0..self.program_header_count).filter_map(move |i| self.segment_at(i).ok().flatten())
    }

    /// The bytes of a segment as they appear in the file.
    #[must_use]
    pub fn segment_data(&self, segment: &Segment) -> &'a [u8] {
        &self.data[segment.file_offset..segment.file_offset + segment.file_size]
    }

    /// The lowest and highest virtual addresses any segment occupies.
    ///
    /// Used to work out how much contiguous physical memory to allocate.
    #[must_use]
    pub fn virtual_extent(&self) -> (u64, u64) {
        let mut lowest = u64::MAX;
        let mut highest = 0u64;
        for segment in self.segments() {
            lowest = lowest.min(segment.virtual_address);
            highest = highest.max(segment.virtual_address + segment.memory_size);
        }
        if lowest == u64::MAX {
            (0, 0)
        } else {
            (lowest, highest)
        }
    }
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(bytes)
}

/// Silences the unused-constant warning for `EI_NIDENT`, which documents the
/// layout even though the fields inside the identifier are read individually.
const _: () = assert!(EI_NIDENT == 16);
