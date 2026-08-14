# Class/control consistency — and the one live disagreement, which is an OWNER call

**STATUS: LIVE, 2026-08-14 (w295), measured at `f0c9fdc2` (base `c42e6678`).** The mechanism has
landed. The **case** — whether `GT200_DEBUGGER` should be denied at all — is **open and reserved
to the owner**; §5 states it with the boundary named.

> ### ⊘⊘⊘ CORRECTED BY THE BOOT THAT VERIFIED IT — **THE REFUSAL HAS NO NAME ON THE WIRE**
>
> This document, and the commit that carried it, claimed the id is now refused *"by name, as
> `ControlNotPermitted::Refused`, carrying `GT200_DEBUGGER`'s own reason."* **That was written
> from the table and not from a boot, and the boot refutes it.**
>
> `[measured, boot w295cup2, rev 940c0648 ≡ f0c9fdc2, run_w295cup2_qemu.log]`
> ```
> unserviced fn 76 cmd 0x83de0309                       ← where it actually lands
> bridge refusals: 20 total, 6 distinct                 ← UNCHANGED from w294
> grep -c ControlNotPermitted run_w295cup2_qemu.log → 0 ← the name never appears
> distinct unserviced ids: 40 (w294) → 41 (w295)        ← the +1 IS this id
> ```
>
> ⇒ **The capability table and the reply chain are different planes.** `CapabilityTable::control`
> is consulted by `translate_control` (`kayfabe-rmrpc/src/lib.rs:1547`) on the **bridge/graph**
> plane; the **reply** plane is `served_policy`'s seat chain, and a control no seat claims falls
> to the `UnservicedLedger` without the capability answer ever becoming the wire's answer. So
> retracting the row from `OBJECT_CONTROLS` is what moved the id, and the class gate is what
> makes the *table* consistent — they are not the same act, and only the first is visible in a
> boot.
>
> ★ This is `admitted_is_served`'s own lesson — *"ADMITTED and SERVED are different gates"* —
> running in the **refusal** direction, which nothing in this tree had stated. The net wire
> effect is the honest one and is exactly the pre-w292 state: **`0x83de0309` → `0x56`, via the
> unserviced ledger.** ⊘ Naming that refusal is real remaining work and is **not** done here.

> ### ⊘⊘⊘ LEAD WITH THIS — THE BRIEF'S PREMISE IS BACKWARDS, AND THE FIX FOLLOWS THE CORRECTION
>
> The rung was briefed as: *"We allow the alloc, allow a control, and refuse the free."*
> **Measured, we do the opposite on the first term: we REFUSE the alloc.** The *guest's kernel*
> allows it — it builds the object locally and returns `NV_OK` to userspace regardless of our
> refusal. So the triple is **REFUSE / SERVE / REFUSE**, and the odd row is the **CONTROL**,
> not the free.
>
> That inverts the repair. Refusing the free is the *correct consequence* of refusing the
> alloc (§4). Serving a control on an object we refused to create is answering `NV_OK` for
> state nothing on our side holds — an echo, not a service.

---

## 1. The triple, measured — four facts, at three boundaries

| step | our device says | the guest's userspace sees | where |
|---|---|---|---|
| `RM_ALLOC hClass=0x83de` | ★ **REFUSE** — `AllocClassNotPermitted::Refused … id=0x000083de` | **`NV_OK`** | `traces/w294_cudalimit/run_w294cup2_qemu.log`; `w294nvd_ce_r1.jsonl.zst` **i=366** |
| `RM_CONTROL 0x83de0309` | ⊘ **SERVE** — `control 0x83de0309 result 0x00000000 x1` | `NV_OK` | same qemu log; `serve_r1` i=390 |
| `RM_FREE` of that object | **REFUSE** — `RmGraphError::FreeUnknown` | **`0x56`** | `w294nvd_ce_r1.jsonl.zst` **i=422**, `hOld=0x5c000072` |

`0x5c000072` is the same handle in all three rows, in every capture that reaches `cuCtxCreate`.
The guest's own kernel reports both halves in one boot:

```
run_s51_d502ac6_engroute_probe.log:424   ALLOC hClass=0x000083de … hObject=0x5c000072  (userspace: NV_OK)
run_s51_d502ac6_engroute_probe.log:785   NVRM: rpcRmApiAlloc_GSP: GspRmAlloc failed: … hClass=0x000083de; status=0x00000056
```

⇒ **The guest tolerates our alloc refusal and keeps the object.** That single fact is what makes
the other two rows possible at all.

> ⚠ **INSTRUMENT NOTE, paid for during this rung.** A first pass read the alloc status at
> offset 24 and reported `NV_OK` for *every* alloc in the stream — 100/100. `NVOS64_PARAMETERS`
> (`iocsize=48`) puts `status` at **offset 40**; offset 24 is `pRightsRequested`, which is
> `NULL`, i.e. a **constant zero that decodes as success**. `NVOS21` (`iocsize=32`) is offset
> 28. The re-decode agrees with the original reading here, but it agreed **by luck**, and a
> uniform-`NV_OK` histogram is the shape that should have been suspected first.

> ⚠ **AND A SILENCE THAT IS NOT A NEGATIVE.** `run_w294cup2_probe.log` contains **no**
> `GspRmAlloc failed … 0x83de` line, which reads as *"the alloc was not refused on that boot"*.
> Its dmesg section **begins at t=81 s** and is a tail: the alloc happens during `cuCtxCreate`
> around t=60–70 s and is simply **not in the capture**. The refusal is in that boot's own QEMU
> ledger. ⇒ Absence in a truncated log is absence of *capture*.

## 2. Why it hid — every gate in this tree quantified over CONTROLS

`DENIED_CONTROLS` and `DENIED_CLASSES` were two independent sorted lists with **no predicate
relating them**. `no_denied_id_is_a_boundary_specific_control`,
`the_permitted_and_denied_sets_are_disjoint…`, `admitted_is_served`, `served_chain_seats`,
`gpfifo_schedule` — every one sweeps **ids**. A class refused while its own controls are
admitted is invisible to all of them, by construction.

And it surfaced three planes from its cause: as an **`RM_FREE` status**, in a guest dmesg
assertion, after `cuCtxCreate` had already returned.

## 3. The mechanism that landed

- `capability::control_owner_class(cmd) = cmd >> 16` — RM's own encoding
  (`FINN_<CLASS>_<IFACE>_INTERFACE_ID << 8 | index`, and the interface id is `class << 8 | iface`),
  which this module already depended on in its binary-API rule. Naming it lets a **class**
  question be asked of a **control**.
- `CapabilityTable::control` refuses any command whose owner class is in `DENIED_CLASSES`,
  carrying the **class's** name and reason.
- ★★★ **It sits ABOVE both blanket rules, and that is the half that outlives this one id.**
  `RM_GSS_LEGACY_MASK` is bit 15 — *half the command space of every class* — and
  `NV2081_BINAPI_CLASS` is a whole class; both admit commands with **no table row at all**. A
  denied class's controls could leak through a blanket that never names them, and **no allowlist
  edit could ever have revealed it**. `0x83de0309` has bit 15 clear and so reached the allowlist;
  `0x83de8xxx` would not have.
- Gates: `no_control_is_admitted_on_a_denied_class` (every boundary × every denied class × the
  resolved answer, plus two synthetic probes per class for the blanket rules) and
  `the_class_gate_names_a_deliberate_disagreement` — the **known-positive**, which builds a
  deliberate class/control disagreement, asserts the predicate names exactly it, asserts the
  resolved answer flips, and then asserts the *same control row with the class row removed* is
  `Listed`, so the refusal cannot be the fixture being unreachable for some other reason.

⇒ **The owner's lever is now one line.** Denying a class denies its controls; admitting a class
re-admits them. There is no second list to keep in step.

## 4. What refusing the FREE actually causes — named, not assumed

Read from `ogkm-580.159.04: src/nvidia/src/libraries/resserv/src/rs_client.c:843-870`:

```c
status = serverFreeResourceRpcUnderLock(pServer, pParams);
NV_ASSERT((status == NV_OK) || (status == NV_ERR_GPU_IN_FULLCHIP_RESET));
objDelete(pResource);                     // ← unconditional
…
done:
    …clientDestructResourceRef(…);        // ← unconditional
```

1. **Nothing leaks in the guest.** `objDelete` and `clientDestructResourceRef` run regardless of
   our status; the assert prints and is discarded.
2. **Nothing leaks on our side.** We refused the alloc, so we never held the object — nor the
   RM-internal client/device/subdevice hierarchy `_ksmdbgssnInitClient` builds
   (`kernel_sm_debugger_session.c:122-198`), which `ksmdbgssnFreeCallback_IMPL` (`:441-457`)
   would otherwise tear down.
3. **No retry loop.** The free is issued once.
4. The cost is one guest assertion line and a `0x56` libcuda ignores. It is **post-verdict** —
   `cup2` reaches `CUP2_RC=0` with it present.

⇒ Refusing the free is harmless *and* is the correct consequence of refusing the alloc. It is
not the row to change.

## 5. ★★★ THE OWNER CALL — and the boundary that admitting the class would widen

The port must pick one; today it had both.

| | **(A) keep the class denied** — what landed | **(B) admit `GT200_DEBUGGER`** — owner's call |
|---|---|---|
| alloc | refuse | serve |
| `0x83de0309` | refuse, with the class's reason | serve (w292's ruling restored **in full**, automatically) |
| free | refuse (`FreeUnknown`) | serve |
| widens the boundary? | **no** — strictly narrowing | **yes** — see below |
| edit to get there | — | delete the `0x83de` row from `DENIED_CLASSES` |

**(A) landed because it is the direction a default-deny table must fail in, and because it needs
no ruling.** But the evidence for (B) is real and should be weighed:

- ★ **Every CUDA context allocates `GT200_DEBUGGER`. This is not a debugger attaching.**
  A real GA106 allocates it at record **401** and frees it at **451** in *every* host capture
  that reaches `cuCtxCreate` (`ctx`, `alloc`, `ce`, `launch`, both runs,
  `nvidia-gpu-passthrough/traces/host_reference_ga106/`). Our guest does it at i=366 in every
  capture. libcuda issues it unconditionally.
- **The C permits it** — `nvidia-gpu-passthrough/src/qemu/nvkvm_fe_alloc_allowlist.h:79` — and
  `cap3`, the green run, has the full triple: alloc seq **447712**, control seq **453701**, free
  seq **483065**.
- **nvproxy permits it.** `DENIED_CLASSES`'s own comment says so: *"nvproxy permits it because
  gVisor forwards to real silicon"*. gVisor admits this class to untrusted sandboxed CUDA.
- **Our refusal does not deny the guest the capability** (§1): it gets the object anyway. The
  refusal buys no reduction in what the guest can do; it buys a desynchronisation.

**⚠ THE BOUNDARY (B) WIDENS, NAMED:**

1. **It admits the object whose *allocation* is what creates the RC-deferral side effect** —
   w292's own correction says so in as many words: *"The RC-deferral side effect is created by
   allocating the `GT200_DEBUGGER` object, not by this control."* Today we refuse that
   allocation on our side.
2. **It makes the five still-denied `0x83de03xx` controls addressable on an object we admitted** —
   `SET_MODE_MMU_DEBUG` (`0307`), `READ_ALL_SM_ERROR_STATES` (`030c`),
   `CLEAR_ALL_SM_ERROR_STATES` (`0310`), `SUSPEND_CONTEXT` (`0317`), `RESUME_CONTEXT` (`0318`).
   They stay refused by name, so the result is *"attach, then fail at first use"* — the shape
   `DeniedBecause::NoPhysicalBoardBus` calls **"the worse shape"** for `NV40_I2C`.
3. **The free is not one object.** `ksmdbgssnFreeCallback_IMPL` frees an RM-internal client
   hierarchy built at construct time (`NV01_ROOT` + `NV01_DEVICE_0` + `NV20_SUBDEVICE_0`), so
   admitting the class means admitting that lifecycle too.
4. `KernelSMDebuggerSession` allocates `RS_FLAGS_ALLOC_NON_PRIVILEGED` and its debug controls
   carry `NON_PRIVILEGED` flags `0x10248` — i.e. the surface is reachable by an **unprivileged
   guest application**, which is the original and still-valid reason for the denial.

⊘ **Point 2 is the honest tension**: (A) is also *"refuse the attach"*, which that same comment
prefers. The difference measured this rung is that **the guest gets the object regardless**, so
(A) does not actually prevent the attach — it only prevents *us* from knowing about it.

## 6. What this rung does NOT claim

- ⊘ It does **not** make `cup2` do anything new. The whole triple is post-`cuCtxCreate`; the
  regression check is that `^CUP2_RC=` is still **0**.
- ⊘ It does **not** touch the free path. `RmGraphError::FreeUnknown` is unchanged.
- ⊘ It says nothing about the other three denied classes (`0x003f`, `0x0071`, `0x402c`); the new
  gate now covers them, and `0x402c0101` was already correctly on the unserviced `LEDGER` — the
  precedent this rung generalises.

## 7. The verification boot — `w295cup2`, rev `940c0648` (≡ `f0c9fdc2` after rebase)

Relaxations carried, byte for byte from w294 — ★ **a relaxed green is a MAP, not the milestone**:
`KAYFABE_VAS_PUBLISH=drain`, `PT_SWEEP=on`, `OPERAND_JOIN=join`, `FB_JOIN=shared`,
`GR_ROUTE=passthrough`, four pinned regions, `ISOLATES=real`, `CE_EXECUTOR=host`.
Stamp gate: `STAMP=940c064844dd… HEAD=940c064844dd…` — the binary is this revision.

| check | expected | measured |
|---|---|---|
| `^CUP2_RC=` **anchored** | `0` (w294 baseline) | ★ **0** — no regression |
| `Xid` | 0 | **0** |
| `host_rows` | `18295 / 18309` | **`18295 of 18309`** |
| `not_granular` | 6 | **6** (223 readings; 669 of `=0`) |
| `0x83de0309` | was `control … result 0x00000000 x1` | **`unserviced fn 76 cmd 0x83de0309`** |
| distinct unserviced ids | 40 | **41** — the +1 is exactly this id |
| bridge refusals | 20 total / 6 distinct | **20 / 6, unchanged** |

⊘ The whole triple is **post-verdict** — it happens after `cuCtxCreate` returns — so a green
`CUP2_RC` was never in doubt and is a **regression check, not evidence for the change**.

## 8. Reds — INHERITED, stated, not fixed here

`[measured at both `72f902f` and `940c0648`, same command, same bench]` **five test targets / nine
tests fail identically on master and on this branch.** Zero added, zero removed:

```
kayfabe-isolate-host  executor_vas_census   the_isolates_address_space_has_exactly_one_mint
kayfabe-isolate-host  guest_ring_census     the_birth_witness_is_read_by_no_decision
                                            the_rings_geometry_is_per_channel_and_stays_that_way
kayfabe-tests  ce_representability_split    the_two_publish_chains_declare_opposite_backing_kinds…
kayfabe-tests  doorbell_reaches_the_completion_observer
                                            a_guest_doorbell_reaches_the_host_completion_observer
                                            a_second_doorbell_over_an_unchanged_ring_forwards_nothing
                                            the_observers_negative_verdict_refuses_the_guest_doorbell
kayfabe-tests  ring_out_of_our_own_framebuffer
                                            a_device_with_no_fb_source_refuses_the_vidmem_ring
                                            a_wired_device_refuses_a_framebuffer_page_nothing_ever_wrote
```

⚠ **`cargo test --workspace` fail-fasts and reported only ONE of these five.** `--no-fail-fast`
is what makes the set visible; a "one pre-existing red" reading is an artefact of the flag.
★ A **sixth** target, `kayfabe-linux-raw --lib` (`spawn_unsafe::tests::a_child_runs_from_an_image_with_no_path_at_all`), failed once in a full `--no-fail-fast` sweep and passed **3/3 in isolation** — the known flake, load-dependent, not a red.
⊘ Also inherited and **not touched**: `cargo fmt --all --check` (21 files) and
`cargo clippy --workspace --all-targets -- -D warnings` (5 errors) are red at `origin/master`.

## 9. ⚠ The `OBJECT_CONTROLS` mirror went stale for the SECOND time in two rungs

w294 found `tests/tests/gpfifo_schedule.rs`'s `assert_eq!` copy of `OBJECT_CONTROLS`
*"STALE BY TWO RUNGS"* and fixed the membership. **The fix did not make the mirror stop being a
mirror**, and this rung's one-line retraction went stale again immediately.

⇒ It caught the edit — the case FOR it — and is also why the edit had to be made in two places —
the case against. Left as a ratchet deliberately, and noted here: **a third staleness should
convert it from a hand-copied list into a derivation.**
