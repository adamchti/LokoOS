# ADR 0006 — Where the kernel lives in the address space

**Status:** Accepted · **Date:** 2026-09-18

## Context

An x86-64 kernel has to choose a virtual address range. The choice affects the
code model the compiler can use, how cheaply a kernel address can be told apart
from a user address, and whether the kernel needs its own page tables switched
in on every system call.

## Decision

The kernel is linked at **`0xFFFFFFFF80000000`** — the top 2 GiB of the
canonical higher half. All of physical memory is mapped at
**`0xFFFF800000000000`**, the first address of the higher half.

Three reasons:

1. **A user address is "not a kernel address" by a single sign test.** In the
   canonical split, every kernel address has bit 63 set and every user address
   does not. Validating a pointer from userland is one comparison, on a path
   that runs on every system call.

2. **The top 2 GiB permits the `kernel` code model.** Every intra-kernel
   reference fits in a 32-bit signed displacement, so calls and global accesses
   are one instruction shorter than the alternatives. This is the standard
   reason Linux and the BSDs make the same choice.

3. **The kernel stays mapped in every address space**, so a system call is a
   privilege transition and not a page-table switch. (Page-table isolation
   against speculative-execution attacks is a separate decision, to be made when
   there is a userland to isolate.)

The physical-memory window exists to break a bootstrapping cycle: allocating a
page table requires writing to a physical frame, which without the window would
require mapping it, which requires a page table. With the window, the kernel
reaches any frame by adding an offset.

The offset is **passed in `BootInfo`**, not compiled in as a constant, so it can
be randomised later without changing the kernel.

## Consequences

`linker.ld` and `loko_boot_protocol::KERNEL_VIRTUAL_BASE` must agree. They are
two files, which is a drift risk; the kernel records where it was linked in
`BootInfo` so a mismatch shows up as a boot-time value rather than as a fault.

The physical-memory window is mapped with 2 MiB pages, read-write and
no-execute. It costs one page-directory frame per gigabyte of RAM. 1 GiB pages
would cost less but need a CPUID check that the early boot path does not yet do.

The window is writable, which means a kernel bug can corrupt any physical frame
through it. That is inherent to having such a window at all; the alternative is
mapping frames on demand, which reintroduces the bootstrapping cycle.
