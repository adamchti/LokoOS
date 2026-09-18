# LKOFS — the LokoOS filesystem

LKOFS has two halves. The **policy half** — what a path is, what the roots mean,
and who may do what — is implemented and tested in `filesystem/lkofs-core`. The
**storage half** — on-disk format, journalling, encryption, device I/O — does not
exist yet.

That order is deliberate. The policy is the part that has to be right, and it is
far cheaper to discover it is wrong before ten subsystems depend on it.

---

## The path model

An LKO path looks like `LKO/Users/Adam/Documents/report.md`. The leading-slash
spelling `/Users/Adam/...` is accepted and canonicalises to the same thing.

### Rejection, not normalisation

**LKOFS refuses `.` and `..` rather than resolving them.**

Normalisation is where directory-traversal bugs live. Two components of a system
disagree about the order of "resolve symlinks" and "collapse `..`", and a path
that passed a check ends up pointing somewhere else. If there is exactly one
spelling of every location, there is nothing to disagree about.

Code that needs to walk upward uses `parent()`, which cannot escape the root: the
parent of `LKO/System` is `LKO`, and the parent of `LKO` is `None`.

### What a component may contain

| Rejected | Why |
|---|---|
| `.` and `..` | See above |
| `/` inside a name | Otherwise `join("a/b")` silently adds two levels, and a caller that validated the component validated the wrong thing |
| `\` | One separator, one spelling |
| NUL, C0/C1 controls, U+2028, U+2029 | Cannot be stored; the separators render as a newline and would let a filename fake a UI line |
| Bidi overrides: U+202A–202E, U+2066–2069, U+200E, U+200F | The "Trojan Source" class. `invoice<U+202E>fdp.exe` renders as `invoiceexe.pdf` |
| Trailing space or dot | Harmless in LKOFS; such names round-trip badly through the Windows compatibility layer, so they are rejected at creation rather than becoming unreachable later |
| Non-ASCII in the **root** component | Roots are a closed ASCII set. Rejecting non-ASCII here kills homoglyph attacks on protected roots as a class — `Ѕystem` with a Cyrillic Dze is simply not a root |

Unicode in ordinary filenames is fully supported. `Bericht über Größen.txt`,
`日本語のファイル.md` and `emoji 🎉 party.png` all work.

### Containment is component-wise

`is_within` compares component by component, never as a string prefix.

```
LKO/Users/Adam2          is NOT within  LKO/Users/Adam
LKO/Users/Adamant/secret is NOT within  LKO/Users/Adam
LKO/Users/Adam/Documents IS     within  LKO/Users/Adam
```

A string prefix test gets the first two wrong, and that is exactly how sandbox
escapes happen. Comparison is ASCII-case-insensitive, matching LKOFS's
case-insensitive lookup, so containment cannot be evaded by changing case
either.

---

## The nineteen roots

The top level is a **closed set**, known at compile time. Because it is closed,
"is this a protected location?" is answered without consulting any on-disk
metadata — which means the answer cannot be changed by anything an attacker can
write to disk.

| Root | Protection | Storage category |
|---|---|---|
| `System` `Drivers` `Boot` `Recovery` | System-immutable | Base OS / Recovery |
| `Services` `Config` `Data` `Logs` `Runtime` | System-managed | Base OS |
| `Apps` `Store` | System-managed | Applications |
| `Compatibility` | System-managed | Compatibility |
| `AI` | System-managed | AI models |
| `Linder` | System-managed | Cache |
| `Dev` | System-managed | Developer |
| `Users` `Lowser` | User-owned | User files / Cache |
| `Temp` `Cache` | Volatile | Temporary / Cache |

Some subtrees are stricter than their root, which `effective_protection` handles:

- **`LKO/Config/Security`** and **`LKO/Services/Security`** are system-immutable,
  not system-managed. Requirement 8 names `Security` among the locations to
  protect, and it is not a root, so it is handled here. They hold firewall rules,
  credential policy and signing trust anchors.
- **`LKO/AI/Memory`** is *user-owned*, not system-managed. Requirement 15 makes
  Loko AI's memory a privacy control, and a control the user cannot erase is not
  a control. Model weights under `LKO/AI/Models` stay system-managed.
- **`LKO/Runtime/Windows`** and **`LKO/Runtime/Mac`** are counted as
  *Compatibility* storage, not Base OS, while `LKO/Runtime/Loko` is Base OS.
  Requirement 2 excludes the compatibility runtimes from the 2 GB budget, and
  `storage_category_of` is where that exclusion is encoded.

The protection class also decides what LokoOS may delete unasked. Only `Volatile`
qualifies. Caches that *look* disposable still go through the user in Linder's
storage panel.

---

## The access policy

One function — `lkofs_core::evaluate(subject, path, operation)` — answers every
"may this?" question in LokoOS. It is pure: no I/O, no clock, no globals.

Rules are evaluated in order, most restrictive first. Reading them top to bottom
is reading the security model.

| # | Rule |
|---|---|
| 0 | The kernel is not subject to the policy it enforces |
| 1 | **Protected locations.** Nothing writes here except an open, verified update transaction — not the shell, not a service, not a driver, not the user |
| 2 | **Compatibility runtimes** cannot touch system locations at all, even to read. A Windows app enumerating LokoOS internals is a fingerprinting surface with no legitimate use |
| 3 | **Drivers** have authority over hardware, not files. Their own root (read), logs, and scratch. Nothing else |
| 4 | **Cross-user isolation**, ahead of any grant. A grant that names another user's directory does not open it |
| 5 | **Scratch space** is open. Forcing every app to negotiate a grant for `Temp` would train users to click yes |
| 6 | **Sandboxed apps** reach their own `AppData` and read their own install directory. They cannot rewrite their own installed code |
| 7 | **System-managed** locations are readable by anything that got this far — LokoOS is inspectable by whoever owns the machine — and writable only by system roles |
| 8 | **User-owned** locations need an explicit grant, except for the shell, which is the user operating their own machine |
| 9 | **Permission changes** always ask |
| 10 | **Loko AI** never destroys silently. Reads freely within its grant, creates freely, but `Write`, `Delete` and `Rename` all return `Confirm` |
| 11 | **Foreign applications** get the same treatment for writes to user data |

### `Confirm` is not `Allow`

The verdict type has three cases, and `Decision::is_allowed()` returns true for
exactly one of them. A caller that wants to treat "ask the user" as "yes" has to
say so explicitly. This is what makes requirement 16 — never let the AI silently
perform a destructive operation — a property of the type rather than a
convention.

Note that Loko AI *creating* a new file is allowed without a prompt. "Save this
summary to Documents" should not need a dialog; overwriting the summary that is
already there should.

### The subject classes

Not a ladder of privilege. Each role is constrained in a different direction: a
driver has authority over hardware the shell does not have, and no authority at
all over user documents.

`Kernel` · `SystemUpdater` · `SystemService` · `Driver` · `UserShell` ·
`UserApp` · `SandboxedApp` · `AiAgent` · `CompatibilityRuntime`

`SystemUpdater` carries a separate `update_transaction_open` flag. Outside a
transaction it has no standing authority at all, so a compromised updater
process sitting idle cannot write to a protected location.

---

## What is not built

- On-disk format, journalling, crash consistency
- Encryption at rest
- Device I/O and the block layer
- File watching, the search index (requirement 47)
- Extended attributes and tags

`lkofs-core` decides who may do what. It needs a device to decide it about.
