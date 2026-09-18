# Working in this repository

Conventions that are not obvious from the code. Read
[documentation/STATUS.md](documentation/STATUS.md) first — it says what actually
exists.

## Building

`cargo xtask <command>` is the entry point for everything. Three targets are in
play and each needs different flags; the driver knows which crate belongs to
which.

```bash
cargo xtask test     # host tests (excludes the freestanding crates automatically)
cargo xtask build    # kernel + bootloader
cargo xtask image    # lay out an ESP in build/esp
cargo xtask iso      # build a bootable hybrid ISO in build/
cargo xtask boot-test # build the ISO and boot it under QEMU
cargo xtask size     # measure against the 2 GB budget
cargo xtask check    # fmt --check, clippy, test, build. What CI runs
```

On Windows without Visual Studio, use `./tools/build.ps1 <command>`. The default
`x86_64-pc-windows-msvc` target needs `link.exe`; the driver falls back to
`x86_64-pc-windows-gnu`, which links with `rust-lld` and needs no external
linker. Set `LOKO_TOOLCHAIN` to override.

`loko-kernel` and `loko-boot` **cannot be built for the host** — they have no
`main`, and define their own panic handler and allocator. Any host command must
exclude them. `xtask` does this; raw `cargo test --workspace` does not.

## Layout rules

- **Directories for work that has not started do not exist.** No empty
  `graphics/`. An empty directory implies work in progress; no directory says
  plainly that it has not begun.
- **Logic worth testing does not live in a crate that cannot be tested on a
  host.** Where hardware and logic are tangled, the logic moves out — that is why
  `loko-memory` is separate from `loko-kernel`.
- **Crates that can `forbid(unsafe_code)` do.** Today: `loko-abi`, `lkofs-core`,
  `loko-security`.

## The two rules that matter

**1. Do not claim something works that you have not seen work.**

This repository carried a boot path that compiled and had never run, and said so
in three places, until CI booted it. That is the shape to aim for: an honest
"unverified" is fine, and it is what lets you tell later whether something
actually started working. If you write a subsystem you could not test, say so in
module documentation and in `STATUS.md`. That is not a failure. Presenting it as
finished would be.

Unimplemented paths return `Error::NotImplemented`, never a plausible-looking
success. Something built on top must fail immediately and obviously rather than
appear to work.

**2. Every security decision needs a test that fails without it.**

The policy functions are pure specifically so this is possible. A rule nobody has
exercised is a rule nobody has checked. Tests are named after the property they
protect — `a_sandboxed_app_cannot_escape_via_a_similar_prefix`,
`integrity_failures_are_never_retried_automatically` — so a failure says what
broke, not just where.

## Style

- `unsafe` carries a `// SAFETY:` comment stating what the caller must
  guarantee. `unsafe_op_in_unsafe_fn` is denied, so an `unsafe fn` gets no free
  pass on its own body.
- Comments explain *why*, not *what*. The code says what.
- User-facing strings are plain language with no error codes. There are tests
  that fail if an explanation contains `0x`.
- Policy is written as ordered rules, most restrictive first, first match wins.
  Reading the function top to bottom should be reading the model.

## Where the decisions are written down

`documentation/architecture/adr/` — language choice, target selection,
dependency policy, the handle-based ABI, ABI stability, kernel address space.
When a decision is expensive to rediscover, it goes there.
