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
| Host test suite | **144 tests, all passing** |
| Kernel | **Builds** to a valid higher-half ELF64 for `x86_64-unknown-none` |
| Bootloader | **Builds** to a valid PE32+ EFI application for `x86_64-unknown-uefi` |
| Bootable ISO | **Yes.** Hybrid image, built and verified on every push |
| Booted in an emulator | **Yes.** QEMU with OVMF, on every push |
| Booted on real hardware | **No.** Nobody has tried |

### LokoOS boots

As of 2026-09-18 LokoOS starts, initialises, and halts as designed. This is the
serial log from the CI run that first proved it, trimmed of firmware noise:

```
[ INFO]: LokoOS bootloader 0.1.0
[ INFO]: stage 1: firmware initialised
[ INFO]: stage 2: reading \EFI\LOKO\loko-kernel
[ INFO]: kernel entry 0xffffffff80003890, 612 KiB across 0xffffffff80000000..0xffffffff80099000
[ INFO]: stage 3: reserving memory
[ INFO]: stage 4: querying hardware
[ INFO]: framebuffer 1280x800 at 0x80000000
[ INFO]: 512 MiB of RAM, framebuffer present, ACPI at 0x1fb7e014
[ INFO]: stage 5: building page tables
[ INFO]: page tables used 12 of 256 frames
[ INFO]: stage 6: exiting boot services
[INFO ] boot       LokoOS kernel 0.1.0
[INFO ] boot       stage 5: kernel entered
[INFO ] boot       protocol 0.1, 129 memory regions, physical window at 0xffff800000000000
[WARN ] security   Secure Boot is off; this image was not verified
[INFO ] boot       stage 6: descriptor tables
[INFO ] graphics   framebuffer 1280x800 at 0x80000000
[INFO ] boot       stage 7: physical memory
[INFO ] boot       stage 8: kernel heap
[INFO ] heap       1024 KiB at 0xffff800000100000
[INFO ] memory     16384 MiB addressable, 461 MiB free, 0 KiB heap in use
[INFO ] boot       stage 9: not implemented; no scheduler or storage stack yet
[INFO ] boot       kernel initialised successfully and is halting
```

Firmware loads the hybrid ISO, the bootloader reads the kernel off the EFI
system partition, loads it, builds page tables and switches to them, and the
kernel comes up on the other side, validates what it was handed, brings up its
descriptor tables, takes ownership of physical memory and starts a heap.

`cargo xtask boot-test` runs this, and CI runs it on every push. It decides
success from the serial log rather than from an exit status, because the kernel
halts rather than exiting, and it reports which of the five boot stages was
reached so a regression localises to a subsystem instead of being a timeout.

### What is still unproven

- **Real hardware.** Everything above is QEMU with OVMF. Firmware in the wild is
  more varied and less forgiving than OVMF, and nobody has put this on a USB
  stick and tried it. If you do, the serial log is the thing to capture.
- **Any machine that is not 512 MiB, 1280x800 and one CPU.** That is what the
  CI runner emulates. Other memory sizes, other framebuffer formats, no
  framebuffer at all, and more than one core are all untested paths.
- **Everything after stage 8.** There is no scheduler, no address-space manager
  and no storage stack, so the kernel halts. It says so on the serial console.

### What it took to get here

Five CI runs, each of which localised the next real bug. Recorded because the
failures are more instructive than the success:

| Run | Outcome |
|---|---|
| 1 | ISO build failed: `10K0050F` is not a hexadecimal volume ID |
| 2 | ISO built and verified; the boot test tripped on a non-idempotent rebuild |
| 3 | Firmware loaded it; the bootloader died at stage 5 trying to map a 1 TiB PCI hole as if it were RAM |
| 4 | Reached stage 6, then triple-faulted: the identity map was marked no-execute, so the instruction after `mov cr3` could not be fetched |
| 5 | Booted |

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
| UEFI bootloader (§6) | Implemented | Loads, maps and jumps. Verified booting in QEMU on every push |
| ELF loader | Implemented | Validates before loading; rejects W+X segments. Verified on a real kernel image |
| Page-table construction | Implemented | Kernel mapped per-segment; W^X enforced from the first instruction. 12 frames on a 512 MiB machine |
| Kernel entry and validation | Implemented | Accepts a real boot info; rejects a bad one with a readable message |
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

5. **An ISO can only be built where `xorriso`, `mtools` and `dosfstools` exist.**
   That rules out Windows, so `cargo xtask iso` fails there with a message
   saying which tools are missing rather than producing a broken image. CI
   builds the ISO on every push and publishes it as an artifact.

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

~~Boot it.~~ Done, and every step below now has a working foundation to be
tested against rather than reasoned about.

1. **Timer and interrupt controller.** Nothing can be scheduled without a tick,
   and ACPI parsing is the prerequisite for both. The RSDP is already found and
   handed to the kernel; nothing parses it yet.
2. **Address-space manager.** Unlocks reclaiming boot memory, tearing down the
   low identity map, and the beginnings of process isolation. Both of those are
   in the limitations list above.
3. **Scheduler and the first userland thread.** Turns the syscall dispatch table
   from written code into running code.
4. **Storage stack.** `lkofs-core` already decides who may do what; it needs a
   device to decide it about.

Worth doing alongside, now that the boot test exists to catch regressions:

- **Try it on real hardware.** The one large unknown left in the boot path.
- **A glyph rasteriser for the early framebuffer**, so a stop screen says
  something rather than showing a colour.

Only then does the graphics stack become the right thing to work on. Building
the desktop before the kernel can schedule would mean building it on a
foundation whose shape is not yet known.
