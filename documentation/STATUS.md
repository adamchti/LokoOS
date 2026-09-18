# LokoOS status

**Version 0.1.0 · Last updated 2026-09-18**

This is the honest ledger. Requirement 69 of the LokoOS design says: do not
create fake functionality and present it as real. That rule is worth nothing
without a place where the real state of every part is written down, so this is
that place.

Every claim in this file is checkable. Anything marked **Implemented** compiles
and, where the code is testable on a host machine, is covered by tests you can
run with `./tools/build.ps1 test`. Anything not marked **Implemented** is not
finished, and the code says so at the point where it stops.

---

## What actually runs today

| | |
|---|---|
| Host test suite | **142 tests, all passing** |
| Kernel | **Builds** to a valid higher-half ELF64 for `x86_64-unknown-none` |
| Bootloader | **Builds** to a valid PE32+ EFI application for `x86_64-unknown-uefi` |
| Booted on hardware or in an emulator | **No.** See "The honest gap" below |

### The honest gap

**LokoOS has never been booted.** The development machine this was built on has
no emulator installed and no spare disk space to install one, so the boot path —
the bootloader finding the kernel, loading it, switching page tables, and the
kernel coming up on the other side — has been written and compiled but never
observed working.

That is a real and significant gap, and nothing in this repository pretends
otherwise. What *is* verified about the boot chain:

- The kernel links to `0xFFFFFFFF80000000` with three `PT_LOAD` segments whose
  permissions are `R-X`, `R--`, `RW-`. No segment is both writable and
  executable. Check it yourself: `./tools/elfinfo.ps1 build/esp/EFI/LOKO/loko-kernel`
- The kernel's `.bss` (596 KiB, mostly the frame-allocator bitmap) is correctly
  uncommitted on disk: the `RW-` segment's memory size exceeds its file size.
- The bootloader is a PE32+ image with subsystem 10, `EFI_APPLICATION`, which is
  what firmware will load.
- The frame allocator, the boot-info validator, the ELF size arithmetic and the
  memory-map conversion are all exercised by host tests.

What is **not** verified: that any of it works on a machine. To close this gap:

```bash
# Install QEMU and OVMF, then:
./tools/build.ps1 image
qemu-system-x86_64 -bios OVMF.fd -drive format=raw,file=fat:rw:build/esp -serial stdio
```

The kernel's entire boot narration goes to the serial port, so `-serial stdio`
shows exactly where it gets to.

---

## Per-component status

Legend: **Implemented** (written, compiles, tested where testable) ·
**Partial** (some of it works, the rest returns an explicit error) ·
**Specified** (designed and documented, no code) · **Not started**

### Foundations

| Component | Status | Notes |
|---|---|---|
| Repository and build system | Implemented | Cargo workspace, `xtask` driver, three targets |
| LKO filesystem model (§7) | Implemented | 19 roots, protection classes, storage categories |
| LKO path model (§8) | Implemented | Parsing, validation, containment. 30 tests |
| LKOFS access policy (§8, §16, §58) | Implemented | 11 ordered rules, 20 tests |
| System-call ABI (§45) | Implemented | 25 syscalls specified; error model with plain-language text |
| Boot protocol (§6) | Implemented | Versioned, validated, 13 tests |
| Physical memory (§4) | Implemented | Bitmap frame allocator, 16 tests |
| Application permissions (§58) | Implemented | 12 permissions, grant model, 15 tests |
| Package trust (§59) | Implemented | Signature, source and version rules, 13 tests |
| Size budget measurement (§2, §48) | Implemented | `xtask size` measures real artefacts |

### Boot and kernel

| Component | Status | Notes |
|---|---|---|
| UEFI bootloader (§6) | Partial | Loads, maps and jumps. **Never executed.** |
| ELF loader | Implemented | Validates before loading; rejects W+X segments |
| Page-table construction | Implemented | Kernel mapped per-segment; W^X enforced from first instruction |
| Kernel entry and validation | Implemented | Rejects a bad boot info with a readable message |
| GDT and TSS (§4) | Implemented | Separate IST stacks for double fault, page fault, NMI |
| IDT and exception handlers (§4) | Implemented | Six handlers, each stopping with a plain-language message |
| Kernel heap | Implemented | 1 MiB, placed in the physical-memory window |
| Serial diagnostics | Implemented | Deadlock-safe; panic path bypasses the lock deliberately |
| Early framebuffer | Partial | Clearing and rectangles work. **No glyph rendering**, so the stop screen is a colour and a bar; the message goes to serial |
| Syscall dispatch (§45) | Partial | Decoding and rights checks written; every call returns `NotImplemented` |
| Scheduler and threads (§4) | Not started | Nothing to schedule yet |
| Address-space manager (§4) | Not started | Kernel runs on the bootloader's tables |
| Interrupt controller, timers | Not started | Needs ACPI parsing |
| Storage stack / LKOFS on disk | Not started | `lkofs-core` is the policy half; the I/O half does not exist |
| Userland | Not started | |

### Everything else

These are specified in the design and have no implementation. Listing them is
the point: the repository should never imply more exists than does.

| Area | Requirements | Status |
|---|---|---|
| Hardware abstraction layer | §5 | Not started |
| Device drivers | §5 | Not started |
| Graphics, compositor, glass UI | §9–§13, §60 | Not started |
| Desktop, dock, top bar, Control Center | §13, §41 | Not started |
| Window manager, virtual desktops | §13, §54 | Not started |
| Notifications | §40 | Not started |
| Loko AI | §14–§16, §43, §56, §57 | Not started |
| Linder | §17, §18 | Not started |
| Lowser | §19, §20 | Not started |
| Loko Store, `.loko` packaging | §21–§23 | Trust model implemented; format and manager not started |
| Windows compatibility | §25 | Not started |
| macOS compatibility | §26 | Not started |
| Compatibility Center | §27 | Not started |
| Loko Settings | §28 | Not started |
| Update channels and Loko Update | §29, §30 | Not started |
| Installer | §33–§36 | Not started |
| OOBE | §37, §39 | Not started |
| Accounts | §38 | Not started |
| Loko Search | §42 | Not started |
| Loko Terminal | §44 | Not started |
| Loko DevKit | §45 | ABI implemented; tooling not started |
| System Monitor | §46 | Not started |
| File indexing | §47 | Not started |
| Recovery environment | §49, §50 | Not started |
| Accessibility | §51 | Architecture noted; not started |
| Networking | §52 | Not started |
| Power management | §53 | Not started |
| Widgets | §55 | Not started |

---

## Known limitations in what *is* implemented

These are real constraints in shipped code, not future work in disguise.

1. **The frame allocator's bitmap is a 512 KiB static**, sized for 16 GiB of
   RAM. A machine with more boots, and the kernel logs how much memory it is
   leaving unmanaged rather than silently ignoring it. The fix is to carve the
   bitmap out of the memory map itself; it is not done because it adds a
   bootstrapping step to the least debuggable part of boot, and the current
   behaviour is correct, just wasteful.
   *Where:* `kernel/loko-kernel/src/main.rs`, `MAX_MANAGED_MEMORY`.

2. **The early framebuffer cannot draw text.** Requirement 64 wants the failure
   message on the screen; today it goes to serial and the screen shows the stop
   colour. A glyph rasteriser and a font are needed. Nothing fabricates a
   rendered message in the meantime.
   *Where:* `kernel/loko-kernel/src/framebuffer.rs`.

3. **Bootloader-reclaimable memory is never reclaimed.** The kernel reserves it
   at startup and has no release path, because the page tables the kernel is
   running on live in it. Reclaiming requires first rebuilding the tables in
   kernel-owned frames.
   *Where:* `kernel/loko-kernel/src/main.rs`, `setup_memory`.

4. **The low 4 GiB identity map is never torn down.** It exists so the `cr3`
   switch survives its own next instruction. The kernel should drop it once it
   is running from the higher half, and does not, because dropping it needs the
   address-space manager.
   *Where:* `boot/loko-boot/src/main.rs`, `IDENTITY_MAPPED_BYTES`.

5. **`xtask image` produces a directory, not a bootable disk image.** Making a
   `.img` needs a FAT32 formatter. Copying the tree onto an already-formatted
   ESP works.

6. **Secure Boot is reported, not performed.** The boot protocol carries a
   `SECURE_BOOT` flag and the kernel logs whether it is set, but the bootloader
   never sets it, because signature verification is not implemented. The kernel
   currently warns on every boot that the image was not verified. That warning
   is accurate.

7. **The host test suite does not run on Windows without a workaround.** The
   default `x86_64-pc-windows-msvc` target needs Visual Studio. `xtask` detects
   this and switches to `x86_64-pc-windows-gnu`, which links with `rust-lld`.
   This is a property of the development machine, not of LokoOS.

---

## What to build next, and why in this order

1. **Boot it.** Everything below is guesswork until the boot chain has been
   observed working once. This is the single highest-value next step and it
   needs only QEMU and OVMF.
2. **Timer and interrupt controller.** Nothing can be scheduled without a tick,
   and ACPI parsing is the prerequisite for both.
3. **Address-space manager.** Unlocks reclaiming boot memory, tearing down the
   identity map, and the beginnings of process isolation.
4. **Scheduler and the first userland thread.** Turns the syscall dispatch table
   from written code into running code.
5. **Storage stack.** `lkofs-core` already decides who may do what; it needs a
   device to decide it about.

Only then does the graphics stack become the right thing to work on. Building
the desktop before the kernel can schedule would mean building it on a
foundation whose shape is not yet known.
