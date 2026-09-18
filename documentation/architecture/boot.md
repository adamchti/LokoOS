# Booting LokoOS

Requirement 6 lists fourteen boot stages. This describes the five that exist,
and is honest about where the sequence currently stops.

| Stage | What happens | Status |
|---|---|---|
| 1 | Firmware initialisation | Firmware's job |
| 2 | `loko-boot.efi` reads the kernel off the ESP | Implemented |
| 3 | Memory reserved, kernel image loaded and relocated | Implemented |
| 4 | Hardware queried, page tables built, `ExitBootServices` | Implemented |
| 5 | Kernel entered, `BootInfo` validated | Implemented |
| 6 | GDT, TSS, IDT | Implemented |
| 7 | Physical frame allocator | Implemented |
| 8 | Kernel heap | Implemented |
| 9 | Scheduler, address spaces, storage | **Not started** |
| 10–14 | Graphics, window manager, desktop, AI services, user session | Not started |

**Stages 1 to 8 are verified running** under QEMU with OVMF, on every push. The
serial log is in [STATUS.md](../STATUS.md). Real hardware is still untried.

---

## Stage 2–4: the bootloader

`boot/loko-boot` is a UEFI application. Its whole job is to put the machine in a
state the kernel can start from, and then get out of the way.

### Reading the kernel

The kernel lives at `\EFI\LOKO\loko-kernel` on the EFI system partition. The ESP
is FAT, so it cannot carry LKO's structure or permissions; it holds only what
firmware must be able to read. `LKO/Boot` is the LKOFS view of this same
directory.

The bootloader also installs itself at `\EFI\BOOT\BOOTX64.EFI`, the name firmware
looks for when nothing is registered in NVRAM — the case on a freshly installed
machine and in every emulator.

### Parsing the image

`elf.rs` is a deliberately minimal ELF64 reader. It understands exactly what
`kernel/linker.ld` produces: a static `ET_EXEC` x86-64 executable with a few
`PT_LOAD` segments.

Every field is read with `from_le_bytes` out of a byte slice rather than by
casting the buffer to a `#[repr(C)]` struct. The file came off a FAT partition
that anything could have written, so it is data, not a value, and there is no
alignment to assume.

Everything is validated before anything is loaded, so loading either happens
completely or does not start. A segment asking to be both writable and
executable is a build error and is rejected.

### Reserving memory

Everything the handover needs is allocated **while an allocator still exists** —
after `ExitBootServices` there is no way to obtain another byte:

| Allocation | Size | For |
|---|---|---|
| Kernel image | image extent | The loaded kernel |
| Page-table arena | 256 pages (1 MiB) | All four levels of tables |
| Kernel stack | 16 pages (64 KiB) | The kernel's first thread |
| Memory-map array | 8 pages | The converted map, up to 1365 regions |
| Boot info | 1 page | The `BootInfo` structure |

UEFI does not promise zeroed pages, and the kernel's `.bss` depends on getting
them, so every allocation is zeroed explicitly.

### Building the page tables

Three mappings, in one fresh four-level table:

1. **The kernel image**, at its linked address, at 4 KiB granularity so that each
   ELF segment gets the permissions it declared. Code is not writable; data is
   not executable. **W^X holds from the first instruction the kernel executes**,
   not from some later hardening step.

2. **All installed RAM** at `0xFFFF800000000000`, with 2 MiB pages, read-write
   and no-execute. Installed RAM, not the whole address space: firmware
   describes apertures far above memory — QEMU puts a PCI hole at 1 TiB — and
   mapping those would need a thousand page-directory frames to describe
   address space that holds nothing. The framebuffer is MMIO and therefore
   outside that range, so it is mapped separately.

3. **The low 4 GiB, identity-mapped, read-execute.** This exists for exactly one
   reason: the instruction after `mov cr3` is fetched from the address it was
   already executing at, which is a low identity address. Without this mapping
   the switch faults on its own next instruction.

   **Read-execute, not read-write.** Marking it no-execute makes that same
   instruction fetch fault with no handler installed, which is a triple fault
   and a machine that silently resets. This cost one CI run to find and is the
   single least obvious thing in the boot path. Nothing writes through an
   identity address between the `cr3` load and the jump, so read-execute keeps
   W^X intact.

   The kernel is supposed to drop this mapping once it is running from the
   higher half, and currently does not — see [STATUS.md](../STATUS.md).

Intermediate entries are always writable and always executable. On x86-64 the
effective permission is the AND of the writable bits and the OR of the
no-execute bits down the whole walk, so restricting an intermediate level would
restrict every leaf under it. The restriction belongs on the leaf, where it
describes one page.

`EFER.NXE` is set explicitly before any no-execute mapping is used. UEFI usually
sets it. "Usually" is not a basis for setting a bit in every data mapping in the
system.

### Exiting and handing over

After `ExitBootServices` nothing may allocate, log, or call a boot service. The
bootloader converts the final firmware memory map into the protocol's form —
collapsing UEFI's memory types onto LokoOS's, dropping zero-length regions, and
insertion-sorting by address because UEFI does not promise an ordered map.

The jump:

```
mov cr3, <page tables>
mov rsp, <kernel stack, in the physical-memory window>
xor rbp, rbp          ; terminate the call chain for stack walkers
jmp <kernel entry>    ; with BootInfo in rdi, per the SysV C ABI
```

The stack pointer and the boot-info pointer are both expressed in the
physical-memory window rather than as identity addresses, so they stay valid
once the identity map goes away.

---

## Stage 5–8: the kernel

### Validation before trust

Every field of `BootInfo` is untrusted until `validate()` returns `Ok`. On a real
machine these values come from firmware, and firmware is not always right.

Checks run in order of how much machinery they need, so the most likely
catastrophic case — a wrong pointer — is caught by a single comparison:

1. Magic number (`LOKOBOOT`, readable in a hex dump)
2. Protocol major version
3. Structure size
4. Memory map pointer and length
5. Physical-offset alignment
6. Every region: aligned, non-empty, non-wrapping, sorted, non-overlapping
7. At least one usable region

Each failure has a message written for whoever is standing in front of the
machine, and several name the recovery action.

### Descriptor tables first

The GDT is set up before the IDT, because IDT entries name a code segment
selector and that selector must be valid by the time an interrupt fires.

The TSS carries three separate Interrupt Stack Table entries: double fault, page
fault, and NMI. This is the difference between a diagnosable error and a silent
reboot. A kernel that takes a page fault *because its stack is the problem* will
double-fault trying to push the fault frame, then triple-fault — which on x86
resets the machine with no message at all.

### Physical memory

The frame allocator is built from the firmware map, then everything in use is
reserved: the kernel image, the framebuffer, all bootloader-reclaimable regions,
and physical frame zero — so that a null pointer stays distinguishable from a
valid allocation at address zero.

The allocator then audits itself: it recounts the bitmap and compares it with
its running total. Cheap enough to do at every boot, and an allocator whose count
has drifted from its bitmap is far better found here than through a symptom
elsewhere.

### The heap

The kernel allocates physically contiguous frames and uses their addresses in the
physical-memory window directly, rather than building a dedicated virtual
mapping. That removes page-table manipulation from the earliest and least
debuggable part of boot, at the cost of the heap being wherever the allocator
put it — which nothing depends on.

1 MiB. Almost nothing in the kernel should need dynamic allocation; the size is
a budget, not a floor to grow from.

### And then it stops

There is no scheduler, no address-space manager and no storage stack, so there
is nothing to hand control to. The kernel says exactly that on the serial console
and halts:

```
[INFO ] boot       stage 9: not implemented; no scheduler or storage stack yet
[INFO ] boot       kernel initialised successfully and is halting
```
