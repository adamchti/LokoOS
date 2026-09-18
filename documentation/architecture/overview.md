# LokoOS architecture

This describes how LokoOS is put together and, more usefully, why. For what
exists today versus what is still design, see
[STATUS.md](../STATUS.md).

---

## The shape of the system

```
                    ┌─────────────────────────────────────────┐
                    │  Desktop shell · Linder · Lowser · Store │   userland
                    │  Loko AI · Settings · Terminal · Monitor │
                    └───────────────────┬─────────────────────┘
                                        │  handles + messages
                    ┌───────────────────┴─────────────────────┐
                    │  System services                        │   userland
                    │  storage · network · index · update     │
                    └───────────────────┬─────────────────────┘
                                        │  system calls
    ┌───────────────────────────────────┴─────────────────────┐
    │  Kernel                                                 │   ring 0
    │  scheduling · memory · handles · IPC · policy            │
    └───────────────────────────────────┬─────────────────────┘
                                        │
    ┌───────────────────────────────────┴─────────────────────┐
    │  Hardware abstraction layer  ·  drivers                  │
    └─────────────────────────────────────────────────────────┘
```

The kernel is small and does not grow. Search, indexing, the package manager,
the AI runtime and the compositor are userland services reached over message
channels, not kernel subsystems. This is not minimalism for its own sake: every
line in the kernel is a line in the trusted computing base that can never be
removed, sandboxed, or restarted after a crash.

---

## The three decisions that shape everything else

### 1. Authority is a handle

A LokoOS system call does not take a path, a name, or a process id. It takes a
`Handle` — an opaque, per-process index into a kernel table that names an object
*and* carries the rights the holder has over it.

A process receives its initial handles at startup, from its `.loko` manifest. It
can derive narrower handles from them, and it can pass handles it holds to other
processes over a channel. It can never widen a handle, and it can never name an
object it was not given.

The consequences are worth spelling out:

- **There is no ambient authority.** A confused-deputy attack needs a way to make
  a privileged process act on a name the attacker supplies. If names are not
  authority, there is nothing to supply.
- **Revocation is real.** When the user turns off "Files" for an app, the kernel
  closes that app's filesystem handles. There is no cached path the app can fall
  back on, because the path was never what let it in.
- **Delegation is explicit and bounded.** A service that hands out a handle
  without `TRANSFER` has handed out authority that cannot be re-delegated.

→ [`sdk/loko-abi/src/handle.rs`](../../sdk/loko-abi/src/handle.rs)

### 2. Policy is a pure function

There are three policy questions in LokoOS, and each is answered by exactly one
function with no I/O, no clock and no globals:

| Question | Function |
|---|---|
| May this subject do this to this path? | `lkofs_core::evaluate` |
| May this application use this capability now? | `loko_security::Record::check` |
| May this package be installed? | `loko_security::evaluate_install` |

Purity is what makes them exhaustively testable, and being testable is what
makes it reasonable to believe they are right. It also means the kernel, the
installer, the recovery environment and Linder's preview pane all reach the same
verdict for the same inputs — which is the only way a permission model stays
coherent as the system grows.

Each is written as an ordered list of rules where the first match wins and the
most restrictive rules come first. Reading `policy.rs` top to bottom *is* reading
the security model.

→ [`filesystem/lkofs-core/src/policy.rs`](../../filesystem/lkofs-core/src/policy.rs)

### 3. The system is inspectable but not modifiable

`LKO/System`, `LKO/Drivers`, `LKO/Boot` and `LKO/Recovery` are readable by
anything that can reach them — the person owns the machine and should be able to
look inside it — and writable by nothing. Not the shell, not a system service,
not a driver, not the user, and not Loko Update sitting idle outside a
transaction.

They change exactly one way: a verified, staged update transaction replaces them
wholesale. There is no in-place edit path to exploit, because there is no
in-place edit path at all.

---

## Layout

### `LKO/` — the filesystem the user sees

LokoOS does not present a Unix hierarchy. The top level is a closed set of
nineteen named roots, each with a declared purpose, a protection class and a
storage category. Because the set is closed and known at compile time, "is this
a protected location?" is answered without consulting any on-disk metadata,
which means the answer cannot be changed by anything an attacker can write to
disk.

The roots and their rules are in
[`filesystem/lkofs-core/src/root.rs`](../../filesystem/lkofs-core/src/root.rs);
the details are in [lkofs.md](lkofs.md).

### The repository

Source layout is in the [README](../../README.md). Two conventions:

- **Crates that must be right are separated from crates that must talk to
  hardware.** `loko-memory` holds the frame allocator with no hardware
  dependency, so every branch is driven from a host unit test. `loko-kernel`
  holds the part that touches a real CPU. The split exists so that the
  allocator's correctness does not depend on having a machine to run it on.
- **Directories for work that has not started do not exist.** An empty
  `graphics/` implies work in progress; no `graphics/` says plainly that it has
  not begun.

---

## Boot

Full detail in [boot.md](boot.md). In summary:

```
firmware → loko-boot.efi → [ExitBootServices] → loko-kernel
              │                                     │
              │ reads the kernel off the ESP        │ validates BootInfo
              │ builds page tables (W^X per segment)│ GDT + TSS + IDT
              │ maps physical memory at the offset  │ frame allocator
              │ identity-maps low 4 GiB for the     │ heap
              │   cr3 switch itself                 │ (stops here today)
              └── jumps with BootInfo in rdi ───────┘
```

Exactly one structure crosses that boundary: `BootInfo`. It is `#[repr(C)]`,
versioned, self-describing, and validated before any field is trusted — because
boot is the one place where a mismatch cannot produce a diagnostic, since the
diagnostic machinery is what is being set up.

---

## Memory

**Physical.** A bitmap allocator: one bit per 4 KiB frame, 32 KiB per gigabyte.
A free-list would be smaller and would offer neither O(1) free with exact
double-free detection nor a structure that can be checked for consistency at any
moment. The allocator starts with everything marked *allocated* and memory
becomes usable only by being explicitly released from a region the firmware
called usable — so a bug in the release path costs memory, which is visible,
rather than handing out firmware-reserved frames, which is not.

**Virtual.** The kernel is linked into the top 2 GiB of the canonical higher
half. Every user address is then "not a kernel address" by a single sign test,
and intra-kernel references fit in a 32-bit displacement, which matters on the
system-call path.

All of physical memory is mapped at a known offset. That is how the kernel
reaches an arbitrary frame before it has an address-space manager — without it,
allocating a page table requires a page table.

---

## Errors

Every LokoOS error carries three things as part of its type: a stable numeric
code, a developer-facing name, and a plain-language explanation with the actions
a UI should offer.

The third part is the unusual one, and it is deliberate. Requirement 64 says the
system must never show `ERR_0x00482` when it could say what went wrong. The only
way to guarantee that across a whole operating system is to make the human
sentence part of the error rather than something each UI layer is trusted to
remember to add. There is a test that fails if any explanation contains a hex
code.

Errors also declare whether they are safely retryable and whether they belong in
the security log. `IntegrityFailure` is deliberately **not** retryable:
automatically retrying a signature failure is how a downgrade attack gets a
second chance.

→ [`sdk/loko-abi/src/error.rs`](../../sdk/loko-abi/src/error.rs)

---

## Records of specific decisions

| ADR | Decision |
|---|---|
| [0001](adr/0001-languages.md) | Which language each layer is written in |
| [0002](adr/0002-targets.md) | Using stock Rust targets instead of custom target specs |
| [0003](adr/0003-dependency-policy.md) | What may be depended on inside the TCB |
| [0004](adr/0004-handle-based-abi.md) | Handles rather than paths in the ABI |
| [0005](adr/0005-abi-stability.md) | How the ABI is allowed to change |
| [0006](adr/0006-higher-half-kernel.md) | Where the kernel lives in the address space |
