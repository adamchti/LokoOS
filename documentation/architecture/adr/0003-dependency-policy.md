# ADR 0003 — What may be depended on inside the trusted computing base

**Status:** Accepted · **Date:** 2026-09-18

## Context

Every crate the kernel depends on is part of the trusted computing base. It runs
in ring 0, it can do anything the kernel can do, and it cannot be sandboxed,
audited at runtime or restarted after a fault. A dependency added for
convenience is a permanent expansion of the attack surface.

The opposite failure is real too: reimplementing `GlobalDescriptorTable` from
the Intel manual produces code that is worse than the widely used crate, and
nobody reviews it.

## Decision

A dependency may enter the kernel only if it meets all of:

1. **It cannot reasonably be written correctly in-house.** Hardware structure
   layouts and instruction wrappers qualify. A data structure does not.
2. **It is `no_std` and pulls in nothing that is not.**
3. **It is small enough to read.** Not skim — read.
4. **It is widely used in the same role**, so that bugs are found by more people
   than us.
5. **It is pinned**, and an update is a reviewed commit.

The current list, in full:

| Crate | Why it is in the TCB |
|---|---|
| `x86_64` | Descriptor tables, page-table entries and privileged instructions. Hand-rolling these is how you get a bug that only appears on one CPU stepping |
| `bitflags` | Flag sets. Small, ubiquitous, no unsafe in the generated code |
| `spin` | Spinlocks and lazy statics. The kernel cannot block |
| `uart_16550` | The serial port. About 200 lines |
| `linked_list_allocator` | The kernel heap. Small and well-reviewed |

The bootloader additionally depends on `uefi`, which is not in the kernel's TCB:
the bootloader exits before the kernel runs, so a bug there cannot outlive
`ExitBootServices` except through the `BootInfo` it produces — which the kernel
validates before trusting.

Policy and protocol crates — `loko-abi`, `lkofs-core`, `loko-security` — depend
on nothing outside the workspace except `bitflags`.

## Consequences

Some things get written here that a crate could have provided. That is the
intended trade.

When a new dependency is proposed, the answer is this list plus one row, or a
reason the five rules do not apply. "It is popular" is not one of them.
