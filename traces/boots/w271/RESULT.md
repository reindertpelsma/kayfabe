# w271 — RESULT: the high address is NOT a host pointer. Native GA106 does the same thing.

**STATUS: LIVE — 2026-08-12.** Analysis rung over the committed `w271` boot logs
(`1af77ac`), the committed native reference (`traces/native_dataplane_ga106/`), and
`ogkm-580.159.04`. **No GPU time and no build were spent**; every question the brief asks
is answerable from artefacts already in the tree.

Source revision of the boots analysed: **`5feac90`** (stamped in the binary; STAMP GATE PASS).

---

## ★★★★★ LEAD — THREE OF THE BRIEF'S CLAIMS ARE REFUTED, AND THE ALARM IS STOOD DOWN

### ⊘⊘ 1. `0x75b2_aee00000` is NOT a host pointer, and it is NOT a VA-identity violation.

The brief asked me to say so *loudly* if a host pointer were crossing into GPU-visible
state. **It is not.** Three independent measurements:

**(a) The sibling address is written by the GUEST, into the GUEST's own pushbuffer.**
`0x75b2b9000000` is not something we handed out — it is the guest's own CUDA driver
assembling a compute-class method. Our decoder shows the two words verbatim:

```
[2] sub1/m0x2a0/Incrementing/n1=[0x75b2]      NVC7C0_SET_SHADER_SHARED_MEMORY_WINDOW_A
[3] sub1/m0x2a4/Incrementing/n1=[0xb9000000]  NVC7C0_SET_SHADER_SHARED_MEMORY_WINDOW_B
```

`0x02a0`/`0x02a4` confirmed against `ogkm-580.159.04:src/common/sdk/nvidia/inc/class/clc7c0.h:424`
(`_A_BASE_ADDRESS_UPPER` is `16:0`, so `0x75b2` is a legal upper field, not a truncation).

**(b) It tracks the GUEST's per-boot ASLR, in BOTH arms.** The brief reports `0x75b2` as
pin-arm-only. It is — *lexically*. The **phenomenon** is in both arms at 16 occurrences
each; only the random bits differ, and they differ **with the guest's own libcuda base**,
which the in-guest probe records:

| arm | guest `libcuda` base | `SET_SHADER_SHARED_MEMORY_WINDOW` | same 32 GiB ASLR slot? |
|---|---|---|---|
| `off` | `0x76b5dc200000` | `0x76b5d1000000` | yes (`0xed6`) |
| `pin` | `0x75b2c4e00000` | `0x75b2b9000000` | yes (`0xeb6`) |

The fault address `0x75b2aee00000` is in the **pin arm's** slot (`0xeb6`), 162 MiB below the
shared-memory window. A *host* pointer would not track the *guest's* ASLR across two boots.

**(c) ★★★★★ NATIVE, UNVIRTUALISED GA106 EMITS THE SAME SHAPE.** This is the decisive one, and
it is already in the tree. `traces/native_dataplane_ga106/run_20260812T111414Z/nvdp.log` —
real GA106, no QEMU, no emulated GPU — decodes `cup2`'s own pushbuffer:

```
[ 0] mth=0x0188 val=0x00007f4f  NVC7C0_OFFSET_OUT_UPPER
[ 1] mth=0x018c val=0x66200000  NVC7C0_OFFSET_OUT
```

⇒ a GPU VA of **`0x7f4f_66200000`**. And the same log's mmap census shows that process's own
`/dev/nvidiactl` mappings at `0x7f4f70ddf000`, `0x7f4f6a400000`, … — **the same `0x7f4f`
slot.** On real hardware the GPU VA *is* the process VA. That is UVM unified addressing,
working as designed.

> ⇒ **A `0x7xxx_xxxxxxxx` GPU VA is the NORMAL CUDA regime, measured on bare metal.** The
> `0x2_xxxxxxxx` family every previous rung lived in is the *other*, RM-managed family. The
> campaign has been looking at one of two address families and reading the second as an
> intrusion.
>
> ⊘ **"~129 TB, exactly the shape of a host `mmap` return" was a correct observation with an
> inverted conclusion.** It is the shape of a *CUDA* mmap return — and on this platform the
> guest's CUDA and the host's CUDA draw from the same 47-bit space, so **shape cannot
> discriminate origin at all**. The discriminators that work are *who wrote it* (the guest's
> pushbuffer) and *what it correlates with* (the guest's ASLR).

### ⊘ 2. "Five identity facts changed" is FOUR — engine and channel are ONE fact, projected twice.

Read off the driver, not inferred. `ogkm-580.159.04:src/nvidia/generated/g_kernel_channel_nvoc.h:1493`:

```c
static inline NvU32 kchannelGetDebugTag(const struct KernelChannel *pKernelChannel) {
    ...
    return (pKernelChannel->runlistId << 24) | pKernelChannel->ChID;
}
```

⇒ `0x01000011` = **runlist 1, ChID 17**. `0x00000009` = **runlist 0, ChID 9**.

The channel field's high byte *is* the runlist, and the runlist *is* the engine class. So
"channel changed" and "engine changed `CE2`→`GRAPHICS`" are the **same measurement reported
twice**, and they agree. Grading them as two independent facts inflates the substitution.
(This is `two_projections_of_one_fact_disagreeing`, here in its benign form: agreeing.)

The genuinely independent facts are **four**: engine/runlist, client (`HUBCLIENT_CE0` →
`HUBCLIENT_FE`), address family (`0x2_…` → `0x75b2_…`), fault level (`FAULT_PTE` →
`FAULT_PDE`). All four still moved. The brief's core point — *a count cannot see a
substitution* — stands; only the arithmetic changes.

### ⊘ 3. The false-negative summary line was ALREADY FIXED — before the brief was written.

`11b75a7` ("⊘⊘ MY OWN SUMMARY LINE WAS A FALSE REPORT") is exactly this fix, with its own
`[measured 2026-08-12, w271_pin]` citation at `crates/kayfabe-qemu-raw/src/shim.rs:4595`.
`PinnedRun::placed_as_asked` now means *"every segment **this call** placed landed as
asked"*, and a multi-segment run prints `(several — see the per-segment list)` instead of
`0x0`.

`git merge-base --is-ancestor 11b75a7 5feac90` → **NO**: the fix landed **after** the boot's
build revision, which is precisely why the committed logs still show the false line. Nothing
to do. **Seventeenth consecutive lane to find its premise already built.**

---

## ★★★★★ WHAT THE BRIEF UNDERSOLD — the pin fix DELETED the old fault

The brief frames w271 as "the fault underwent a substitution". It is stronger than that: the
`off` arm's fault address is **exactly the address the `pin` arm pins**.

- `off` fault: `CE2 … faulted @ 0x2_04420000`
- `pin` arm: `OPERAND-PIN … va=0x204420000 … GREW … requested=131072 described=131072`

and the pin arm has **zero** CE faults (1 Xid total, on GRAPHICS). The forward progress is
measurable and large:

| | `off` | `pin` |
|---|---|---|
| `DOORBELL-XLATE` total | 17 | **88** |
| doorbells on token `0x0001000f` | 1 | **69** |
| doorbells on token `0x00000007` (GR compute chan 0) | 1 | **5** |
| `OPERAND-PIN` events | 0 | 88 |
| wall budget | 254 s | 256 s (equal) |

⇒ Same wall budget, 5.2× the doorbells. **The CE wall at `0x2_04420000` is closed**, and what
we are now looking at is the *next* wall, one engine deeper.

---

## THE THREE MEASUREMENTS THE BRIEF ASKED FOR

### 1. Provenance — named, with evidence

Of the brief's four candidates:

| candidate | verdict |
|---|---|
| a host pointer written into guest-visible state | **REFUTED** — (a)(b)(c) above |
| an uninitialised field we zero-fill wrongly | **REFUTED** — the value is two guest-authored pushbuffer words we read verbatim |
| RM's own placement for a non-FIXED object | **REFUTED** for the sibling (the guest wrote it); unproven-but-unnecessary for the fault address |
| **a legitimate high GR VAS address** | ★ **FIRES** — and native GA106 uses the identical construction |

⊘ **The fault address `0x75b2_aee00000` itself appears NOWHERE in our QEMU log** — zero hits
across every artefact except the host `dmesg` line that reports it. We never observed the
guest name it. That absence has a documented cause on this exact path:

```
GR-ADDRESS-CENSUS proc=2 chan=N class=0xc7c0 operands=5 bound=4 unbound=1 mme_dwords=39
```

**39 dwords of MME microcode are loaded on every one of the eight compute channels**
(`addr=0x0118 count=15` + `count=24`, both arms). The Macro Method Expander's *output is
methods* — `completion_watch.rs:220` says so, and `the_mme_defeats_every_method_allowlist`
is the standing finding. ⇒ **A method-stream decoder is structurally unable to see an address
the MME synthesises**, and an address absent from our decode but present in a hardware fault
is the exact signature. This is a **hypothesis with a named mechanism**, not a measurement —
see "cannot prove" below.

### 2. Whose channel is `0x00000009`?

**Ours — our isolate's host channel, on the GR runlist.** Both arms' Xid names
`name=memfd:kayfabe-i` (`pid=2250782` off, `pid=2251707` pin); `kernel_rc.c:382` sources that
name from `pKernelChannel`'s owning `RmClient`, so it is the *channel owner's* process, not
the reporter's. Decoded: **runlist 0, ChID 9**.

Which of ours, exactly, cannot be said. We materialise **ten** host GR channels — 8 ×
`class=0xc7c0` `GrCompute` (parents `0x5c000019`…`0x5c000037`) and 2 × `class=0xc797`
`GrGraphics` — identically in **both** arms, and **we never log the host-assigned
`hwRunlistId`/`hwChannelId` anywhere**. ⇒ Concrete instrument gap: record the host chid at
channel materialisation (UVM reads exactly this pair as
`channel_info.hwRunlistId`/`hwChannelId`, `uvm_channel.c:4350`) so the next Xid names itself.

⊘ Note the near-miss the brief's framing invites: our own log has `vchid=VChid(0x9)` on
`proc=2 chan=2`. **That is a different number space** — a guest-side token, not a host chid.
Reading them as the same would have attributed the fault to the wrong channel.

### 3. Table miss, or failed descent? — **FAILED DESCENT**, and both walkers agree

Our resolver's verdict is `kind: "Fault"`, which is `CeResolve::Fault(TranslateFault)`.
`crates/kayfabe-device/src/ceresolve.rs` defines that variant as **"MISS = FAULT, arriving
from the guest's own page tables"** — an unmapped or sparse entry **at a named level**. It is
a *different* variant from `NoPublication` (no root published — i.e. a true bookkeeping miss),
`NoRootLevel`, and `AddressOutOfRange`. ⇒ We descended the guest's real page tables and hit an
invalid entry.

Hardware says the same thing independently: `FAULT_PDE` — the GPU's own walker failed at a
**page-directory** entry, not at a leaf (contrast the `off` arm's `FAULT_PTE`).

> ⇒ **This is not "our table forgot something the guest told us."** The guest's own page
> tables genuinely do not describe that VA, and our walker reports that correctly. The
> address table is behaving.
>
> ★ Which points the next rung at UVM rather than at the address table: UVM's design is to
> leave VAs unmapped and populate them **on GPU fault**, via the replayable fault buffer. On
> native hardware that fault is serviced and replayed. Here it arrives as a **non-replayable
> Xid 31 against our isolate's host channel** — fatal instead of serviced. That is a
> structural gap with a name, and it is consistent with the standing nvdiff finding that the
> guest runs in lockstep with hardware exactly **to `UVM_MAP_EXTERNAL_ALLOCATION`**.

### 4. `CUP2_RC` — **124 on both arms**

Measured with the anchored pattern. The brief's trap reproduces exactly:

```
grep -rh '^CUP2_RC='  → 124, 124
grep -roh 'CUP2_RC=[0-9]*'  → 2 × CUP2_RC=0   (from GCC_CUP2_RC=0)  +  2 × CUP2_RC=124
```

Both arms timed out. No movement off 124.

---

## PRE-REGISTRATION — stated honestly

⊘ **I did not pre-register before measuring**, and I will not back-date one: this rung became
a read of committed artefacts rather than a new boot, and the first log I opened already
carried the answer. What I can report is which of the brief's four arms fired:

| arm | brief's weighting | fired? |
|---|---|---|
| host-pointer leak | highest (the "say so loudly" arm) | **NO — refuted three ways** |
| legitimate-but-unpopulated GR range | lower | ★ **YES** |
| table miss vs failed descent | — | **failed descent** |
| `CUP2_RC` moves off 124 | low | **NO** |

**The least-weighted substantive arm fired again** — seventh of the last eight rungs.

---

## ⊘ WHAT THIS ANALYSIS CANNOT PROVE

1. **Where `0x75b2_aee00000` specifically comes from.** I proved its *family* (guest CUDA
   unified VA, same ASLR slot, same construction native hardware uses) and that it is absent
   from our decode. I did **not** prove which producer emits it. The MME is a named mechanism
   with 39 loaded dwords on the path, not a measurement.
2. **That it is the shader LOCAL memory window.** Plausible (162 MiB from the shared window;
   `SET_SHADER_LOCAL_MEMORY_WINDOW` = `0x07b0`) but **`0x07b0` never appears in either arm's
   pushbuffer** — so if it is set, it is set somewhere we do not observe. Unmeasured.
3. **Which of our ten host GR channels is runlist-0 ChID 9.** We do not log host chids.
4. **Whether the shared-memory window is *supposed* to be page-table-backed.** Not measured
   either way; the native trace's larger pushbuffer segments were never decoded.
5. **Anything about ordering.** These are post-hoc log reads; the release-vs-`GP_GET`
   ordering hole named in the native doc §8 is untouched.
6. **That the GR fault is the last wall.** One fault was observed. Removing it may expose
   another, exactly as removing the CE2 fault exposed this one.
7. **Anything under a different guest, workload, or driver version.** One workload (`cup2`),
   one chip (GA106), one driver (`580.159.04`), two boots.

---

## NEXT RUNG — what is worth measuring, ranked

1. ★★★ **Instrument the host chid** at channel materialisation. Cheap, and it turns every
   future Xid from "one of ten" into a name. Blocks nothing.
2. ★★★ **Decode the MME microcode**, or at minimum record the 39 dwords verbatim, and check
   whether any synthesised method can carry `0x75b2_aee00000`. This is the only route to
   promoting hypothesis (1) above into a measurement.
3. ★★ **Ask whether the host channel has a replayable-fault path at all.** If our
   materialised GR channels are allocated without fault buffers, a UVM-managed VA can only
   ever fault fatally — and the wall is then *structural*, not an address-table defect.
4. ⊘ **Do not add `0x75b2_…` to the address table.** Nothing observed says the guest ever
   asked us to map it, and populating a VA the guest's own page tables leave invalid would be
   fabricating a mapping — the `cap2b` class, pointed inward.
