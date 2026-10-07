# w281 / RESULT — THE HOST COPY ENGINE EXECUTED THE GUEST'S METHODS, AND FAULTED ON THE OPERAND

**STATUS: LIVE — 2026-08-12.** Branch `w281-lift-the-pushbuffer-refuse`, both boots at source
revision **`eb20f8209904b4f35cf5c118cf0c17e334983fa6`** — stamp gate PASS on both (binary
`kayfabe-rev` == tree HEAD, both printed), tree clean, `cap2b` guard **0**, this rung's own
address-literal guard **0**, `ENOSPC_LLVM=0`, `=== W281 EXIT rc=0 ===`. All six carried arms
PASS on both. Every number below was read from an artefact opened in this session.

Pre-registration: `PREREGISTRATION.md`, committed before the boots.

---

## ★★★★★ LEAD — THE PLANE CROSSED FROM "REFUSED" TO "EXECUTED ON REAL HARDWARE"

One variable: `KAYFABE_PUSHBUF_VIDMEM`. Same binary (`GUEST_MD5=a28f06884ed3080e7c1d9b9185a46ca2`
on both, equal to the native md5), same 53 ioctls, same six carried arms, route B ON on both.

| | `w281_clientoff` (route **OFF**) | **`w281_client`** (route **ON**) |
|---|---|---|
| `FWD-PUSHBUF` lines | **0** | **1** — `proc=2 chan=0 ranges=1 → VIDMEM RUNS PLANNED (pb_vidmem=true fb_source=true)` |
| refusal VAs decoded | `0x0 0x120000000 0x120010000` **`0x120020000`** | `0x0 0x120000000 0x120010000` — ★ **`0x120020000` GONE** |
| first doorbell refusal | `PushbufferAperture { va: GpuVa(4831969280) }` = **the pushbuffer** | `RmError::Other(19275)` = **`CE_NEVER_RETIRED`** |
| `CE-SUBMIT` | **0** | **1** — `dst=0x120010000 len=4096 **by=HostCe**` |
| **host Xid** | **0** (`HOST_DMESG_XID=0`) | ★★★ **1** — `Xid 31 … ENGINE CE0 HUBCLIENT_CE1 faulted @ 0x1_20010000, FAULT_PTE ACCESS_TYPE_VIRT` |
| `R33_RC` | 1 | 1 |
| client arm 1 | FAIL, `GP_GET 0 PUT 1` | FAIL, `GP_GET 0 PUT 1` |

**H1 FIRED** (the pushbuffer VA is gone from the refusals, and present on the control).
**H2 FIRED** (the route was actually taken, not merely armed).

★★★★★ **And the third fact is bigger than either.** With the route on, the decoded methods
reached a **real host copy engine**, which **fetched them, tried to execute them, and faulted
on the guest's own destination operand**:

```text
CE-SUBMIT dst=0x120010000 len=4096 by=HostCe gp_get=1 gp_put=1
          sem=0x00000000 want=0x00000001 → NEVER-RETIRED
Xid (PCI:0000:00:07): 31 … MMU Fault: ENGINE CE0 HUBCLIENT_CE1
          faulted @ 0x1_20010000. Fault is of type FAULT_PTE ACCESS_TYPE_VIRT
```

`0x1_2001_0000` is **exactly the destination the guest's own pushbuffer declared** — method
`[1]sub4/m0x400/n4=[0x1,0x20000000,0x1,0x20010000]` ⇒ src `0x1_2000_0000`, dst `0x1_2001_0000`,
`[2]m0x418=[0x1000,0x1]` ⇒ 4096 bytes. ⇒ **The engine could not have faulted at an address it
never read.** The control's host Xid is **0**, so the fault is attributable to this flag and to
nothing else.

⇒ **This is the first time the guest's own CE methods have been executed by host hardware on
this workload.** `w246`'s *"route B enumerates a ring and does not submit work"* is now
superseded **for the pushbuffer route** — and note the shape: it took the *pushbuffer*, not the
ring, to get there.

---

## ⊘⊘⊘ LEAD THE OTHER WAY TOO — **ZERO OF THE THREE PASS-CRITERIA ARE MET IN THE GUEST**

The brief asked which of the three the guest meets. **None.** Verbatim, from the client:

```text
FAIL R33 arm 1 COPY = dst[0] 0x3f0011cc -> 0x3f0011cc (want 0xc0ffee33),
     dst[last] 0x3f0011cc (want 0xc0fff232), semaphore 0x00000000 (want 0x00000001),
     GP_GET 0 GP_PUT 1 — the entry was NEVER fetched
```

1. **`GP_GET` catches `GP_PUT`** — ⊘ **NO.** `GP_GET 0 GP_PUT 1` in the guest, unchanged.
2. **The bytes moved** — ⊘ **NO.** `dst[0]` still reads its pre-fill `0x3f0011cc`.
3. **The semaphore carries the declared payload** — ⊘ **NO.** `0x00000000`.

Arms 2 (OCCUPIED) and 3 (FREE) green, as before. Native arm, same binary, minutes earlier:
**all three met** (`4096 bytes moved … engine semaphore 0x00000001 … GP_GET 1 caught GP_PUT 1`).

### ⚠⚠ THE TRAP THIS RUNG PLANTED, NAMED BEFORE ANYONE READS IT WRONG

The `CE-SUBMIT` line says **`gp_get=1 gp_put=1`**. **That is the HOST channel we forwarded onto,
not the guest's.** The guest's own USERD still reads `fbuserd@0x50088 GET=0 PUT=1`, and the
client's verdict says `GP_GET 0 GP_PUT 1`. ⇒ **Criterion 1 is NOT met**, and a reader grepping
`gp_get=1 gp_put=1` would conclude it was. Same class as `a_count_cannot_see_a_substitution`
and as `w280`'s `CE-SUBMIT 0 → 68`: **the number moved and the thing counted changed.**

---

## ⊘⊘ WHAT I MUST *NOT* CLAIM — H4 WAS ALREADY TRUE ON THE CONTROL

H4 predicted the methods would decode (`pbm[16w of 64B]`, `SET_OBJECT 0xc7b5`, the semaphore
`0x120022000`). They do — **and the control prints the identical line.** The doorbell descent's
`pbm[..]` decode is a **diagnostic** that reads the framebuffer directly; it is a different
reader from `read_pushbuffer`, and it was never gated on `VidmemRoute`.

⇒ **H4 is NOT evidence for this rung.** The evidence that the *forwarding* path decoded them is
`FWD-PUSHBUF` (1 vs 0) and `CE-SUBMIT` (1 vs 0), and only those. ⚠ Two readers producing the
same line on both arms is exactly the shape that would have let a green be claimed from the
control's own output.

## ARMS — how they fell

| # | prediction | outcome |
|---|---|---|
| **H1** | `PushbufferAperture` at `0x1_2002_0000` GONE, a new differently-named refusal | ★★★★ **FIRED** — gone on the armed arm, present on the control; new refusal is `CE_NEVER_RETIRED` |
| **H2** | `FWD-PUSHBUF … VIDMEM RUNS PLANNED` with `pb_vidmem=true fb_source=true` | ★★★ **FIRED** — 1 vs 0 |
| **H3** | the three criteria met in the guest | ⊘ **did not fire** — none met, as pre-registered ("not predicted") |
| **H4** | the methods decode | ⊘ **UNUSABLE** — true on both arms, different reader. See above |
| **H5** ⚠ | a blank vidmem pushbuffer forwards something | ⊘ **did not fire** — the one `CE-SUBMIT` carries the guest's **declared** dst/len, not a blank page's |
| **H6** | the wall moves to the CE operand / semaphore | ★★★★★ **FIRED, and further than predicted** — not a refusal at the operand but a **hardware fault** at it |
| **H7** ⊘ | nothing changes | ⊘ did not fire |
| **H8** | the control is identical to `w280_client` | ★ **FIRED** — same refusal VA, same `R33_RC=1`, same arm-1 line ⇒ the device did not change under me |
| **H9** | a VOID boot | ⊘ did not fire — md5 matched on both, `total=53 failed=0`, `RING-PROJ=1`, `DOORBELL-XLATE=1`, 31 `NVRM` lines each |
| **H10** | boot fails / ENOSPC | ⊘ did not fire |

---

## ★ THE BLOCKER, BY NAME

**`CE_NEVER_RETIRED` (`0x4B4B`), caused by `Xid 31 FAULT_PTE` on `ENGINE CE0 HUBCLIENT_CE1` at
`0x1_2001_0000`.** In words: **the guest's CE operand VAs are not bound in the address space of
the host channel we submitted on.** The host engine read the methods, resolved `0x1_2001_0000`
in *its* page tables, found nothing, and faulted.

⊘ This is **not** the `RingFbNeverWritten`/`PushbufferAperture` family — those are *our*
refusals. This is **real hardware refusing a real address**, which is a different and better
kind of wall: it means everything upstream of the operand binding now works.

★ Note the continuity with `w277`'s `TABLE-DESCRIBES` finding and with
`shape_cannot_discriminate_origin`: the fault VA is one the **guest** declared, in the guest's
own range `0x100000000..0x11fffffff` (`root=0x6000/ap1/sh47`). The operand-publication problem
is now the single named thing between this client and a pass.

## ⊘⊘ WHAT THIS RUN CANNOT PROVE

- **It is not a pass.** Zero of three criteria in the guest. It is a **stage**.
- **It cannot say the operand binding is the LAST blocker** — only that it is the *next* one.
  Behind it sit the semaphore write-back and the guest's own `GP_GET` advance, neither of which
  has been reached.
- **The completion plane still has no oracle**; `sem=0x00000000 want=0x00000001` is our own
  read, not an independent witness.
- **It says nothing about `cup2`** — its pushbuffers are sysmem (`w280`, 16/16 `pb=S:`), so this
  gate is not on its path. No `CUP2_RC` was produced this rung and none is claimed.
- One workload, one chip (GA106), one driver (`580.159.04`), one boot per arm.

## ★ THE NEXT ONE FACT

Bind the guest's CE operand VAs into the host channel's address space before submitting, then
re-run this exact pair. The falsifier is already in hand and is hardware's own: **`HOST_DMESG_XID`
must go 1 → 0 while `CE-SUBMIT` stays 1**. If the Xid moves to a *different* VA
(`0x1_2000_0000`, the source, or `0x1_2002_2000`, the semaphore) that is the wall advancing
again and is a clean result; if it stays at `0x1_2001_0000` the binding did not take.

## ARTEFACTS

| what | where |
|---|---|
| pre-registration (committed pre-boot) | `PREREGISTRATION.md` |
| the whole run incl. every gate | `w281_run.log` |
| the armed arm | `run_w281_client_qemu.log.gz`, `_probe.log`, `_dmesg.log` |
| the control | `run_w281_clientoff_qemu.log.gz`, `_probe.log`, `_dmesg.log` |
| the native arm, same binary, same run | `xid_w281_native.log` |
| the change | `kayfabe-fwd/src/lib.rs` (`plan_`/`fetch_`/`decode_pushbuffer`), `kayfabe-rt/src/device.rs` (three-phase `parse_pushbuffer`, `set_pushbuffer_vidmem`), `kayfabe-qemu-raw/src/shim.rs` (`KAYFABE_PUSHBUF_VIDMEM`) |
| the tests, one watched RED | `tests/tests/pushbuffer_out_of_our_own_framebuffer.rs`, `tests/tests/l1_mean.rs` |
