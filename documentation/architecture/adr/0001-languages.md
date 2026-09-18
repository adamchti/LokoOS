# ADR 0001 — Which language each layer is written in

**Status:** Accepted · **Date:** 2026-09-18

## Context

The LokoOS design says to use appropriate low-level technologies and explicitly
warns against forcing everything into one language. That is right, but it is not
a decision — it is permission to make one. Without a rule, the choice gets made
per-file by whoever writes it first.

## Decision

| Layer | Language | Why |
|---|---|---|
| Kernel, bootloader, drivers, services | Rust | Memory safety in the one place where a bug is unrecoverable and unsandboxable |
| Policy and protocol crates | Rust, `no_std`, `forbid(unsafe_code)` | Must be right, must be testable on a host, must have no way to be wrong |
| Inline assembly | Rust `asm!` | For the handful of operations with no Rust expression: `cr3` loads, `rdmsr`/`wrmsr`, `hlt` |
| Standalone `.s` files | Not used | A separate assembler is a second toolchain for a few dozen instructions |
| C | Only to link existing C libraries | None yet. Each one is a separate decision |
| Build automation | Rust (`xtask`) | The build driver runs on every machine; a Rust binary needs only the toolchain already required |
| Shell | PowerShell / POSIX sh, thin wrappers only | Bootstrap and convenience. No logic that a wrapper on one platform could get different from the other |

Rules that follow:

1. `unsafe` requires a `// SAFETY:` comment stating what the caller must
   guarantee. `unsafe_op_in_unsafe_fn` is denied workspace-wide, so an `unsafe
   fn` does not get a free pass on its own body.
2. Crates that can be `forbid(unsafe_code)` are. Today: `loko-abi`,
   `lkofs-core`, `loko-security`.
3. Logic worth testing does not live in a crate that cannot be tested on a host.
   Where hardware and logic are tangled, the logic moves to its own crate — this
   is why `loko-memory` is separate from `loko-kernel`.

## Consequences

The kernel needs nightly Rust, for `abi_x86_interrupt` and
`alloc_error_handler`. Both are long-standing features that every Rust kernel
uses; the pin in `rust-toolchain.toml` makes a toolchain change a reviewed
commit.

Choosing Rust for drivers will make some hardware harder to support, since
vendor reference code is C. When that becomes concrete, the choice is a binding
per driver, not a rewrite of this decision.
