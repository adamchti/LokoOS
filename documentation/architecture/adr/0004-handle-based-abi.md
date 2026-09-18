# ADR 0004 — Handles rather than paths in the system-call ABI

**Status:** Accepted · **Date:** 2026-09-18

## Context

The obvious ABI for a filesystem is `open(path, flags)`. It is what every
developer expects, it needs no setup, and it is the reason a large family of
security bugs exists.

The problem is *ambient authority*: any process can name any file, and whether
it may touch it is decided by a check somewhere else, against an identity the
process carries implicitly. That gap between naming and checking is where
confused-deputy bugs, TOCTOU races and sandbox escapes live. It is also why
revoking a permission on a path-based system is so weak — the process still
knows the name, and only a check stands between it and the file.

## Decision

No LokoOS system call takes a path.

A process receives handles at startup according to its `.loko` manifest, and
derives narrower ones with `HandleDerive`. `FsOpenAt` opens relative to a
directory handle. A process with no directory handle cannot reach the filesystem
at all — not because a check refuses it, but because it has no way to express
the request.

Rights are carried by the handle and checked on every call, not once at open
time. `HandleRights::derive` can only narrow, and requires the `DERIVE` right to
do even that.

## Consequences

**What this buys.**

- Revocation is real. Closing a handle removes the authority; there is no cached
  name to fall back on.
- A sandbox is the set of handles a process holds, which is enumerable and
  inspectable, rather than a policy that must anticipate every path.
- TOCTOU on the path is gone: there is no window between resolving a name and
  using it, because the handle *is* the resolved object.

**What it costs.**

- Every application must be written for it. A POSIX program expecting `open()`
  does not port unchanged.
- The compatibility runtimes have real work to do: translating a Windows or
  macOS application's path-based calls into handle operations, using only the
  handles the runtime was granted. This is the honest cost of the decision, and
  it is one reason those runtimes are separate, optional and untrusted.
- Passing a file between processes means passing a handle over a channel, which
  is more machinery than passing a string.

## Alternatives rejected

**Paths plus a sandbox policy** (the AppArmor/seccomp shape). Keeps ambient
authority and adds a second system that must agree with the first about what a
path means. Two systems that must agree about path normalisation is exactly the
bug class LKOFS is designed to avoid.

**Paths resolved against a per-process root** (chroot). Better, but the root is
still ambient within itself, and it cannot express "this one file" without
building a directory to hold it.
