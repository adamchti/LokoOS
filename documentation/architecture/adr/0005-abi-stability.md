# ADR 0005 — How the system-call ABI is allowed to change

**Status:** Accepted · **Date:** 2026-09-18

## Context

`loko-abi` is the contract with every binary ever compiled for LokoOS. Changing
a discriminant, reordering a struct field or reusing a syscall number breaks
programs whose source may no longer exist. Operating systems that got this wrong
are still paying for it.

The project is at version 0.1.0 and the ABI will change. The question is not
whether, but under what discipline — because the habits formed now are the ones
that will be in place when it stops being allowed to change.

## Decision

**Permanently fixed, from the first release:**

- Syscall numbers. A removed syscall keeps its number reserved and returns
  `NoSuchSyscall`.
- `Error` discriminants. New variants are appended.
- `Permission` and `Root` discriminants — they appear in installed application
  records and in on-disk structures.
- The field order and layout of any `#[repr(C)]` type that crosses the boundary.

**May change, with a minor version bump:**

- Adding a syscall.
- Adding an `Error`, `Permission` or `HandleType` variant. Every such enum is
  `#[non_exhaustive]`, so a match on it must already have a wildcard arm.
- Adding a field to the *end* of a `#[repr(C)]` struct that carries its own
  `size`, which `BootInfo` does.

**Requires a major version bump:** anything else.

`ABI_VERSION_MAJOR` and `ABI_VERSION_MINOR` are recorded in a binary's `.loko`
manifest. The loader refuses a binary whose major version this kernel does not
implement, with a message the user can act on rather than a fault. A binary
built against a lower minor version always runs on a higher one.

**Enforcement.** Each stability property has a test that fails if it is
violated. `numbers_are_unique`, `codes_are_unique_and_positive`,
`discriminants_are_contiguous_from_one` and
`the_structure_layout_is_what_the_bootloader_will_write` exist to make an
accidental break a red CI run rather than a field report.

## Consequences

Removing a badly designed syscall costs a number forever. Accepted: an unusable
number is cheaper than a broken binary.

The pre-1.0 period is where the ABI should be got right, which means resisting
"we can fix that later". After 1.0, later means never.
