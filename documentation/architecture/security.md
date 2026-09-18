# Security model

Three questions, three pure functions, three test suites. Everything else in
LokoOS's security story reduces to one of them.

| Question | Function | Tests |
|---|---|---|
| May this subject do this to this path? | `lkofs_core::evaluate` | 20 |
| May this app use this capability now? | `loko_security::Record::check` | 15 |
| May this package be installed? | `loko_security::evaluate_install` | 13 |

The first is documented in [lkofs.md](lkofs.md). This covers the other two, and
the principles behind all three.

---

## Principles

**Least privilege is structural, not administrative.** A process cannot name what
it was not given ([ADR 0004](adr/0004-handle-based-abi.md)). This is not a policy
that has to be configured correctly; it is the shape of the ABI.

**Every decision is testable.** No policy function touches the filesystem, a
clock or a global. A security rule nobody can exercise is a security rule nobody
has checked.

**Failing closed is not enough — it has to fail *legibly*.** Every denial carries
a plain-language explanation of what happened and how to change it. A refusal the
user cannot act on produces a user who disables the protection.

**Anything that could destroy data asks first.** Not a confirmation dialog bolted
on at the UI layer, but a distinct `Confirm` verdict that a caller must handle
separately from `Allow`.

---

## Application permissions

Twelve permissions (requirement 58 plus the AI permission from requirement 16):
`files` `camera` `microphone` `network` `notifications` `clipboard` `location`
`ai` `system-settings` `bluetooth` `screen-capture` `run-at-login`.

### Declared, then granted

An application can only ever *use* a permission it *declared* in its `.loko`
manifest. Declaring is public and inspectable in the Loko Store. Asking for
something never declared is refused outright — `DenyReason::NotDeclared` — and
the user is never interrupted about it, because a permission the app did not ask
for in public is not one to negotiate in private.

### Covert-capable permissions cannot be granted permanently by a prompt

The camera, the microphone, location, screen capture and the clipboard can all be
used without the user noticing. For these, the strongest grant a *prompt* can
produce is "while I'm using this app". `Record::decide` clamps anything stronger
rather than rejecting it, so a UI bug downgrades to the safe answer instead of
quietly handing out a permanent grant to the microphone.

A permanent grant is still possible — through `set_from_settings`, which is the
user going to Settings and choosing deliberately. That friction is the point: it
is the difference between a decision and a reflex.

The three that *capture* — camera, microphone, screen — also show a live
indicator whenever they are in use.

### Revocation and sessions

| Action | Effect |
|---|---|
| `revoke` | Sets the state to *refused*, immediately. Not back to "not asked" — the app does not get to re-prompt the moment it is denied |
| `end_session` | Clears one-time grants only. Refusals survive |
| System-wide switch off | Overrides every grant, including `Always` |

There is no "takes effect on next launch". That would be a window in which the
promise is not kept.

### Loko AI holds nothing by default

`LOKO_AI_PERMISSIONS` lists what Loko AI may ask for. Every one starts as
`NotAsked`. Being part of the system does not grant Loko AI anything; the user
turns each capability on.

It is deliberately not given `location` or `screen-capture`. Nothing in
requirement 14's list of what Loko AI does needs either, and a system assistant
that can see the screen is a different product with a different consent
conversation.

---

## Package trust

Two rules are absolute and not user-overridable:

**A package whose contents do not match its signature is never installed.** Not
with a warning, not with a checkbox, not in developer mode, not from any source.
A failed integrity check means the bytes are not what the publisher signed, and
no amount of user confidence changes that.

**A package cannot claim to be a LokoOS component unless it is signed by the
LokoOS root**, and even then only if it arrived through Loko Update. Otherwise
the most valuable label in the system is free.

### The rest of the ladder

| Signature | Proves integrity | Proves identity | Outcome |
|---|---|---|---|
| `LokoOfficial` | yes | yes | Install |
| `VerifiedPublisher` | yes | yes | Install from the Store; warn if sideloaded |
| `KnownPublisher` | yes | yes | Install from the Store; warn if sideloaded |
| `UnknownPublisher` | yes | **no** | Warn: unknown publisher |
| `Unsigned` | no | no | Refused unless Developer Mode, and still warns |
| `Invalid` | no | no | Refused, always |

The `UnknownPublisher` row is the interesting one. A valid signature from a key
nobody has vouched for proves the package has not changed since it was signed and
says *nothing* about who signed it. Conflating those is how a "signed = safe"
badge becomes misleading, so the two properties are separate methods on the type.

### Downgrades are refused

An older version than the one installed is refused. The older version is older
for a reason, and the usual reason is a fixed vulnerability. Going back requires
uninstalling first — a deliberate act, not a click in an update flow.

Versions compare semantically, never lexically: `1.10.0` is newer than `1.9.0`.

---

## What is designed but not built

| Area | Requirement | Status |
|---|---|---|
| Secure Boot verification | §6, §31 | Flag carried in `BootInfo`; the bootloader never sets it, and the kernel warns on every boot that the image was unverified — accurately |
| Signature verification itself | §59 | The *decision* given a verification result is implemented. The cryptography is not |
| Process isolation, sandboxing | §31 | Needs a scheduler and an address-space manager |
| Encryption at rest | §31, §32 | Needs the storage stack |
| Firewall | §28, §31 | Needs a network stack |
| Secure credential storage | §31 | Not started |
| Audit log | §63 | `Error::is_security_relevant` marks what belongs in it; nothing writes it yet |

Every one of these is listed in [STATUS.md](../STATUS.md) as well. The decisions
are made and tested; the machinery that enforces them is what remains.
