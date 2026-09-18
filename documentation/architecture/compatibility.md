# Windows and macOS compatibility

**Status: architecture only. No code.** This records the decisions that have been
made so that the work, when it starts, starts from a position rather than from
scratch.

---

## What LokoOS will and will not claim

Requirement 24 is explicit: do not falsely claim that every Windows or macOS
application will work. That is a promise no one can keep, and a compatibility
story that over-promises is worse than one that is narrow and honest, because
users calibrate on the first application that fails.

So LokoOS states compatibility **per application**, from evidence, using the
statuses in requirement 26:

| Status | Meaning |
|---|---|
| `Native` | Built for LokoOS |
| `Compatible` | Tested, works |
| `Partial` | Tested, works with named limitations |
| `Experimental` | Untested or inconsistent |
| `Unsupported` | Known not to work |

An application with no evidence is `Experimental`, never `Compatible`. The Loko
Compatibility Center (requirement 27) shows the evidence, not just the verdict.

---

## Why the runtimes are separate, optional, and untrusted

**Separate.** The base OS targets under 2 GB (requirement 2). A Windows
compatibility runtime is hundreds of megabytes. Bundling it would blow the budget
for every user, including those who never run a Windows application. It is a
download, and `LKO/Runtime/Windows` does not exist until it is installed — so a
missing runtime is distinguishable from an empty one.

**Optional.** `storage_category_of` classifies `LKO/Runtime/Windows` and
`LKO/Runtime/Mac` as *Compatibility* storage rather than *Base OS*, so they are
excluded from the size budget. That exclusion is encoded in one function and
covered by a test.

**Untrusted.** This is the important one. A foreign application's code was
written for a different security model and cannot be assumed to respect LokoOS's.
`SubjectClass::CompatibilityRuntime` is therefore the most constrained role in
the policy:

- It cannot read `LKO/System`, `LKO/Drivers`, `LKO/Boot`, `LKO/Recovery` or
  `LKO/Services` **at all** — not even to enumerate them. A Windows application
  fingerprinting LokoOS internals has no legitimate use.
- Every write to user data returns `Confirm`, not `Allow`.

→ `filesystem/lkofs-core/src/policy.rs`, rules 2 and 11.

---

## The hard part: handles versus paths

[ADR 0004](adr/0004-handle-based-abi.md) removed paths from the LokoOS ABI. A
Windows or macOS application is built entirely around path-based calls.

This is the real cost of that decision, and it lands here. The runtime must
translate `CreateFileW("C:\Users\Adam\Documents\a.txt", ...)` into an operation
on handles it actually holds. Concretely:

1. The runtime is granted a set of directory handles, exactly as any application
   is — from the user, through the normal permission flow.
2. It maintains a mapping from the drive letters and paths the foreign
   application believes in to the handles it holds.
3. A path the mapping cannot satisfy fails with the foreign platform's
   equivalent of "access denied", which foreign applications already handle.

The consequence is that a Windows application's file access in LokoOS is bounded
by what the *user* granted the runtime, not by what the application asks for.
That is stricter than the application expects, and some will break. That is the
correct trade, and it needs to be visible in the Compatibility Center rather than
discovered.

---

## Technology selection: not yet made

Requirement 25 lists candidates — Wine, Proton-style layers, DXVK, Vulkan
translation, virtualisation. No choice has been made, and making one now would be
guessing, because the criteria depend on parts of LokoOS that do not exist.

The criteria, recorded so the decision is not made by accident:

| Criterion | Why it matters |
|---|---|
| Licence compatibility | LokoOS is Apache-2.0. A runtime's licence terms must permit distribution as an optional component |
| Graphics translation | Depends on what the LokoOS graphics stack exposes, which is not designed yet |
| Confinement | Whichever is chosen must be confinable to the `CompatibilityRuntime` role above. A runtime needing broad system access disqualifies itself |
| Maintenance burden | A fork of a large upstream project is a permanent cost |
| Size | It is a download, but it is still the user's disk |

**macOS is a separate architecture, not a variant of the Windows one.** It is
also subject to Apple's platform and licensing restrictions, which LokoOS will
respect. The realistic scope is a framework for supported applications and APIs,
not general macOS binary compatibility, and requirement 26 says as much.

---

## Third-party code is never vendored

`.gitignore` excludes `compatibility/*/vendor/`. Compatibility runtimes are
fetched, verified and installed — never committed into this repository. Their
licences, provenance and update cadence are their own, and mixing them into the
LokoOS tree would make all three unclear.
