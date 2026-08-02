# What the guest can and cannot do to the host GPU

> **Status: threat-model statement plus a source audit. Task #129, written 2026-07-31 against
> `ogkm-580.159.04` and this tree at `225126f`. No GPU was switched on for this document.**
>
> **★ Amended 2026-08-01 (task #133, the actionable half).** Three changes, each marked in
> place: **F1's census was re-counted and CORRECTED** (591 → 613 closed, 43.5 % → 45.1 %; 22
> entries carry both `INTERNAL` and `NON_PRIVILEGED` and are closed by `control.c:799-803`, a
> gate §3.3 did not cite); **F11's `hClient` invariant is now STRUCTURAL** and its resolution
> block names the compiler errors that were watched firing; and **§3.4 was added** to state,
> in one place, that P is *not* carried by our allowlist (F5) and that the Case-1/Case-2 split
> is *not* a security mechanism (F13). Still no GPU, and still no run behind the security
> reasoning itself.

## 0. The epistemic frame — read before quoting anything below

⊘ **I ran no hardware for this file.** Where a statement is about NVIDIA's driver it is a
**reading** of the open kernel modules at a named file:line: that says what the driver *does*,
never what *happens*. Where a statement is about our own tree it is either a reading of the
code or a citation of somebody else's run, named as theirs. `claim_ledger.md` names the
distinction and this file obeys it; labels are used strictly, following the scheme
`compute_limiting_and_priority.md` §0 established:

| label | meaning |
|---|---|
| **[src@580]** | read out of `research_clones/ogkm-580.159.04/`, cited file:line. Nothing ran. |
| **[src-rust]** | read out of this tree at `225126f`, cited file:line. Nothing ran. |
| **[run: …]** | somebody else's experiment, named — commit, date, and the box it ran on |
| **[inferred]** | a conclusion drawn from one or more of the above |
| **[unknown]** | nobody here knows, and this file says so instead of guessing |

★ Two counts in §4 are **census arithmetic over the ogkm tree**, re-derived for this document
rather than carried across. A census is a reading of 1 359 table entries; it is `[src@580]`
like every other reading here, and it has no machine behind it.

---

## 1. The property, stated so it can be falsified

The owner's requirement was *"the guest should be unable to brick the host gpu, I think largely
already impossible by unprivileged construction"*. The intuition is right and the construction
is real, but **"cannot brick" is not a checkable sentence** — it quantifies over every possible
persistent harm, names no mechanism, and would be exactly the unearned absolute this project's
own ledger exists to catch. What the construction actually buys is narrower, stronger, and
testable:

> ### ★★★ P — the blast-radius property
>
> **Given a VMM that is doing what it was written to do, every effect a hostile guest can
> produce on the host GPU is an effect a LOCAL UNPRIVILEGED PROCESS on the same host could
> already produce.**
>
> Falsified by exhibiting one host-GPU effect reachable **through the guest interface** that
> is not reachable by an unprivileged local process holding no capability.

## ★★★ 1.1 — THE TRUST BOUNDARY, and the correction of 2026-07-31

This section is a **correction**, made on the owner's ruling, and the sentence that
occasioned it is quoted verbatim rather than paraphrased:

> *"(a) isn't really possible since the VMM does much more than only our project, we can't
> sandbox that, it isn't our job also (that's the VMM's job). Our worst case scenario for
> security is if the VMM (the hypervisor) is compromised; a further compromise is out of
> scope (like the isolated container/namespace the VMM runs in)."*

⇒ **A compromised VMM is the BOUNDARY, not a step inside it.** Everything this document
argues is argued about what a *guest* can reach through a VMM that is behaving. The VMM is
a large program that hosts far more than this project, confining it is the deployment's job
and not ours, and treating "and then the VMM is also compromised" as a step in an attack
chain both over-claims and mis-assigns the work.

Three consequences, and the third is the one that changed a finding below:

1. **P is stated relative to a trusted VMM**, above, in the property itself rather than in a
   footnote. The falsifier is correspondingly narrowed: a counterexample must be reachable
   **through the guest interface**, not through code running inside the VMM's own address
   space.
2. **VMM compromise is EXPLICITLY OUT OF SCOPE**, and so is anything downstream of it —
   including whatever container, namespace or service manager the VMM itself runs in. It is
   named here so that "we did not think about it" is never a reading available to a later
   auditor. It is not a residual risk this project tracks; it is a boundary this project
   draws.
3. ★★★ **The seccomp filter §4 F14 recommends is NOT load-bearing for P.** F14 found that an
   RM descriptor in a root VMM yields the 265 `PRIVILEGED` controls an isolate cannot reach,
   and this document concluded that a `seccomp` filter on the VMM was therefore load-bearing
   for P. Under the ruling above **that conclusion does not follow**: reaching those controls
   requires issuing an `ioctl` from inside the VMM, which is code the guest does not run and
   which a behaving VMM does not contain. A guest cannot make a behaving VMM issue an RM
   escape on a descriptor that VMM never uses that way.

⊘ **F14 itself is NOT withdrawn, and must not be deleted.** Its *mechanism* — that RM derives
privilege from the **caller at ioctl time** and not from the opener — is a true and load-
bearing fact about the driver, and it is precisely the reason not to hand the descriptor up
in the first place. What changes is its status: from *"a latent violation of P awaiting a
mitigation"* to *"the standing argument for a design decision the owner has since taken."*
That decision is `#133`'s **(b)** — the mapping moves behind an isolate verb, so the GPU
descriptor never crosses upward at all — and it is chosen for **hygiene and contract**, not
to close F14. See `isolate_vmm_fd_crossing.md` §12 for the verb, and for the half of the
problem it does **not** cover.

Three things P deliberately does and does not say, because each has been misread already:

- **P is a comparison, not a safety claim.** It says the guest is no worse than a local
  unprivileged process. It does **not** say either of them is harmless. If the host driver has
  a bug reachable from an unprivileged `ioctl`, the guest reaches it too, and P is still true.
- **The reference class is a PROCESS, not the CUDA runtime.** An unprivileged local process may
  issue any non-privileged RM escape with any payload it likes; it is not confined to the calls
  libcuda happens to make. That makes P more defensible and also weaker than a casual reader
  will assume, so it is written out here rather than left to inference.
- **P is about the host GPU.** The host *kernel*, the VMM process and cross-tenant isolation
  inside our own system are separate boundaries with their own documents
  (`core_security_threat_model.md` for the core's logical invariants,
  `l1_os_shell.md` for the OS seams). §4 F8 records one place the scopes touch.
- ★ **P is a statement about THIS tree, and it is false of the C artifact** — §4 F12. The C is
  this project's standing oracle, so that distinction has to be carried with the claim rather
  than assumed.

---

## 2. ★★ BRICK and WEDGE are different, and the difference must not be implied away

| | **BRICK** | **WEDGE** |
|---|---|---|
| what it is | persistent damage that survives a reboot | a hung engine needing a GPU reset |
| examples | VBIOS/firmware flash, fuses, falcon microcode, ECC configuration, persistence mode | a non-terminating kernel; a malformed pushbuffer |
| needs privilege? | **yes** — §4 F1/F5 | ★★★ **no** |
| in reach of a cap-dropped isolate? | **no** | ★★★ **yes** |
| in reach of a local unprivileged process? | no | **yes** |

⇒ **The unprivileged construction answers BRICK and does not touch WEDGE.** A wedge is one
submission that never returns; every lever our layer holds acts before the ring or after the
completion, so once the doorbell has been rung we hold nothing
(`compute_limiting_and_priority.md` §6.3). On a single-tenant host a wedge costs the tenant its
own GPU. On a shared host it is a denial of service to every co-tenant, and it is *the*
multi-tenancy exposure. §5 states where v1 stands on it.

★ Note that WEDGE sits **inside** P, not outside it: an unprivileged local process can wedge the
GPU too. P is therefore true and the exposure is real at the same time, which is precisely why
P has to be stated as a comparison rather than as an absolute.

---

## 3. Why P holds — three mechanisms, in the order they bind

### 3.1 The isolate holds no capability, and it opens the device only afterwards

Every host GPU operation in this system is issued by an isolate child process — ⚠ with one
qualification that landed the same day as this document and is taken up in full at F14: since
`82945eb` (task #131) a GPU descriptor can be handed **up** to the VMM over `SCM_RIGHTS`, so what
is unconditionally true is that the isolate is the process that **opens** the device. That child
surrenders `PR_SET_NO_NEW_PRIVS`, `PR_SET_DUMPABLE`, the whole capability bounding set, the
three capability sets via `capset`, and the ambient set, and it lives in a user namespace of its
own (`crates/kayfabe-linux-raw/src/sandbox_unsafe.rs:532-565`, entered at `:602-625`)
**[src-rust]**. The drop is **read back** with `capget`/`prctl` and one surviving bit is an
`Err` that discards the `/dev` descriptor on the way out (`:548-563`) **[src-rust]** — so the
argument is about an observed end state rather than about a sequence of calls having been made.

That end state was measured by the author of the drop, before and after, and the run is named
there rather than here: `[run: commit 2575177, 2026-07-30, RTX 3060 / 580.159.04, root VMM]`.
The falsification instruments are committed alongside it and can fail —
`crates/kayfabe-isolate-host/tests/sandbox_escape.rs::the_sandboxed_child_holds_no_capability_at_all`
(`:412`), `::the_sandboxed_childs_capability_ceiling_is_empty_when_it_could_be_emptied` (`:473`)
and `::the_sandboxed_child_lives_in_its_own_user_namespace` (`:515`), each with a deliberately
mis-ordered control mode kept executable beside it (`:318`).

★ **The order is the whole property.** `sandbox::enter` calls `surrender_privilege()` **last**
and only then returns the `/dev` descriptor (`sandbox_unsafe.rs:808,828-830`), and the isolate's
`build_backends` opens `/dev/nvidiactl` and `/dev/nvidia<N>` from that descriptor on the next two
lines (`crates/kayfabe-isolate-host/src/child.rs:209-212`) **[src-rust]**. Every RM `ioctl` this
system issues therefore happens after the surrender. F2 in §4 is why that ordering is
load-bearing rather than tidy, and F7 is where it is weaker than it looks.

### 3.2 RM re-derives the caller's privilege on every single ioctl

`RmIoctl` sets `secInfo.privLevel = osIsAdministrator() ? RS_PRIV_LEVEL_USER_ROOT :
RS_PRIV_LEVEL_USER` at the **top of every escape**
(`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:304`) **[src@580]**, and
`osIsAdministrator()` bottoms out in `NV_IS_SUSER()` = `capable(CAP_SYS_ADMIN)`
(`ogkm-580: src/nvidia/arch/nvalloc/unix/src/os.c:614-617` →
`ogkm-580: kernel-open/nvidia/os-interface.c:378-381` →
`ogkm-580: kernel-open/common/inc/nv-linux.h:537`) **[src@580]**. `capable()` is asked of the
**initial** user namespace, so entering one of our own does not restore it `[inferred]`.

⇒ A process that holds no capability is `RS_PRIV_LEVEL_USER` on every call it will ever make,
and there is no call it can make to become otherwise `[inferred]`.

### 3.3 ★★★ RM's control plane is kernel-privileged by DEFAULT, and that is what makes P structural

`rmControlValidateClientPrivilegeAccess`
(`ogkm-580: src/nvidia/src/kernel/rmapi/control.c:675-712`) **[src@580]** applies two rules:

- a control carrying `RMCTRL_FLAGS_PRIVILEGED` (`0x4`, `control.h:202`) is refused
  `NV_ERR_INSUFFICIENT_PERMISSIONS` below `RS_PRIV_LEVEL_USER_ROOT` (`control.c:686-699`);
- ★ a control carrying **none** of `NON_PRIVILEGED` / `PRIVILEGED` / `INTERNAL` requires
  `RS_PRIV_LEVEL_KERNEL` (`control.c:702-711`) — a level no userspace escape can reach, since
  `escape.c:304` can only produce `USER` or `USER_ROOT`.

⚠ **A third rule lives outside this function, and F1's ★ CORRECTION is what found that.**
`RMCTRL_FLAGS_INTERNAL` is *not* refused by `rmControlValidateClientPrivilegeAccess` — in the
second rule above it is one of the flags whose presence **satisfies** the check. What closes
it is `if (ctrlFlags & RMCTRL_FLAGS_INTERNAL) { if (!pRmCtrlParams->bInternal) return
NV_ERR_NOT_SUPPORTED; }` (`ogkm-580: src/nvidia/src/kernel/rmapi/control.c:799-803`)
**[src@580]**. Same file, same unconditional path, different function — but reading the
`INTERNAL` column off this section alone under-counts the closed set by 22.

**This is the sentence the whole property rests on:** the bar is applied to *the credentials of
the caller*, not to *which control the caller chose*. Nothing we forward, allow, mis-adjudicate
or fail to filter can change the answer, because we are not the one being asked.

### ★★★ 3.4 What makes P true is NOT our allowlist — and the docs must not say it is

Stated separately because it is the single most quotable sentence in this document and it has
already been mis-stated elsewhere in the tree:

> **P holds because RM re-derives the caller's privilege on every ioctl and refuses by
> default. It does not hold because of anything we allow, deny, divert or filter.**

Three consequences, each of which strikes a citation that was being made:

1. ⊘ **The `CapabilityTable` allowlist/denylist does not carry P** — §4 F5. All six of our
   explicitly denied controls are `NON_PRIVILEGED` in RM, i.e. reachable by any unprivileged
   local process already. Deleting the whole table tomorrow would leave P standing. What the
   table buys is **cross-tenant isolation inside our own system**
   (`core_security_threat_model.md`) and defence in depth — real things, different property.
   Writing it the other way round would make P depend on a table we maintain, which is exactly
   the fragility P was formulated to avoid.
2. ⊘ **The Case-1/Case-2 control split is a CORRECTNESS mechanism and must not be cited as
   security** — §4 F13. `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` is `PRIVILEGED` and
   `NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_INFO` is kernel-only, **both are on our ingress
   allowlist**, and admitting them breaches nothing because RM refuses both regardless. The
   split exists so an unprivileged replay does not come back `InsufficientPermissions`. It is
   not a containment boundary and it has none of a boundary's obligations.
3. ★ **The scale of the driver's own default is the argument.** 613 of 1 359 exported controls
   — **45.1 %** — are closed to every userspace caller before our layer says anything (F1).
   Our allowlist is defence in depth over a decision the host driver makes again regardless.

⊘ **No run stands behind §3.3/§3.4, and none behind F1 either.** All of it is `[src@580]` or
`[inferred]`. F1's census was independently **re-derived** on 2026-08-01 against ogkm
**580.159.04**, which caught a 22-entry error — but a census is a parser over source text, not
a measurement, and none of this has been in front of a running driver.

---

## 4. The audit — what the forwarded op set actually reaches

The host-facing surface is small enough to enumerate: **nine** RM escapes and **one** `mmap` of
a device fd, all in `crates/kayfabe-isolate-host/src/rm.rs` **[src-rust]** — `CHECK_VERSION_STR`
(`:1285`), `REGISTER_FD` (`:526`), `RM_ALLOC` (`:711`), `RM_CONTROL` (`:868`),
`RM_MAP_MEMORY_DMA` (`:924`), `RM_UNMAP_MEMORY_DMA` (`:949`), `RM_MAP_MEMORY` (`:1013`),
`RM_ALLOC_MEMORY` (`:1531`), `RM_FREE` (`:2576`), and `map_cpu`'s single `mmap` (`:1037`), plus
one 32-bit store into the mapped usermode window that is the doorbell (`:1744-1755`). Each
finding below asks the same question of one of them: *does it exceed an unprivileged local
process?*

### F1 — The census: 45.1 % of RM's control plane is unreachable from userspace at all

Re-derived for this document over the generated exported-method tables
(`ogkm-580: src/nvidia/generated/*.c`, 1 359 entries) **[src@580]**:

| class | count | who can call it |
|---|---|---|
| `NON_PRIVILEGED` (`0x8`) **and nothing stronger** | 746 | any unprivileged process — **and therefore us** |
| `PRIVILEGED` (`0x4`) | 265 | `CAP_SYS_ADMIN` in the initial userns only |
| `INTERNAL` (`0x80`, not also `PRIVILEGED`) | 233 | RM-internal callers only |
| `KERNEL_PRIVILEGED` (default, no flag) | 115 | `RS_PRIV_LEVEL_KERNEL` only |

⇒ **613 of 1 359 (45.1 %) are closed to every userspace caller**, us included `[inferred]`.
★ This census is **not** in `compute_limiting_and_priority.md` §4.2, which counted only the
*access-right* field. Both are true and they measure different gates; F4 says why the
difference matters.

#### ★ CORRECTED 2026-08-01 — this table used to read 768 / 211 / **591 (43.5 %)**

**[src@580]** — and, per §0, a census is a **reading**, not a measurement: nothing ran but a
parser over source text. Re-derived independently on 2026-08-01 against ogkm tag
**580.159.04** (named because `ogkm` is versioned and the vendored tree elsewhere in this
project is **610.43.02**, which does not agree with it); the tag was taken from `version.mk`
`NVIDIA_VERSION`, and the parser read the `/*flags=*/ 0x…u` literals inside every
`__nvoc_exported_method_def_*[]` table under `src/nvidia/generated/*.c`.

The re-count reproduced **1 359 total**, **265 `PRIVILEGED`** and **115 kernel-by-default**
*exactly*, so the universe and the parser agree with the original. One cell did not:

- **22 entries carry BOTH `INTERNAL` and `NON_PRIVILEGED`** — counted, and the arithmetic
  closes on both sides: `233 − 211 = 22` and `768 − 746 = 22`, the same 22 entries. The
  original count credited them to the *reachable* side.
- They are **not** reachable. `RMCTRL_FLAGS_INTERNAL` is enforced independently of the
  privilege-level rules: *"if the `INTERNAL` flag is specified, the call will only be allowed
  to be issued from RM itself. Otherwise, `NV_ERR_NOT_SUPPORTED` is returned"*
  (`ogkm-580: src/nvidia/inc/kernel/rmapi/control.h:234-239`), implemented as
  `if (ctrlFlags & RMCTRL_FLAGS_INTERNAL) { if (!pRmCtrlParams->bInternal) return NV_ERR_NOT_SUPPORTED; }`
  (`ogkm-580: src/nvidia/src/kernel/rmapi/control.c:799-803`) **[src@580]**. Carrying
  `NON_PRIVILEGED` as well does not soften it — that clause never reads the privilege level.

⇒ The correction moves **22 controls from the reachable side to the closed side**, which makes
§3.3's argument *stronger*, not weaker. It is recorded rather than silently patched because a
census is the kind of number that gets quoted downstream.

⚠ **And the citation in §3.3 was imprecise, which is how the miscount survived.** §3.3
attributes all three rules to `rmControlValidateClientPrivilegeAccess`
(`control.c:675-712`). That function implements only **two** of them — it refuses
`PRIVILEGED` below `USER_ROOT`, and it refuses when *none* of
`NON_PRIVILEGED`/`PRIVILEGED`/`INTERNAL` is set. It does **not** refuse `INTERNAL`; there,
`INTERNAL` *satisfies* the second clause. The gate that closes `INTERNAL` is the separate one
at `control.c:799-803` above. Both are inside `rmapi/control.c` and both are unconditional,
so the conclusion is unchanged — but "closed by §3.3's rules" was not literally true of the
`INTERNAL` column, and anyone re-deriving the table from §3.3 alone would land back on 591.

### F2 — RM caches a privilege level per client, and our ordering is what closes it

The most promising counterexample I could construct, and it is closed rather than absent.
`__nvoc_ctor_RmClient` stores `pClient->cachedPrivilege = pSecInfo->privLevel` at **client
allocation time** (`ogkm-580: src/nvidia/src/kernel/rmapi/client.c:95`) and roughly thirty sites
read it back through `rmclientGetCachedPrivilege` (`client.c:375-380`) **[src@580]**. An
`NV01_ROOT` allocated while the process still held `CAP_SYS_ADMIN` would carry `USER_ROOT` **for
the lifetime of the client**, regardless of what the process surrendered afterwards
`[inferred]`.

It does not happen here, for the ordering reason in §3.1: the device fd is minted after
`surrender_privilege()` and the RM client ladder runs on that fd
(`child.rs:209-212`, `rm.rs:554`) **[src-rust]**. ⚠ But it is worth stating as the *reason* the
ordering matters, because the ordering currently reads as filesystem hygiene and it is also the
thing standing between us and a permanently-admin RM client. See F7 for what still guards it.

★ One right is immune to this by construction and it is the one that matters most for §5:
`RS_ACCESS_NICE` carries `RS_ACCESS_FLAG_UNCACHED_CHECK`
(`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_access_rights.c:46-49`), which forces
re-evaluation rather than a latch (`rs_access_map.c:232-240`) **[src@580]**.

### F3 — ★ A correction to `compute_limiting_and_priority.md` §4.1: `RS_ACCESS_NICE` has TWO grant paths

That note says the only way to hold `RS_ACCESS_NICE` is `privLevel >= RS_PRIV_LEVEL_USER_ROOT`
via `osIsAdministrator()`. There is a second, independent path: after the privileged arm fails,
`_rsAccessGrantCallback` invokes the resource's own access callback
(`ogkm-580: src/nvidia/src/libraries/resserv/src/rs_access_map.c:533-536`), and for the client
resource that is `cliresAccessCallback_IMPL` → `osCheckAccess(RS_ACCESS_NICE)`
(`ogkm-580: src/nvidia/src/kernel/rmapi/client_resource.c:141-156`) →
**`capable(CAP_SYS_NICE)`** (`ogkm-580: kernel-open/nvidia/os-interface.c:395-398`)
**[src@580]**.

The conclusion is unchanged and now rests on two legs instead of one: both are live `capable()`
checks against the initial user namespace, and a process holding no capability fails both
`[inferred]`. The correction matters because the note's single-path phrasing invites the fix
*"just grant `CAP_SYS_ADMIN`"*, which would not be the whole answer, and because a reader
auditing the scheduling controls needs to know which capability to look for.

### F4 — The "exactly five" count is CORRECT, re-derived, and should not be softened

The brief flagged it as unconfirmed. `grep -h "accessRight=" src/nvidia/generated/*.c` over
`ogkm-580` returns **1 354 entries at `0x0` and exactly 5 at `0x2`**, with no third value, and
`accessRight=` occurs **nowhere in the tree outside `src/nvidia/generated/`** **[src@580]**. The
five are the two `SET_INTERLEAVE_LEVEL`s, `MAKE_REALTIME`, `RESTART_RUNLIST` and
`FIFO_RUNLIST_SET_SCHED_POLICY`, matching that note's table row for row. ⇒ **No softening
needed.**

⚠ What *does* need a caveat is the inference drawn from it. "The entire tree has a tiny
access-right surface" is true and reads as "a tiny privilege surface", which is false: F1's 265
`PRIVILEGED` controls are a **53× larger** gate that the access-right census does not see, and
`RS_ACCESS_NICE` is one narrow right layered on top of it, not the driver's main bar
`[inferred]`.

### F5 — ★★ Our own denylist is STRICTER than the host driver, so it does not carry P

`CapabilityTable` refuses by default — `ControlPermit::Denied(NotOnAllowlist)`
(`crates/kayfabe-abi/src/capability.rs:509`) and the same for alloc classes (`:534`)
**[src-rust]** — with six explicitly denied controls (`:1197-1230`). Cross-referencing all six
against RM's own tables **[src@580]**:

| control | RM class | accessRight |
|---|---|---|
| `NV00E0_CTRL_CMD_IMPORT_MEM` `0x00e00102` | `NON_PRIVILEGED` | `0x0` |
| `NV00F1_CTRL_CMD_DISABLE_IMPORTERS` `0x00f10003` | `NON_PRIVILEGED` | `0x0` |
| `NV2080_CTRL_CMD_GPU_EXEC_REG_OPS` `0x20800122` | `NON_PRIVILEGED` | `0x0` |
| `NV2080_CTRL_CMD_NVLINK_GET_PLATFORM_INFO` `0x20803083` | `NON_PRIVILEGED` | `0x0` |
| `NVB0CC_CTRL_CMD_ALLOC_PMA_STREAM` `0xb0cc0105` | `NON_PRIVILEGED` | `0x0` |
| `NVB0CC_CTRL_CMD_EXEC_REG_OPS` `0xb0cc010a` | `NON_PRIVILEGED` | `0x0` |

⇒ **All six are reachable by any unprivileged local process.** Denying them buys us something
real for a *different* boundary — cross-tenant isolation inside our own system, which is
`core_security_threat_model.md`'s subject — but it contributes **nothing** to P, and if the
allowlist were deleted tomorrow P would still hold on §3.3's mechanism `[inferred]`. Stating it
the other way round would be the mistake: it would make P depend on a table we maintain.

#### ★ STRUCK 2026-08-01 — and three code citations still to strike, located

A tree-wide sweep for places that cite the allowlist/denylist as the *security* property, or
the Case-1/Case-2 split as a containment mechanism, was run on 2026-08-01 over all 57 `.md`
files, every `crates/*/src/**.rs` and `crates/*/tests/**.rs`, the root `tests/` crate, and all
23 `Cargo.toml` descriptions.

**F13 / Case-2: the tree is CLEAN.** Every site that mentions `classify_control`, the Case-2
set or the `PROMOTE_CTX`/`GET_CTX_BUFFER_INFO` diversion already uses the correctness framing,
and four of them say *"wrong layer, never a privilege gain"* outright
(`kayfabe-arch/src/lib.rs:622-627`, `kayfabe-fwd/src/lib.rs:1608-1612` and `:1927-1929`,
`kayfabe-isolate/src/lib.rs:166-170`, `kayfabe-isolate-host/src/rm.rs:380-384`,
`execution_plane.md:225-226`). **No edit is owed.** Recorded as a *verified negative* so the
next auditor does not re-run the sweep.

**F5 / allowlist: three rustdoc sites still overstate it**, and they are named here with their
current wording because ⊘ **this commit deliberately did not edit them** — `kayfabe-abi` and
`kayfabe-rmrpc` were both being modified by a concurrent `GspRmAlloc` task, and a doc-comment
edit landing in the middle of that is a merge conflict for no urgency. They are latent
documentation defects, not live security defects.

| site | current wording | why it overstates |
|---|---|---|
| `crates/kayfabe-rmrpc/src/lib.rs:389-392` | *"the only thing standing between a guest and an arbitrary class was whether we happened to decode it, so adding a decoder widened the **security boundary**"* | promotes the table from mechanism to **the** security boundary. It is not one: RM adjudicates the class regardless. Correct restatement: adding a decoder used to widen **what we permit**, which is a policy surface, not P. |
| `crates/kayfabe-abi/src/capability.rs:50` | section heading *"## Ordering, which is a security property"* | the table's internal resolution order is a **review/policy** property. The paragraph under it is fine; the heading is the false citation. |
| `crates/kayfabe-abi/src/capability.rs:213-216` | *"there is no version of this we want reachable from a guest"* (on the `EXEC_REG_OPS` deny rows) | reads as *our deny row makes it unreachable*. It does not — both rows are `NON_PRIVILEGED` in RM. ★ The **neighbouring** `DeniedBecause::SmDebuggerTrapping` doc at `:237-241` already gets this exactly right and says the surface *is* reachable by an unprivileged app; bring these up to that standard. |

★ `EXEC_REG_OPS` deserves its own sentence, because "unprivileged arbitrary register access"
would be a startling thing to leave implied. It is non-privileged **at the dispatch layer only**;
each offset is then checked against a per-register allowlist unless the caller is admin
(`gpuValidateRegOffset_IMPL`, `ogkm-580: src/nvidia/src/kernel/gpu/gpu_access.c:1613-1640`)
**[src@580]**. So the unprivileged reach is *allowlisted* registers, not arbitrary ones.

### F6 — The two blanket permits are far narrower than the standing alarm, and their residue is unknown

`CapabilityTable::control` short-circuits its allowlist for any command with bit 15 set
(`RM_GSS_LEGACY_MASK`, `capability.rs:152,483-485`) and for class `0x2081`
(`NV2081_BINAPI_CLASS`, `:157,486-488`) **[src-rust]**. The standing worry about this is recorded
as a *"2^31 unreviewed commands"* alarm that was itself adjudicated as counting address space
rather than risk. Counting the reachable set instead **[src@580]**: of the 1 359 exported
entries in `ogkm-580`, **two** have bit 15 set — one `NON_PRIVILEGED`, one `INTERNAL` — and
**zero** belong to class `0x2081`.

⇒ The bit-15 permit waves through **one** control that RM will actually execute for an
unprivileged caller `[inferred]`. That is a much sharper statement than the alarm, and it is
consistent with the adjudication's reasoning: the space is GSP-serviced, and the open tree has no
body for it. ⚠ **What the host driver does with a bit-15 command it has no table entry for is
`[unknown]`** — the lookup has no entry, the routing to GSP is firmware, and neither is readable
here. It is bounded by §3.3 either way: whatever it dispatches to is adjudicated against our
credentials.

### F7 — ⚠ The ordering that closes F2 is enforced by call order, not by the type system

`sandbox::enter` returns a `DevDir` and `RmConnection::open` consumes one, which reads as a
capability token. It is not one: `DevDir::open` is `pub` and unbounded, its own doc says so
(`crates/kayfabe-linux-raw/src/chardev_unsafe.rs:95`, `:473`), and two binaries call it directly
with whatever privilege their caller holds (`bin/rmladder.rs:256`, `bin/sandbox_probe.rs:186`)
**[src-rust]**. Those two are diagnostics and say they are not the isolate, so this is not a live
defect. But P's most load-bearing step is currently held by the order of two adjacent lines in
`child.rs`, and nothing in the type system stops a future production caller from minting a
descriptor — and an RM client — before the surrender.

⇒ **Named follow-up, cheap:** make the production RM path require a token only
`sandbox::enter` can mint, so F2's counterexample becomes unrepresentable rather than
merely not-currently-written. This is the same shape as the confused-deputy hardening
`core_security_threat_model.md` §4 applied to handle resolution.

### F8 — The one seam where P is not yet established, because the thing is not built

`/dev/nvidia-uvm` is deliberately excluded from the isolate's sandbox policy, on the stated
grounds that *"UVM is opened by the VMM process, never by a sandboxed isolate"*
(`crates/kayfabe-linux-raw/src/sandbox_unsafe.rs:221-222`, pinned by
`::a_gpu_policy_never_contains_the_uvm_node` at `:983`) **[src-rust]**. The VMM is the privileged
side. Today this is moot — **nothing in this tree opens `/dev/nvidia-uvm` at all**, and the UVM
plane is unbuilt **[src-rust]** — but §3's argument is an argument about the isolate, and it will
not cover a UVM path that lives in the VMM. ⇒ When that path is built, P must be re-established
for it explicitly; it does not inherit.

### F9 — Absent seccomp does not bear on P, and saying why is worth a line

There is no seccomp filter in this tree; it is named rather than stubbed
(`crates/kayfabe-isolate-host/src/lib.rs:57-60`) **[src-rust]**. It is tempting to file that
under P and it does not belong there: a local unprivileged process has an unfiltered syscall
surface too, so an unfiltered isolate does not *exceed* the reference class `[inferred]`. What
seccomp buys is defence in depth for the host **kernel** boundary if the isolate is ever
compromised through the driver — a different property, correctly tracked in the OS-shell docs
rather than here.

### F10 — Guest-chosen class ids and payloads reach the driver, and that is inside P

`RmBackend::alloc` forwards a guest-chosen class id (`rm.rs:1441`), `alloc_engine_object`
forwards a guest-chosen class **and** params blob (`rm.rs:1629`), and `HostRmBackend::control`
forwards any command with any payload verbatim (`rm.rs:1701-1711`) **[src-rust]**; the
`CapabilityTable` gate in front of them is the default-deny of F5. This is the largest
guest-controlled surface we present to the host driver, and it is nonetheless **inside P**: an
unprivileged local process can issue `NV_ESC_RM_ALLOC` and `NV_ESC_RM_CONTROL` with any class,
command and blob it likes `[inferred]`. ⚠ What follows from that is the caveat in §1: P being
true means a malformed-payload bug in the host driver is *equally* reachable, not unreachable.

### F11 — ★★★ The isolate's kernel-visible euid is the VMM's, and RM keys a real check on euid

**This is the closest thing to a counterexample in the tree, it is not a capability question at
all, and it survives every mechanism in §3.**

`surrender_privilege` drops capabilities. It does not change uid, and it cannot: the user
namespace map is written from `outer_ids()` as the single line `0 <outer_uid> 1`
(`crates/kayfabe-linux-raw/src/sandbox_unsafe.rs:596-617`) **[src-rust]**, so on a VMM running as
root the isolate's uid **as the host kernel sees it** is 0. `NV_CURRENT_EUID()` is
`__kuid_val(current->cred->euid)` (`ogkm-580: kernel-open/common/inc/nv-linux.h:156`) — the
initial-namespace value — reached through `os_get_euid`
(`ogkm-580: kernel-open/nvidia/os-interface.c:1431-1435`) **[src@580]**.

RM uses that euid as a client security token, and the check is an **OR**:

```c
if ((pClientTokenUser->euid != pCurrentTokenUser->euid) &&
    (pClientTokenUser->pid  != pCurrentTokenUser->pid))
    return NV_ERR_INVALID_CLIENT;
```

(`ogkm-580: src/nvidia/arch/nvalloc/unix/src/os.c:3844-3868`, driven from
`_rmclientUserClientSecurityCheck`, `ogkm-580: src/nvidia/src/kernel/rmapi/client.c:447-512`)
**[src@580]**. A matching euid alone passes. The check is **on by default** — the property
initialises to true independent of any registry key
(`ogkm-580: src/nvidia/generated/g_system_nvoc.c:103`) **[src@580]**.

⇒ **The reference class is widened.** A local unprivileged process (euid 1000) fails that check
against any root-owned RM client. Our isolate, on a root VMM, passes it — so RM's cross-user
client-handle protection, which is a real boundary between an unprivileged process and every
root GPU client on the host (`nvidia-persistenced`, a display server, another root CUDA
process), does not stand between the isolate and those clients `[inferred]`.

★ **Why P nonetheless holds, and — until 2026-08-01 — it was one line rather than an
invariant.** Every ioctl we issue stamps *our own* client into the `hClient` field. The guest
supplies object handles, never the client handle, so it cannot name a foreign client through
the op set that exists. The euid is a latent widening, not a live one.

What made that fragile was **where** the property lived: `RmConnection::raw_alloc` took a
`root: u32` parameter and all eight call sites *happened* to pass `self.client`. Nothing said
so, nothing checked it, and one future call site passing anything else would have converted a
latent widening into a live one with **no red test anywhere**.

#### ★★★ RESOLVED 2026-08-01 — follow-up (2), and it is structural rather than checked

The invariant is now one a wrong call site **cannot satisfy**. `rm.rs` has a private
`mod own_client` whose `OwnClient(u32)` has a **private field** and exactly one constructor,
`OwnClient::allocate_root`, which *performs* the `NV01_ROOT_CLIENT` allocation and wraps the
handle RM wrote back. So *"an `OwnClient` value exists"* and *"this process minted that
client"* are **one statement**, in the shape `#139`'s `StickyAnswerGuard` and `#137`'s
`WantedTable::from_cmd` both took. `raw_alloc` no longer has a client parameter at all — it
stamps `self.client`, so a caller cannot express a foreign client.

Three deliberately-wrong call sites were planted and **watched failing to compile**
`[run: 2026-08-01, branch `f11-invariant`, 38-core x86_64 build box, rustc 1.97.1,
`cargo build -p kayfabe-isolate-host`; each mutation applied, the error recorded, then
reverted and the file verified byte-identical by `sha256sum`]`:

| planted mistake | compiler says |
|---|---|
| pass a foreign client to `raw_alloc` | `error[E0061]: this method takes 4 arguments but 5 arguments were supplied` |
| `OwnClient(0xdead_beef)` | `error[E0423]: cannot initialize a tuple struct which contains private fields` |
| `client: 0xdead_beefu32` in the connection | `error[E0308]: mismatched types … expected `OwnClient`, found `u32`` |

⚠ **What is NOT structural, stated because the split is the honest part.** The ABI parameter
blocks in `kayfabe-abi` type their client fields as plain `u32`, and typing them is a
crate-wide change deliberately not made. So a **new struct literal** in `rm.rs` could still
write `h_client: 0xdead_beef` and compile. That residue is covered by a *checked* gate —
`crates/kayfabe-isolate-host/tests/own_client_invariant.rs`, three tests, whose universe of
"client-ish field names" is **read out of `kayfabe-abi` at test time** rather than listed
(`gates_quantified_over_a_list`). Five more mutations were planted and watched going red,
including two that break the instrument itself so the gate cannot pass vacuously.

⊘ **Follow-up (1) is still open and is the better fix.** Running the isolate at a **non-zero
uid** — mapping it to `nobody` rather than to the VMM's uid — makes the widening *disappear*
instead of being argued about. The map is `0 <outer_uid> 1`, so it is a change to how the
namespace is built and needs its own analysis of what else in the sandbox assumes uid 0. What
landed here bounds the consequence; it does not remove the cause.

⊘ **Still no run on the security question itself.** Whether F11's euid widening is exploitable
at all remains `[unknown]` — it needs a root-owned RM client on the box whose handle value we
could name. Nothing in this section has been in front of a real driver; the runs cited above
are a compiler and a test binary.

### F12 — ★★ A real counterexample, in the C artifact, and it is why the evidence base matters

The C research artifact keeps a **root RM client in the VMM's own process, in the host initial
namespace, outside every sandbox**, and answers a guest request from it. `nvkvm_admin_ensure`
opens `/dev/nvidiactl` and `/dev/nvidia0` there and `nvkvm_admin_get_pid_mem` issues
`NV2080_CTRL_CMD_GPU_GET_PID_INFO` on it (`C: src/qemu/nvkvm_isolate_handlers.c:668-793`)
**[src-C]**. The guest reaches it by sending `GET_PID_INFO`. The stated reason is sound — inside
`CLONE_NEWPID`/`NEWUSER` the driver attributes 0 bytes, so per-process VRAM can only be read
from the initial namespace — and the mitigation is real: the tgid queried is the validated
isolate's own, never a guest-named pid.

⇒ **It is nevertheless a privileged capability whose trigger is a guest message**, which is
precisely the shape P forbids, and it is **not ported** — nothing in this tree opens a device
node outside the isolate **[src-rust]** (F8 is the one named future exception). It is recorded
here because the C is this project's standing oracle and is where people go for evidence: **P is
a statement about the Rust port and it is false of the C artifact.** Anyone quoting P at the C
would be quoting it at the wrong system.

★★ **And this is exactly the kind of C bug the port exists to subtract.** `port_the_c_not_a_redesign`
is the standing directive — *reproduce the C and subtract its named bugs, because its bugs are
enumerable and its correct behaviour is not*. F12 is one of those named bugs: a root RM client,
outside every sandbox, whose trigger is a guest message. The port does not have it, and the
reason it does not is not luck — **nothing in this tree opens a device node outside the
isolate**, which is a structural difference and not a policy one. Recording the contrast is how
the subtraction stays visible; a later reader who finds `nvkvm_admin_ensure` in the oracle and
assumes the port inherited it would be wrong in the direction that matters.

### F13 — The host egress is default-FORWARD, and its whole policy is a two-entry `matches!`

Two controls on our **ingress** allowlist are not something RM would serve an unprivileged
caller **[src@580]**:

| control | RM flags | class |
|---|---|---|
| `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` `0x2080012b` | `0x10244` | **`PRIVILEGED`** |
| `NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_INFO` `0x20801219` | `0x8000` | **kernel-privileged** (neither `PRIVILEGED` nor `NON_PRIVILEGED` ⇒ §3.3's default) |

Admitting them at ingress does not breach P — §3.3 means RM refuses both regardless — but the
thing that keeps them off the wire is `classify_control`, whose `else` arm **forwards**
(`crates/kayfabe-fwd/src/lib.rs:1925-1933`), and whose entire diverting set is
`matches!(cmd, PROMOTE_CTX | GET_CTX_BUFFER_INFO)` in `kayfabe-mocks`
(`crates/kayfabe-mocks/src/lib.rs:208-210`), delegated to by every real `Arch`
(`kayfabe-chips/src/ad10x.rs:319`, `gh100.rs:360`, `kayfabe-crec/src/ga10x.rs:76`)
**[src-rust]**. The repo already records the egress as default-forward and open
(`eight_blockers_resolved.md` §"Still open").

⇒ ★ The point for P is narrow and worth stating exactly: **the Case-2 split is a correctness
mechanism, not a security one.** It exists so an unprivileged replay does not return
`InsufficientPermissions`; it is not what makes P true, and it should not be cited as though it
were. What makes P true is that RM refuses.

### F14 — ★★★ The fd crossing hands a GPU descriptor to a ROOT process, and "a descriptor confers exactly the access the opener had" is FALSE for RM

`82945eb` (task #131) added an `SCM_RIGHTS` crossing in both directions, so the VMM can now hold
a GPU descriptor the isolate opened. That note is admirably direct about the cost — it says the
`#96` property *"is **weakened**"*, that the VMM may `ioctl` the descriptor and *"nothing
structurally prevents this"*, and it keeps only the narrower property that the process which
**opens** the device is unprivileged (`isolate_vmm_fd_crossing.md` §2). All of that stands.

⚠ **One sentence in the same section does not, and it is the one that bounds the damage.** §2's
"what it cannot do" list ends with *"it cannot escalate — a descriptor confers exactly the access
the opener had."* For an ordinary file that is how Linux works: permission is checked at `open`
and the descriptor carries it. **RM does not work that way.** `secInfo.privLevel` is computed
from `osIsAdministrator()` at the top of **every** `RmIoctl`, it is the **only** place in
`escape.c` privilege is assigned (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:304`,
sole occurrence), and `nv_file_private_t` carries no privilege field at all
(`ogkm-580: kernel-open/common/inc/nv-linux.h`) **[src@580]**.

⇒ **A descriptor confers the access the CALLER HAS AT IOCTL TIME, not the access the opener
had** `[inferred]`. On a root VMM that is `RS_PRIV_LEVEL_USER_ROOT` — so the same descriptor
that yields 768 non-privileged controls in the isolate yields all **265 `PRIVILEGED`** ones in
the VMM (F1). §3.2's mechanism is symmetric, and this is its other edge: the thing that makes
our isolate safely unprivileged is exactly the thing that makes the VMM privileged on the very
same fd.

★ **Not reachable today, and the reason is that the consumer is unbuilt.** `CrossedFd` has no
call site outside its own module, its re-export, and the crossing's tests **[src-rust]**; the
`#128` passthrough the crossing exists for is explicitly not built. So this is a **latent**
widening, like F11 was — but the comparison no longer runs all the way. ★ **F11's half of it
was closed on 2026-08-01** (see F11's ★★★ RESOLVED block): *"`hClient` is never guest-derived"*
is now a type a wrong call site cannot satisfy. This one is still held by nothing but the
absence of a caller, so it is now the **weaker** of the two and the one to reach for next.

### ★★★ F14 — SUPERSEDED IN PART, 2026-07-31. Read §1.1 first.

⚠ **This finding's mechanism stands; its consequence (2) below was wrong and is withdrawn.**
Both halves are kept in place rather than rewritten away, because a finding that is edited
into agreement with a later decision teaches nothing, and this one's *reason* is now the
argument for the design.

**What stands.** Everything above the line: RM computes `secInfo.privLevel` from
`osIsAdministrator()` at the top of every escape, `nv_file_private_t` carries no privilege
field, and therefore a descriptor confers the access **the caller has at ioctl time**. The
"a descriptor confers exactly the access the opener had" clause in
`isolate_vmm_fd_crossing.md` §2 is **false for RM** and is corrected there.

**What is withdrawn.** The original text read: *"(2) That mitigation is now load-bearing for
**P**, not only for `#96`."* It is **not**. Under §1.1's ruling a compromised VMM is the
boundary rather than a step inside it, so reaching those 265 privileged controls requires
code inside the VMM issuing an escape — which a behaving VMM does not do and which a guest
cannot cause. P is stated relative to a behaving VMM and is not violated here. ⊘ The
`seccomp`-on-the-VMM mitigation is not owed, and sandboxing the VMM is explicitly not this
project's job.

**What replaced it.** The owner took option **(b)** for `#133`: an isolate verb performs the
mapping and hands the VMM **memory** (a sealed `memfd`) instead of a descriptor —
`RmBackend::export_backing`, `isolate_vmm_fd_crossing.md` §12. The reason is stated as
hygiene and contract, *"the descriptor simply should not be somewhere we do not control"*,
**not** as closing F14. F14 is why the design is worth having.

★ **And it does not cover everything**, which is a first-class result rather than a caveat:
(b) is complete for memfd-backed regions and **incomplete for real device MMIO**. See §12 of
that note for which mappings fall in each class, and for the two independent citations that
make the incompleteness a measurement rather than an omission.

★ One clause of the original consequence (3) stands unchanged and is worth keeping: the
note's closing asymmetry — *"the VMM is the more trusted side… a GPU descriptor adds to a
process that could already do more damage"* — is true about the **host** and does not
transfer to P, because P compares against a local *unprivileged* process.

### Findings summary

**No live counterexample to P was found in this tree, and two near-misses were.** Ranked by what
it would cost to be wrong:

1. **F14** — RM assigns privilege **per ioctl from the caller**, so "a descriptor confers
   exactly the access the opener had" is false here. ★ **Its consequence for P was withdrawn
   on 2026-07-31** (§1.1, §4 F14): a compromised VMM is the boundary, not a step inside it,
   so the `seccomp`-on-the-VMM mitigation is not load-bearing for P and is not owed. The
   mechanism stands and is now the standing argument for `#133`'s decision (b) — the
   descriptor no longer crosses at all for the classes (b) covers.
2. **F11** — the isolate's init-namespace euid is the VMM's, RM keys a real check on euid, and
   P holds only because `hClient` is never guest-derived. The one finding here that is not
   about capabilities at all. ★ **The `hClient` half is CLOSED as of 2026-08-01** — it is a
   private-field newtype whose only constructor is the root-client allocation, so a foreign
   client is `error[E0423]`/`error[E0061]` rather than a red test. The **cause** is untouched:
   the isolate still runs at the VMM's uid, and follow-up (1) — map it to `nobody` — remains
   open and is the better fix.
3. **F12** — the C artifact *does* violate P, through a root RM client reachable by a guest
   message. Not ported; recorded because the C is where the evidence base lives.
4. **F2/F7** — RM caches a per-client privilege level, and the ordering that stops us latching
   an admin one is call order in one function rather than a type.
5. **F13** — two controls RM treats as privileged/kernel-only sit on our ingress allowlist; the
   egress that would carry them is default-forward with a two-entry mock policy. ⊘ The
   Case-1/Case-2 split this describes is a **correctness** mechanism; §3.4 records that it must
   not be cited as security, and a tree-wide sweep on 2026-08-01 found **no** site doing so.

F5, F6 and F10 each looked like a way to exceed the bar and each turns out to be inside it, for
the same reason every time — §3.3 makes the host driver, not us, the adjudicator. F8 is the
honest exception: a seam where P is not yet established because the code is not written.

---

## 5. The wedge exposure, stated plainly

**A hostile guest can hang a host GPU engine, and nothing in our construction prevents it.**
The guest reaches a real channel through a real doorbell store (`rm.rs:1744-1755`)
**[src-rust]**, which is what makes the compute real; a non-terminating kernel or a malformed
pushbuffer submitted through it is a submission that never returns, and every lever we hold acts
before the ring or after the completion (`compute_limiting_and_priority.md` §6.3).

What follows:

- **Single-tenant: accepted for v1.** The tenant denies itself its own GPU. That is the
  posture this document records, and it is a decision rather than an oversight.
- **Multi-tenant: this is the exposure** — ★ but read §5.1 before quoting this bullet: the
  *compute-hang* half of it was measured on 2026-08-01 and **does not hold**. Per-VM pacing does not close it — a rate limiter
  makes an honest tenant fair and does nothing to a hostile one, which needs to get exactly one
  pushbuffer through (`compute_limiting_and_priority.md` §6.3). The two mitigations that would
  actually bear on it are **one guest per physical GPU** (partitioning the blast radius rather
  than bounding it) and **preempt-or-reset**, whose only unprivileged candidate lever is
  `NVA06C_CTRL_CMD_PREEMPT` on our own TSG — and whether a preempt lands on a spinning kernel is
  `[unknown]` from the open tree (that note §3.3, §6.3).
- ⊘ **Do not describe the wedge as a violation of P.** It is inside it, and conflating "P holds"
  with "the GPU is safe from denial of service" is the specific misreading this document is
  written to prevent.

---

## ★★★ 5.1 MEASURED 2026-08-01 — an infinite kernel does NOT deny service, and §5's first shape is REFUTED

§5 above names two shapes: *"a non-terminating kernel **or** a malformed pushbuffer"*. **The
first one has now been run on real hardware, and it does not do what §5 implies.**

`[run: scripts/bench/gpu_wedge_containment.sh, 2026-08-01T21:48Z, vast 46529600, RTX 3060
GA106, host driver 580.159.04 open, kernel 6.8.0-59, repo at 36f746a; full log
docs/reference/bench_evidence/wedge-containment-36f746a-ga106.log]`

An attacker process launches a kernel that spins on a global flag that is never written, at
**224 blocks × 1024 threads = 229 376 threads** against GA106's ~43 008 resident capacity, so
the device is genuinely oversubscribed and the scheduler must juggle it. A second, independent
process then does real, verified work.

| arm | result |
|---|---|
| liveness, trivial victim | **3/3** then **12/12** over 60 s, every one `rc=0` |
| correctness under the wedge | `bad=0` on every run — never once wrong |
| fairness, victim doing real GPU work | 2.66 s idle → **5.57 s** under the wedge (**~2.1×**), still correct |
| `nvidia-smi` during the wedge | responsive |
| **Xid** | **0** — before, during, and after |
| aftermath: kill the attacker | util → 4 %, no compute apps, no residue, victim back to 0.1 ms. **No GPU reset needed** |

⇒ **On this hardware a hostile tenant cannot deny the GPU to another tenant by hanging a compute
kernel.** It costs its neighbour roughly a factor of two in throughput — which is a *fairness*
problem, and the honest name for it — not a liveness or correctness one.

★★ **The run above establishes the OUTCOME, not the MECHANISM.** Ampere-class compute preemption
and context time-slicing are the obvious explanation and are almost certainly right, but the
`gpu_wedge_containment.sh` run did not instrument preemption — it observed only that the victim
survived. Why it survived is `[inferred]`, and nothing here should be cited for it.

### ⊘ What this does NOT establish — and the list matters more than the result

1. **The malformed-pushbuffer shape — §5's other half — is untested.** It is the one that can
   actually *fault*, and therefore the only one that can reach the escalation hazards in §7. A
   hang and a fault are different events and this experiment produced only the former.
2. **The escalation path was never exercised, because no recovery ever fired.** Zero Xid in 60+ s
   means the RC watchdog did not trip at all on a headless compute context. ⊘ Do not read "the
   wedge was contained" as "recovery is contained" — the second was not tested, and this is
   exactly the [[a_boolean_witness_cannot_attribute]] shape: an absence of Xid is compatible with
   both "recovery is fine" and "recovery was never asked". (A boolean witness cannot attribute —
   the same trap that made E0's first isolate witness read as a causal claim; see
   `execution_plane_increments.md` §3.5.)
3. **VRAM exhaustion is a separate and untested vector**, and it needs no preemption story at all
   — a tenant that allocates the framebuffer denies it by ordinary means.
4. Untested: multiple simultaneous wedgers, graphics/display contexts, MMU-fault storms, and any
   non-GA10x part. `compute_mode` was `Default` throughout; `EXCLUSIVE_PROCESS` would change the
   question by forbidding the second tenant outright.

## ★★★ 5.2 MEASURED 2026-08-01 — a FAULT is contained too, and this time the escalation path WAS entered

§5.1 closed with the honest limit that **zero Xid means recovery was never *asked*, not that
recovery is contained**. That gap is now closed for the fault shape.

`[run: scripts/bench/gpu_fault_containment.sh, 2026-08-01T23:34Z, vast 46529600, RTX 3060
GA106, host 580.159.04 open, repo eea787f; full log
docs/reference/bench_evidence/fault-containment-eea787f-ga106.log]`

An attacker stores through a wild, unmapped device VA. RM logs a genuine fault:

```
NVRM: Xid (PCI:0000:00:08): 31, pid=…, name=gpu_wedge_probe, channel 0x00000008,
  MMU Fault: ENGINE GRAPHICS GPC1 GPCCLIENT_T1_0 faulted @ 0x7000_00000000.
  Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_WRITE
```

| arm | result |
|---|---|
| the attacker's own context | `CUDA_ERROR_ILLEGAL_ADDRESS`, **sticky** — dead for reuse, as CUDA specifies |
| ★ **victim holding a LIVE context across the fault** | **2 682 576 iterations, 2 682 576 correct, 0 wrong, 0 errors** |
| a fresh victim afterwards | `OK bad=0` |
| the GPU afterwards | util 3 %, **no reset, no "reboot required", no "fell off the bus"** |

⇒ **On this hardware a guest-reachable compute fault is contained to the offending channel.**
The Xid fired, RM's recovery ran, and a second tenant's *live* context did not lose a single
iteration.

★ **This is stronger evidence than §5.1**, because the escalation path was actually exercised
rather than merely not needed. §7's three hazards — whole-runlist preempt, node-level
reboot-required latch, GSP-death halting every channel — did **not** materialise for this fault
class.

★★ **The arm that carries it is the live-context one, and the weak arm is kept visible to show
why.** A victim started *after* the fault gets a *fresh* context and would survive a
context-scoped kill without noticing — it is in the log as arm B2 precisely so nobody mistakes
it for the finding. Same trap as the 1×32 occupancy error below: an instrument that could not
have shown otherwise.

⊘ **Still not established, and the list is shorter but not empty:** this is an **MMU fault from
a wild store, NOT a malformed pushbuffer** — the pushbuffer here was well-formed and RM built
it. What it shares with §5's second shape is that it reaches the fault/Xid path at all. A
submission malformed at the *pushbuffer* level (bad method headers, a bogus GPFIFO entry) is a
different input to a different parser and remains untested. VRAM exhaustion, multiple
simultaneous faulters, and non-GA10x parts also remain untested.

## ★★★ 5.3 MEASURED 2026-08-02 — the THIRD shape is NOT contained, and it is the crude one

§5.1 and §5.2 both came back the same way: a second tenant kept full liveness and correctness.
Both of those rest on **the GPU's scheduler**. This one rests on nothing — it is ordinary
allocation, and Q19 named it as the likely easier vector while leaving it untested.

`[run: scripts/bench/gpu_vram_denial.sh, 2026-08-02T00:16Z, vast 46529600, RTX 3060 GA106
(12 288 MiB), host 580.159.04 open, repo b628df4; full log
docs/reference/bench_evidence/vram-denial-b628df4-ga106.log]`

| arm | result |
|---|---|
| an **unprivileged** hog allocating 256 MiB chunks until refused | **11 776 MiB in 46 chunks — 95.8 % of the board**; free drops to **20 MiB** |
| ★ a second tenant, twice | **cannot even `cuCtxCreate`** — `CUDA_ERROR_OUT_OF_MEMORY`, in **0.29 s** |
| aftermath: hog exits | used → **0 MiB**, victim `OK bad=0`, **no Xid, no reset** |

⇒ **Denial is total.** Not "the victim's allocation fails" — **the victim cannot get onto the
device at all**, which is strictly worse, because a tenant that cannot create a context cannot
even begin to degrade gracefully.

★ **But it is honest denial, and that distinction is the actionable one.** The refusal is
**immediate (0.29 s), named (`CUDA_ERROR_OUT_OF_MEMORY`), and fully reversible** — the moment
the hog exits the board is clean and the next tenant works, with no Xid and no GPU reset. That
is the opposite of the wedge fear this document was written around: it is a *resource* problem,
not a *recovery* problem.

⇒ **The mitigation is therefore ordinary and buildable: a per-tenant VRAM quota**, enforced
where we already broker allocation. Contrast §5's wedge, whose only real mitigations were one
guest per physical GPU or a privileged reset we do not own. ★ This is the one multi-tenant
exposure so far that is squarely **inside** our reach.

⊘ Untested here: many small allocations rather than few large (fragmentation may deny at a
lower total), a hog that grows to fill whatever a victim releases, and whether host-side
`cuMemAllocManaged`/UVM changes the picture.

### ★ The instrument was wrong first, and it was wrong in the flattering direction

The first version of this experiment span **1 block × 32 threads** on a 28-SM GPU. The victim
survived — but it survived because **27 SMs were free**, so no preemption was ever involved and
the result was guaranteed before the experiment ran. It reported total containment and **could
not have shown otherwise**. `nvidia-smi utilization.gpu` reads **100 %** in both the 1×32 and the
224×1024 case, so the one number that looked like a saturation check could not discriminate
either. The finding only became real once the wedge oversubscribed the device.
⇒ Recorded in the script's own header so the next person cannot re-run the weak version by
accident: suspect the instrument before believing a result that flatters you.

---

## 6. ★ Recovery — what task #130 established, and what it did not

The device-recovery cycle landed at `[run: commit c97b640, 2026-07-31, 38-core box
70.30.221.109, 1 329 passed / 0 failed workspace `--no-fail-fast`]`, with
`crates/kayfabe-qemu-raw/tests/device_recycle.rs` driving realize → unrealize → realize twice on
one machine and finding one real reference leak in the process.

⊘ **That is not wedge recovery, and it must not be read as any part of it.** Three limits, each
recorded by that work itself rather than inferred here:

1. It is a cycle over **our emulated device model** — the register plane, the GSP state machine,
   the memory-region references. No host GPU is involved in it at any point.
2. **Isolate lifetime is not observable at that seam and is not covered**, stated in that
   commit's own report and in the test module's docs.
3. `RegPlane::device_reset` is explicitly **not** a reload: counters and the unclaimed-offset
   sample survive it, and that residue is diagnostic-only.

⇒ A guest-wedged *host* engine is not recovered by unrealize → realize. Recovering it needs a
GPU-level reset, which is privileged and belongs to the host operator — the same place the RC
watchdog timeout lives (`compute_limiting_and_priority.md` §3.3). The recovery story for a wedge
is therefore **out of our hands by the same construction that makes P true**, which is the
uncomfortable symmetry worth writing down: we cannot brick it *and* we cannot un-wedge it.

---

## 7. ★ What I could not determine

Stated as gaps, not padded into guesses.

- **Whether a wedge's blast radius stops at the offending channel.** ★★★ **PARTIALLY ANSWERED
  BY EXPERIMENT, 2026-08-01 — see §5.1.** The *non-terminating compute kernel* shape does **not**
  deny service to another tenant, measured on real hardware. What is still `[unknown]` is
  narrower and is now stated as such in §5.1: the escalation path below was never exercised,
  **because nothing made it fire**. The open tree notifies at channel-or-TSG scope and stubs out
  "recover all channels", but the reset decision is GSP's, and three escalation hazards are
  visible in it — a whole-runlist preempt inside the per-channel halt, a node-level "reboot
  required" latch on a UVM-owned channel, and a GSP-death path that halts every channel GPU-wide
  (`compute_limiting_and_priority.md` §3.3, `[src@580]` there). Reaching any of them needs a
  **fault**, not a hang.
- **What the host driver does with a bit-15 command absent from its tables** (F6). Not readable:
  the space is GSP-serviced and GSP is a signed binary.
- **Whether the proprietary driver implements `NV0000_CTRL_CMD_PUSH_UCODE_IMAGE`** (`0x285`).
  It is `NON_PRIVILEGED` with `accessRight=0x0`, and in `ogkm-580` its body is a flat
  `return NV_ERR_NOT_SUPPORTED` (`ogkm-580: src/nvidia/src/kernel/rmapi/client_resource.c:5548-5555`,
  documented VMware-only at `ctrl0000gpu.h:931-944`) **[src@580]**. It is inert in the open tree
  and it is the one *name* in the non-privileged set that reads brick-class, so it is recorded
  rather than dismissed. Both drivers must work, so an inert body here is not an answer for
  there.
- **Whether F11's euid widening is exploitable at all.** It needs a root-owned RM client on the
  host whose `hClient` value an attacker can name, and RM's handle namespace is global rather
  than fd-scoped. Whether such a handle is guessable in practice, and whether anything besides
  `_rmclientUserClientSecurityCheck` stands in the way, was **not established** — and no run
  exists on either side of it.
- **Whether P survives on a host whose VMM is not root.** Everything in §3.1 was reasoned about
  the rootless arm of `acquire_mount_namespace`, which is tried first by design
  (`sandbox_unsafe.rs:586-604`), but the privileged fallback arm at `:624` leaves the child in
  the parent's user namespace. The capability argument is unchanged; F11 gets *better* (a
  non-root VMM maps to a non-root isolate) and the `ptrace` reasoning in that module's docs gets
  worse. **Not analysed here.**
- **Where the bit-15 / GSS-legacy rule was adjudicated.** The brief cites it as "task #107";
  **no artifact under that label exists in either repo** — `#107` in the C artifact is the EGL
  present path. The adjudication itself is real and is at
  `crates/kayfabe-abi/src/capability.rs:41-48` with its pinning test at `:2071`, plus the C's
  `C: docs/audits/nvproxy_control_allowlist.md:16-20,493-495`. Recorded so the next reader
  does not search for the label.
- ⊘ **Nothing in this document was reproduced on hardware by me.** The two runs it leans on are
  other people's and are named as theirs in §3.1 and §6.

---

## 8. Where this is cross-referenced

- `core_security_threat_model.md` — the core's logical isolation invariants I1–I4. That document
  is scoped to the pure logic core and defers host-breakout surfaces; **P is the system-level
  companion to it** and is pointed at from its §1.
- `compute_limiting_and_priority.md` §1 — opens by citing this document for the brick/wedge
  split. F3 corrects one mechanism in its §4.1 and F4 confirms its §4.2 count.
- `l1_os_shell.md` — the OS seams where the sandbox and the isolate lifecycle live.
