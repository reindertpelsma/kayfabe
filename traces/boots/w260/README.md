# w260 — the FB-leaf JOIN, BOOTED on a real GA106. Every prediction scored.

**STATUS: MEASURED 2026-08-11. LIVE.**

`[measured 2026-08-11T16:11–16:24+00:00, bench `vh`, RTX 3060 GA106, host driver 580.159.04,
rev `62ab8755245b3c320de8365b08e3da4f1031292a` — asserted from the binary's own
`kayfabe-rev:` stamp, gated BEFORE the first boot was allowed to run, and re-read out of each
arm's probe log. Three boots, ONE binary, `KAYFABE_ISOLATES=real KAYFABE_CE_EXECUTOR=host
NVKVM_RAM_BACKEND=memfd KAYFABE_GUEST_RAM=memfd`, `POST_CAPTURE_HOOK=cup2_hook_w232.sh`.
All three `boot_capture.sh RC=0`; driver terminator `=== W260 EXIT rc=0 ===` present.]`

Pre-registration: `predictions_recorded_before_the_boot.md`, committed at `62ab875` **before**
the bundle was built. R32 evidence: `traces/real_ga106/rmladder_r32_fb_memfd_join*.txt`.

---

## ★★★ 0. LEAD: THREE OF THE BRIEF'S FOUR UNKNOWNS WERE NOT ANSWERED, AND COULD NOT BE

This was **pre-registered as P10**, not discovered afterwards. Across all three arms:

| failure-path line | off | shared | private |
|---|---|---|---|
| `RELEASED and NOT bound` | 0 | 0 | 0 |
| `THE INSTALL REFUSED` | 0 | 0 | 0 |
| `BIND REFUSED` | 0 | 0 | 0 |
| `COULD NOT CLAIM IT` | 0 | 0 | 0 |
| `REFUSED BY NAME` | 0 | 0 | 0 |

⇒ **Unknown #1 (install→bind ordering), #2 (the release path), #3 (the attempt-once `refused`
set) are UNMEASURED.** Not "passed" — **unmeasured**, which is a different verdict and is the
`dlen=0` lesson applied to my own predictions. The reasons are structural, and a *second*
green boot would not help:

- **#1** is observable **only on a failing install.** With the install succeeding, "bind before
  install" and "bind after install" have **identical end states**. The boot cannot discriminate
  the very thing the port changed.
- **#2** runs on failure paths only. Nothing failed, so `release_unadopted_fb_leaf` was never
  called. The `0x51`-avoidance argument remains **argued from staging semantics, exactly as the
  brief said** — the boot did not upgrade it.
- **#3** needs **two census operands in ONE leaf**. Measured: three FB operands in **three
  distinct leaves** (P3, below). The `refused` set is never consulted a second time, so the
  attempt-once behaviour has no occasion to occur.

⊘ These need **fault injection** or a workload whose census puts two operands in one leaf.
They are not boot-shaped questions.

## ★★ 0.1 A SECOND REFUTATION, and this one is mine: THE ESTABLISHMENT COPY IS UNEXERCISED

`established=0 bytes over 0 page(s), of which 0 NON-ZERO` on **all three leaves, in both armed
arms**, with **3** `⊘ the establishment copy was VACUOUS` notes each. The code's own line says
it: *"That is CORRECT … and it is NOT evidence that the copy works."*

★ This matters more than it looks, because the establishment copy is **the load-bearing half of
the ordering-safety argument**. `fb_join.md` §1.2 justifies the whole install-then-bind shape
with *"the establishment copy removes the question rather than answering it: after it there is
one memory, so there is never a merge."* **That copy has now run three times and moved zero
bytes.** The argument is still sound by construction; it is simply **not yet witnessed**, and
nothing in this boot distinguishes a working copy from a no-op one.

## ★★ 0.2 AN INSTRUMENT DEFECT: the JOINED line says "ONE memory" IN THE NEGATIVE CONTROL

Verbatim, from `run_w260_private_qemu.log`:

> `… → JOINED (private) memory=0xcafe005e host_va=0x200000000 placed_as_asked=true … — ★ ONE
> memory: the pages the host GR engine walks to are the pages this device's framebuffer window
> now reads and writes.`

⊘ In the `private` arm that sentence is **false by construction** — the arm exists precisely to
make the two views *different* memory, and the probe two lines later proves they are. The only
token on that line that separates the arms is `(private)` vs `(shared)`. A reader grepping
`"ONE memory"` gets **3 hits in the negative control**.
⇒ Same family as *"a green test can hold a wall in place"*: the discriminating text must vary
with the thing it discriminates. **Fix: make the clause conditional on `self.fb_join`.** ⊘ Not
fixed in this rung — recorded, because changing the binary after the measurement would forfeit
the stamp gate.

---

## 1. SCORING — every prediction, including the failures

| # | prediction | verdict | evidence |
|---|---|---|---|
| **P0** | binary stamp == `git rev-parse HEAD`, no `-dirty`, one value | ✅ **PASS** | gate ran **before** boot 1; `kayfabe-rev:62ab875…` re-read from all 3 probe logs |
| **P1** | one `FB-JOIN arm=<arm> exports_directory=true` per arm, matching the tag | ✅ **PASS** | off/shared/private each print their own arm; `exports_directory=true` on all three |
| **P2** | census `operands=5 bound=4 unbound=1 mme_dwords=39`, unchanged | ✅ **PASS** | **8 channels × 3 arms = 24 census lines, all identical**. Backing does not move binding |
| **P3** | 3 FB operands in 3 **distinct** leaves, at the named VAs | ✅ **PASS** | `SET_VALID_SPAN_OVERFLOW_AREA` `0x200000000`/`0x400000`; `SET_TEX_SAMPLER_POOL` `0x10002000000`/`0x800000`; `SET_TEX_HEADER_POOL` `0x10000000000`/`0x600000` — exactly `w228`'s |
| **P4** | ★ 3 × `JOINED … placed_as_asked=true` on doorbell 1 | ✅ **PASS** | `count JOINED = 3`, `ALREADY JOINED = 21`, `placed_as_asked=false` **= 0** |
| **P5** | probe runs **exactly once**; 7 × `NO PROBE`; no `PROBE MISS` | ✅ **PASS** | `NO PROBE = 7`, `PROBE MISS = 0`, `PROBE ABORTED = 0` (⚠ see §1.1) |
| **P6** | `shared`: both directions AGREE over 1024 words | ✅ **PASS** | `DIRECTION 1 … AGREES over 1024 words`; `DIRECTION 2 … AGREES over 1024 words` |
| **P7** | ★★ `private`: both disagree at **word 0**, with named values | ✅ **PASS, EXACTLY** | D1 `got 0x00000000, want 0x5a1a5a5b`; D2 `got 0x5a1a5a5b, want 0xa5e5a5a4` — **all four constants as pre-registered** |
| **P8** | establishment copy vacuous, `nonzero=0` | ✅ **PASS** (⚠ weak direction — see §0.1) | `established=0 … 0 NON-ZERO` ×3, `VACUOUS notes = 3` |
| **P9** | ★ execution/completion plane moves by **exactly zero** | ✅ **PASS** | see §2 |
| **P10** | ★★★ zero failure-path lines ⇒ #1/#2/#3 unanswered | ✅ **PASS** | see §0 |
| **P11** | R32 passes, negative fails | ✅ **PASS** | J1 + J2 both 65536/65536; negative fired at word 0 |

**12 predictions, 12 held.** ⚠ That is *not* a good sign in itself and is worth naming: a
pre-registration in which nothing is refuted is one that was not reaching. The two things I
got *wrong* are not in the table — they are §0.1 (I predicted the copy would be vacuous and
scored it PASS, without pre-registering that a vacuous copy leaves the ordering argument
unwitnessed) and §1.1.

### 1.1 ⊘ MY SCORING INSTRUMENT OVERCOUNTED — caught, and reported rather than quietly fixed

`DIRECTION 1 lines = 2` in the `private` arm. **The probe still ran once.** My grep counted a
second line that merely *contains* the substring:

> `★★ AND THE VALUE READ BACK IS DIRECTION 1'S OWN PATTERN, not zeros …`

⊘ A substring count is not an event count. The same script reports `DIRECTION 1 lines = 1` for
`shared`, where that bonus line does not fire — so **the instrument's answer changed with the
arm for a reason that has nothing to do with the arm.** Suspect the instrument first.

---

## 2. ★ THE SIZE OF THE JUMP — `cup2` moved by ZERO steps, not one, not several

Pre-registered as P9, with the reason recorded **before** the log existed so it could not be a
rationalisation.

| | off | shared | private | `w251`/`w256` |
|---|---|---|---|---|
| `CE-SUBMIT` | 0 | 0 | 0 | 0 |
| `COMPLETION-WATCH → NOT-OBSERVED` | 8 | 8 | 8 | 8 |
| `samples=` per channel | 88,87,86,85,84,83,82,81 | **identical** | **identical** | identical |
| `NotACopyEngineChannel` | 9 | 9 | 9 | 9 |
| `CUP2_RC` | **124** | **124** | **124** | 124 |

★ **Stated plainly, as asked: `cup2` did not advance a single rung.** `CUP2_RC=124` is the
standing 180 s wall at `cuCtxCreate`, identical on all three arms.

⊘ **And I hold that this is not evidence against task #231**, for the reason pre-registered:
#231 quantifies over changes that touch the **execution** path, and this branch adds **no route
to an engine at all** — `Route::NotACopyEngineChannel` refuses every `GrCompute` doorbell
upstream of everything here, 9 times per boot, untouched by the diff. A supply-side
precondition landing with zero execution movement is the **predicted** shape.
⚠ But the honest form of that claim is a **standing debt, not a discharge**: the passthrough
model now owes a boot in which the doorbell *is* routed. Until then "supply is ready" is a
claim about our own code, not about the guest. If the first routed boot **also** moves things
by one step, #231's premise is the thing to doubt.

---

## 3. VERDICT ON THE FOUR UNKNOWNS

| # | unknown | verdict |
|---|---|---|
| **1** | install→bind ordering | ⊘ **UNMEASURED.** Structurally unobservable on a green boot. Needs fault injection |
| **2** | `release_unadopted_fb_leaf`, `0x51` avoidance | ⊘ **UNMEASURED.** Never called. Covered offline only (`tests/tests/fb_leaf_backing.rs:816,855`) |
| **3** | the attempt-once `refused` set | ⊘ **UNMEASURED.** Needs two census operands in one leaf; the workload has three operands in three leaves |
| **4** | ★ does the ported chain reach `JOINED … placed_as_asked=true` | ✅ **YES, MEASURED.** 3 leaves, `placed_as_asked=true` on all, both directions agreeing over 1024 words, with a negative control that fired at word 0 on named constants |

★ **Unknown #4 is answered in the strong form.** The `8eb8dcd` result the brief called *"cited,
not reproduced"* is now reproduced **on the inverted ordering the port introduced**, on a stock
guest, on a real GA106 — and, unlike `8eb8dcd`, with a **negative control in the same binary**.

## 4. Evidence guard — asserted per arm, not assumed

| arm | dmesg | NVRM | `RmInitAdapter` failures | `SMI_RC` | qemu log | host dmesg delta |
|---|---|---|---|---|---|---|
| off | 5379 B | 31 | **0** | 0 | 177 356 B | 0 lines (921→921) |
| shared | 5379 B | 31 | **0** | 0 | 194 221 B | 0 lines |
| private | 5379 B | 31 | **0** | 0 | 194 468 B | 0 lines |

- ⚠ The identical **5379 B** is not a copied file: **md5s differ** (`a26808e1…`, `211b0015…`,
  `6da59998…`). Checked, because three identical sizes is exactly what a stale-artefact bug
  looks like.
- ★ **`0 adapter` means the adapter SUCCEEDED**, and that reading is asserted mechanically, not
  inferred: `boot_capture.sh:252` **dies** if `n_adapter == 0` *and* `nvidia-smi` failed. It did
  not die, and `SMI_RC=0` is in all three probe logs ⇒ the adapter was exercised and emitted no
  `RmInitAdapter` **failure** line.
- ⊘ **`run_w260_*_hostdmesg.log` is 0 bytes on all three arms, and that is a RESULT.**
  `boot_capture.sh:299` deliberately does not assert it non-empty; the count is stated instead
  (`HOST_DMESG_LINES=0`, watermark `921 → 921`). ⇒ **the host driver emitted nothing at all
  during these boots** — RM accepted all six joins without a single diagnostic, in a campaign
  where CE-channel attempts have produced 241 unread `kfifoRunlistSetId` complaints.

## 5. Is `fb-join-port` safe to fast-forward onto `master`?

**Yes**, with the caveats above recorded rather than resolved. `master` is an ancestor of this
branch, so it is a true fast-forward. The supply side reaches `JOINED` on hardware; the
negative control proves the probe can fail; the mechanism underneath it (R32/J1/J2) is now
measured rather than argued. ⊘ What ships **unwitnessed** is the establishment copy (§0.1) and
all three failure paths (§0) — none of which is a regression, since none of them has ever been
witnessed on any branch.
