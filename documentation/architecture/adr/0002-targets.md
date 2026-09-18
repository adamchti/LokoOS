# ADR 0002 — Stock Rust targets instead of custom target specifications

**Status:** Accepted · **Date:** 2026-09-18

## Context

Hobby kernels traditionally ship a custom target JSON: `x86_64-lokoos.json`,
with `os: none`, soft float, no red zone, and a hand-copied `data-layout`
string.

That string is the problem. It must match exactly what the bundled LLVM expects,
and LLVM changes it between versions. When it drifts, the failure is a compiler
error at best and a miscompilation at worst, and the cause is a line nobody
remembers copying.

## Decision

Use the tier-2 targets Rust already ships:

- **`x86_64-unknown-none`** for the kernel. Soft float, no red zone, static
  relocation, freestanding — exactly what a kernel needs.
- **`x86_64-unknown-uefi`** for the bootloader. Produces a PE32+ image with
  subsystem 10, which is what firmware loads.

No `targets/*.json` in this repository.

Build `core`, `compiler_builtins` and `alloc` from source with `-Zbuild-std`,
rather than relying on the precompiled `rust-std` rustup ships for these
targets. rustup does ship one, but depending on it means a contributor's first
build fails with a rustup error instead of just working, and `rust-src` is
required for the toolchain anyway.

## Consequences

Kernel layout is controlled through `linker.ld` and the linker arguments
`build.rs` emits, which is where it belongs — the address the kernel is linked
at is a link-time fact, not a codegen one.

Building `core` from source costs roughly a minute on a cold build. Acceptable,
and it removed an entire class of "works on my LLVM" failure.

If a future target genuinely needs a feature no stock target provides — a
different code model, a nonstandard ABI — this decision gets revisited for that
target specifically. It will not be revisited to save a minute of build time.
