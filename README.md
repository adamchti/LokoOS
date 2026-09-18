# LokoOS

**An Operating System Powered by AI.**

LokoOS is a modern desktop operating system built from the kernel up: small,
fast, private, and designed around an AI assistant that is part of the system
rather than an app bolted onto it.

This repository is the real thing — a bare-metal x86-64 kernel, a UEFI
bootloader, and the security and filesystem models the rest of the system will
be built on. It is early. **[documentation/STATUS.md](documentation/STATUS.md)
is the honest ledger of what exists and what does not**, and it is the first
thing to read.

---

## Where the project actually is

| | |
|---|---|
| Host test suite | 142 tests, all passing |
| Kernel | Builds to a valid higher-half ELF64, W^X enforced |
| Bootloader | Builds to a valid PE32+ EFI application |
| Has it booted? | **Not yet.** See [STATUS.md](documentation/STATUS.md) |

The boot path is written and compiles. It has never been run, because the
machine it was developed on has no emulator. That gap is documented rather than
papered over, and closing it is the next task.

---

## Getting started

### Requirements

- Rust nightly (pinned in `rust-toolchain.toml`; rustup installs it
  automatically)
- `rust-src` — needed to build `core` for the bare-metal targets
- A linker for your host. On Linux and macOS you already have one. On Windows,
  either the Visual Studio Build Tools with the C++ workload, or nothing at all
  — the build driver falls back to a self-contained GNU toolchain.
- QEMU and OVMF, to actually boot it

### Build and test

```bash
cargo xtask test      # host test suite
cargo xtask build     # kernel and bootloader
cargo xtask image     # lay out an EFI system partition in build/esp
cargo xtask size      # measure against the 2 GB base-OS budget
cargo xtask check     # what CI runs: fmt, clippy, tests, build
```

On Windows without Visual Studio, use the wrapper, which picks a toolchain that
can link:

```powershell
./tools/build.ps1 test
```

### Boot it

```bash
cargo xtask image
qemu-system-x86_64 -bios OVMF.fd -drive format=raw,file=fat:rw:build/esp -serial stdio
```

The kernel narrates every boot stage to the serial port, so `-serial stdio`
shows exactly how far it gets. If you are the first person to see it come up,
please update [STATUS.md](documentation/STATUS.md).

---

## What is here

```
LokoOS/
├── boot/
│   ├── loko-boot/            UEFI bootloader: ELF loading, page tables, handoff
│   └── loko-boot-protocol/   The one structure that crosses that boundary
├── kernel/
│   ├── loko-kernel/          The kernel: GDT, IDT, memory, heap, syscall dispatch
│   └── loko-memory/          Bitmap frame allocator, host-testable
├── filesystem/
│   └── lkofs-core/           The LKO path model, root taxonomy and access policy
├── security/
│   └── loko-security/        App permissions and package trust
├── sdk/
│   └── loko-abi/             The stable system-call ABI
├── tools/
│   ├── xtask/                The build driver
│   ├── build.ps1             Windows convenience wrapper
│   └── elfinfo.ps1           Dependency-free ELF inspector
└── documentation/
    ├── STATUS.md             What exists and what does not
    └── architecture/         How it is put together, and why
```

Directories the design calls for but which have no code yet — `drivers/`,
`graphics/`, `compositor/`, `desktop/`, `apps/`, `compatibility/`, `installer/`,
`recovery/` — are deliberately absent rather than present and empty. An empty
directory implies work in progress; no directory says plainly that it has not
started.

---

## Design principles

These are the ones that have already shaped the code, not aspirations.

**Authority is a handle, never a path.** A LokoOS process cannot name a resource
it was not given. There is no ambient authority to confuse-deputy around, which
is what makes revoking a permission mean something: revocation closes handles,
and there is no cached path string to fall back on.
→ [`sdk/loko-abi/src/handle.rs`](sdk/loko-abi/src/handle.rs)

**One decision, one function.** "May this subject do this to this path?" is
answered by exactly one pure function. So is "may this package be installed?"
and "may this app use the camera?". A policy spread across an installer, a shell
and a service is a policy with three subtly different answers.
→ [`filesystem/lkofs-core/src/policy.rs`](filesystem/lkofs-core/src/policy.rs)

**Reject rather than normalise.** LKOFS refuses `..` instead of resolving it.
Directory-traversal bugs live in the gap between two components that disagree
about the order of normalisation steps. If there is one spelling of every
location, there is nothing to disagree about.
→ [`filesystem/lkofs-core/src/path.rs`](filesystem/lkofs-core/src/path.rs)

**Errors are written for people.** Every error in LokoOS carries a plain-language
explanation and the actions a UI should offer, as part of the type. Not as
something a UI layer is trusted to remember to add. There is a test that fails
if an explanation contains a hex code.
→ [`sdk/loko-abi/src/error.rs`](sdk/loko-abi/src/error.rs)

**LokoOS is inspectable but not modifiable.** The system's own files are readable
by whoever owns the machine and writable only inside a verified update
transaction — not by the shell, not by a service, not by the user, and not by an
idle updater sitting outside a transaction.
→ [`filesystem/lkofs-core/src/policy.rs`](filesystem/lkofs-core/src/policy.rs)

**The AI never destroys anything on its own initiative.** Loko AI reads freely
within what it has been granted and creates new files freely. Changing or
deleting something that already exists always goes to the user, with the files
named.
→ requirement 16, enforced in `policy.rs` rule 10

**Unfinished means unfinished.** Every subsystem that does not exist returns a
distinct `NotImplemented` error rather than a plausible-looking success, so that
anything built on top fails immediately instead of appearing to work.

---

## Contributing

Read [documentation/architecture/overview.md](documentation/architecture/overview.md)
first, then the ADRs in `documentation/architecture/adr/`. They record why things
are the way they are, which is usually the part that is expensive to rediscover.

Before opening a pull request:

```bash
cargo xtask check
```

Two rules matter more than style:

1. **Do not claim something works that you have not seen work.** If you write a
   subsystem you could not test, say so in its module documentation and in
   `STATUS.md`. That is not a failure; presenting it as finished would be.
2. **Every security decision needs a test that fails without it.** A rule nobody
   has exercised is a rule nobody has checked.

---

## Licence

Apache-2.0. See [LICENSE](LICENSE).

LokoOS contains no code, assets, icons or branding from any other operating
system. The visual language it will have is its own.
