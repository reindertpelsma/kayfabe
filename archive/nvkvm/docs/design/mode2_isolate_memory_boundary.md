# The isolate memory boundary — how guest RAM is exposed, and bounded

**Status:** owner design, 2026-08-10, worked out jointly and recorded here because it existed only in
conversation. Companion to `mode2_guest_ram_crossing.md` (how guest RAM reaches the isolate at all)
and `mode2_channel_ownership_split.md` (why it must be mapped rather than copied).

⚠ **Marked throughout: what is decided, what is refuted, and what still needs building.**

---

## 1. What this is, and what it is not

**It is defense in depth.** The primary defense is that the guest never reaches the isolate with
anything unvalidated: every NVIDIA struct is decoded, every pointer translated explicitly, and
anything unknown is refused — the `nvproxy` model. In normal operation a guest gets neither arbitrary
memory nor RCE in the isolate, and **access to the isolate is itself gated** — a guest cannot obtain
arbitrary isolate access.

⊘ **It is not the allowlist.** There is a standing correction on this: three rustdoc sites were fixed
for calling the class allowlist "the security boundary". The boundary is *the set of operations we
are willing to issue*, plus *the VAS we populate*, plus — after this page — *the guest memory the
isolate can map*.

★ **It changes what the per-`Proc` isolate means.** The isolate exists for **VA identity, not
security** (standing ruling). So until now the per-`Proc` split was never a confidentiality boundary.
This page makes it one for guest memory, deliberately.

⊘ **And the naive version does not work.** One fd for the whole guest RAM block lets isolate *A* map
guest process *B*'s pages **and the guest kernel's**. That is a privilege the client process could
never have, so it is a real escalation relative to the thing the isolate serves — not merely
untidy. ⚠ An earlier argument of the assistant's — *"a compromised isolate already drives a real GPU,
which is more capability"* — is **wrong**: the GPU's reach is exactly the VAS we populate, bounded by
construction. A whole-block mapping is not. They are not comparable.

---

## 2. The mechanism

Guest RAM is a **memfd** (see `mode2_guest_ram_crossing.md` for how it becomes one). The fd is handed
to the isolate at a **fixed, known number**, and the seccomp filter is installed **after** that number
is known, so it can be hardcoded into the filter.

**Denied outright on that fd:** `read`, `write`, `lseek`, `ioctl`, and `close`.
**Denied so the number cannot move:** `dup`, `dup2`, `dup3`, `fcntl(F_DUPFD*)`.
⚠ **And reopening by path must be denied too** — `/proc/self/fd/N` is an `openat`, not a `dup`, and
would yield a fresh unpinned fd.

**Notified, not denied:** `mmap` — via `SECCOMP_RET_USER_NOTIF`, so the VMM inspects the arguments and
decides. ⚠ **`mremap` must be in the notified set as well**: it can extend an existing mapping past
what was approved.

★ **Why seccomp-notify is sound here, which it often is not.** `mmap`'s arguments are **all scalars**
— addr, len, prot, flags, fd, offset. There is no pointer for the isolate to rewrite after the
supervisor has read it, so the standard TOCTOU objection does not apply. This is one of the cases the
mechanism was made for.

⊘ **Decide what happens if the supervisor dies.** The isolate must **fail closed**, not fall through.

---

## 3. The transaction — and the rule that makes it strong

★★★★★ **THE LOAD-BEARING RULE: the VMM ORIGINATES the numbers; it never validates numbers the
isolate proposed.** If the isolate may say *"I would like offset X length Y"* and the VMM checks *"is
that inside guest RAM?"*, the check is **circular** — it validates a request against itself. The
`(offset, length)` must come from the VMM's own address-table derivation. This is the same rule as
*"no raw guest pointer reaches the isolate"*, and the same failure as
[an echo is unverifiable by its reply].

**The protocol** (owner, 2026-08-10):

1. The VMM takes the region **lock**.
2. The VMM takes a **reference** to the region on the isolate's behalf. Any other thread trying to
   free it now sees that an isolate holds it and knows it must be unmapped first.
3. The VMM queues *"mmap guest RAM at offset X, length Y"* to the isolate.
4. The VMM **releases the lock** — see step 8 for why this is safe.
5. The isolate issues `mmap`; seccomp notifies the VMM. (With seccomp off, this simply passes.)
6. The VMM receives the notification. The transaction is still in flight and **every parameter matches
   exactly**, so it accepts.
7. The isolate returns; the VMM completes the transaction and drops the authorization.
8. **`mmap`/`munmap` are serialized per isolate** (across isolates they run in parallel). So a free
   arriving between 5 and 8 queues its `munmap` *after* the mmap completes — the isolate's transaction
   must finish before the page can be freed, moved or otherwise revoked, **or the timeout fires and
   the isolate is killed and the reference released**.

**On isolate death, every guest-RAM reference it held is released automatically.**

### ★★ The distinction that carries the safety argument

**The lock and the reference are two different mechanisms with two different jobs**, and step 4 is
only safe because of it:

- the **lock** is short-lived mutual exclusion on the region's *metadata*;
- the **reference** is what blocks *reclamation*, and it outlives the lock.

⚠ Collapsing these into one concept is exactly how a later reader "simplifies" this by dropping the
reference at step 4. **Name them separately in the code.**

### What the serialization buys

★ Because `mmap`/`munmap` are serialized per isolate, there is **at most one outstanding
authorization**, and it lives exactly as long as the in-flight transaction. So this is a single
`Option<Authorization>`, **not** a pending set with expiry logic. ⊘ That also dissolves the *banked
authorization* hazard by construction: an authorization the isolate never consumes cannot be spent
later, because it is withdrawn when the transaction ends.

### Matching

Match on **more than offset and length**: `addr` (the VMM dictates it anyway, for VA identity),
`prot` (authorizing read-only and receiving `PROT_WRITE` is a silent escalation), and `flags`.
**Default-deny when nothing is pending** — an `mmap` on that fd outside a transaction is a
kill-worthy event, not a denial to log. **Timeout withdraws the authorization automatically**, not as
a separate cleanup step someone must remember.

### Freeing

The `munmap`-confirmed free is the right shape, and the reason is worth stating: the kernel frees
pages on `munmap` regardless — what the confirmation buys is **knowing the isolate has lost access
before the range is reused**. ⚠ **On timeout → kill, free after `waitpid` (reap confirmed), not after
`kill`** — between the signal and the mm teardown the mappings still exist, and reclaiming on the
signal reintroduces the use-after-free the reference was preventing.

⚠ **And the confirmation is a blocking wait.** There is currently **no completion-wait architecture**
— everything is synchronous inline and the reactor has never been reached. **Design this on the
reactor from the start**, or it becomes the tenth blocking site on a path that already has nine.

---

## 4. Fallbacks and configuration

### ★★ UID separation and userns COMPOSE — and UID is the common case, not the fallback

⊘ **An earlier revision of this page called UID separation a "fallback for when unprivileged userns is
unavailable". That has the availability backwards.**

- **`CAP_SETUID` is in Docker's DEFAULT capability set.** A container running a VM has it already.
- **Unprivileged userns inside a container is the thing that needs extra cooperation from the host** —
  relaxed seccomp/AppArmor profiles, or capabilities granted at `docker run`. For a container whose
  whole job is running a VM with `/dev/kvm` exposed, **`CAP_SETUID` may be genuinely sufficient and is
  the cheaper ask.**

⇒ **The design is: a distinct UID per isolate ALWAYS (cheap, default-available), PLUS userns when it
is available (strictly more).** They stack; this is not a ladder with one rung chosen.

⊘ **And a second correction: UID separation is NOT orthogonal to the memory boundary — it is what
stops the boundary being bypassed laterally.** The `SCM_RIGHTS`-passed memfd is beyond DAC, true. But
if isolate *A* can `ptrace` isolate *B*, or read `/proc/B/mem`, then every `mmap` check applied to *A*
is moot: *A* simply reads *B*'s already-approved mapping. **UID separation is what makes the per-`Proc`
mmap authorization mean anything at all.**

⚠ And it matters most in exactly the configuration where the memory boundary is weakest: with seccomp
**disabled** (the documented opt-out), UID separation is the only thing still standing between
isolates.

**Both are disableable, and both default ON.** ⚠ **The risk inverts when you do that**: this project's
own history is that **default-off paths never run** — the C's `m2hostsem`, `m2cexec` and `m2cefwd` were
all default-off and effectively untested, and one of them means its only green `cup2` had the GPU
doing nothing. With default-on, the **disabled** configuration becomes the untested one. ⇒ **The
disabled path must be loud**: named in the boot report (*"memory boundary DISABLED — unprivileged
userns unavailable"*), never silent. Keeping it switchable is also how the boundary gets proven: a
gate you cannot turn off cannot be A/B'd, and this campaign has found several vacuous ones.

**Cost:** none on the hot path. `SECCOMP_USER_NOTIF` fires on `mmap`, which happens at **bind/alloc
time**. The pushbuffer, ring and doorbell never touch it.

---

## 5. Build order — the shape now, the enforcement later

★ **"Hard to retrofit" is right, and it is because this is a SHAPE, not a check.** If the crossing
lands without it, every call site is written assuming *"I can map what I need"*, and retrofitting means
auditing and rewriting all of them. Checks can be added later; shapes cannot.

⇒ **Split it:**
- **The interface — now.** *The isolate never `mmap`s guest RAM on its own; it is instructed.* Small,
  on the critical path anyway, and the irreversible part. ★ It also makes the seccomp check trivial
  later: when the VMM instructs a specific offset+length, the notification follows immediately with
  the same parameters, and acceptance is a match rather than a policy decision.
- **The enforcement — later, behind that interface.** fd pinning, the filter, the notify loop, the
  `munmap` confirmation. All of it lands **without touching a single call site**, because the shape
  already forced everything through one door.

---

## 6. ⊘ Refuted alternatives

**Copying guest RAM into the isolate and back.** Tempting — the VMM already has guest RAM mapped — and
**it does not work.** The guest **polls** its completion semaphore directly out of its own RAM. A poll
has **no trigger point** at which we could copy back: no event, no ioctl, no exit. The same holds for
the ring, where the guest advances `GP_PUT` and expects the engine to see it. ★ And the C agrees: for
guest RAM (sysmem) it **mapped** — `pci_dma_map` → host VA → `OS_DESCRIPTOR` over the real pages. It
copied only for emulated-framebuffer seeding, which is memory **we** own.

**The Mode-1 trick — an ephemeral memfd substituted for "guest RAM" as PCIe memory.** ⊘ Only possible
because a **cooperating guest kernel module** was part of that project. With a stock driver there is no
such cooperation, so it does not transfer.
