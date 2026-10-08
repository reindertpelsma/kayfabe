# Physical-operand oracle — does an UNPRIVILEGED host channel honour a PHYSICAL operand?

**STATUS: RESEARCH, 2026-10-08 (extended same day with the GR/compute/video classes and the
physical-DESTINATION arm). Recommendation only; no ruling written. Blocks the owner's Windows
channel-policy decision (`OWNER_RULINGS.md` §U.3, `docs/design/the_three_channel_kinds.md` §3).**

**Headline: no method is honoured on an unprivileged channel on any engine tested or read.** The
copy engine is the only engine whose push-buffer methods can express a physical-aperture operand,
and the hardware refuses it (Xid 32) on a `USER` channel — for a physical SOURCE *and* a physical
DESTINATION, on two CE instances, with and without kayfabe's belt. The 3D, compute and video
classes have no physical-operand method at all (reading). So there is nothing to honour that the
hardware does not refuse, and the forward-unknown route has no honoured counterexample here.

## The question

Kayfabe only ever creates `PRIVILEGE_USER` host channels (`PRIVILEGED_CHANNEL=0`). The
forward-unknown proposal — forward an untranslatable push-buffer entry to an unprivileged host
channel whose VAS maps only the VM's own memory — is safe only if the hardware refuses a
**physical-aperture** operand on such a channel. A physical operand bypasses the MMU, so a
guest-authored physical address arriving at the engine unmediated is a **host** physical address
(`the_three_channel_kinds.md` §3).

- **Hypothesis (inferred):** hardware ignores or faults a PHYSICAL operand on an unprivileged channel.
- **Falsifier:** a copy whose SRC is PHYSICAL and names the physical address of a page the test owns
  (known pattern), DST virtual, delivers that pattern to DST from an UNPRIVILEGED channel =
  physical addressing is honoured = hypothesis false for that engine.

## Platform

| | |
|---|---|
| GPU | NVIDIA GeForce RTX 4070, AD104 (Ada), arch `0x4` |
| host driver | 595.91.07 |
| CE class exercised | `0xC7B5` AMPERE_DMA_COPY_B (the class the 4070's CE exposes) |
| source rev | `a5e0ebf1` (this branch); oracle built on the host from that commit |
| binary | `kf-harness` bin `kf-phys-oracle` (native-only, not in default CI) |
| date | 2026-10-08 |
| OGKM read | `open-gpu-kernel-modules` 595.84 tree (same 595 family as the host driver) |

Each arm is one process (`kf-phys-oracle <arm> <deny>`), so a channel RC cannot poison the next.
The two demo VMs were stopped before the runs (`interactive.sh stop`, `windows_broker.sh stop`)
and the host desktop/GPU health was checked after every arm (`nvidia-smi`, `dmesg`).

## OGKM reading (what the source states, not a run)

| source | wording | what it states |
|---|---|---|
| `alloc_channel.h:135-143` `NVOS04_FLAGS_PRIVILEGED_CHANNEL` | *"tells RM whether to give the channel admin privilege … needed so that guest can update page tables **in physical mode** and do scrubbing"* | physical mode is an **admin/privileged** capability of the channel |
| `alloc_channel.h:158-170` `NVOS04_FLAGS_DENY_PHYSICAL_MODE_CE` | *"deny access to the **physical mode of CopyEngine** regardless of whether or not the client handle is admin. If set to true, this channel allocation will always result in an **unprivileged** channel … primarily meant for vGPU since all client handles granted to guests are admin"* | physical CE mode is gated on admin; vGPU **explicitly** denies it on guest channels |
| `kernel_channel.c:271-291` | a CPU-RM client is `PRIVILEGE_USER` unless `rmclientIsAdmin`/kernel; the `PRIVILEGED_CHANNEL` bit is set only for admin/kernel. On GSP a guest-RM VF may request it *"to perform actions such as updating page tables in physical mode or scrubbing. Security … enforced by VMMU and IOMMU"* | the `USER`/`ADMIN` level is the gate; physical mode belongs to admin |
| enforcement | the `DENY_PHYSICAL_MODE_CE` bit is **defined** in the open header but **consumed** in GSP/engine, not in open CPU-RM | the open tree states the intent; the **hardware/GSP** enforces it — which is what the runs below test |
| `clc7b5.h:66-83` `SET_SRC/DST_PHYS_MODE` + `clc7b5.h:121-126` `LAUNCH_DMA_SRC/DST_TYPE` | method-level: `TARGET` = LOCAL_FB/COHERENT_SYSMEM/NONCOHERENT_SYSMEM/PEERMEM; `SRC/DST_TYPE` = VIRTUAL/PHYSICAL | the physical operand is a per-`LAUNCH_DMA` choice; **no** "privileged-only" wording at the method level — privilege is a channel/instance-block property |
| `clc56f.h:206-213` `SEM_ADDR_*`, `clc56f.h:266-284` `GP_ENTRY0/1` | GP entry and host semaphore address are **GPU VAs**; no per-entry physical-aperture field | at the host/FIFO level there is **no** guest-settable physical operand; the pushbuffer-fetch aperture is fixed by the channel's instance block (RAMFC) |

## Hardware result (the runs on the 4070, rev `a5e0ebf1`, 2026-10-08)

All births reported `PRIVILEGED_CHANNEL=0 privilege=USER` (the `born_user` reply check). `deny=1`
echoed `reply_flags=0x80` (`DENY_PHYSICAL_MODE_CE` bit 7); `deny=0` echoed `0x0`. Both are `USER`.

| engine / operand | channel (`deny_physical_ce`) | delivered? | CE released? | dmesg | verdict |
|---|---|---|---|---|---|
| CE `0xC7B5` VIRTUAL (control) | USER, deny=1 | **yes** `0x5a1d0000..1` | yes | no Xid | rig works (production flags) |
| CE `0xC7B5` VIRTUAL (control) | USER, deny=0 | **yes** `0x5a1d0000..1` | yes | no Xid | rig works (belt off) |
| CE `0xC7B5` PHYSICAL LOCAL_FB | USER, deny=1 (production) | **no** (DST sentinel intact) | no | **Xid 32** pid 2096908 | PHYSICAL REFUSED |
| CE `0xC7B5` PHYSICAL LOCAL_FB | USER, deny=0 (belt off) | **no** | no | **Xid 32** pid 2097004 | PHYSICAL REFUSED |
| CE `0xC7B5` PHYSICAL COHERENT_SYSMEM | USER, deny=1 | **no** | no | **Xid 32** pid 2097498 | PHYSICAL REFUSED (see sysmem caveat) |
| CE `0xC7B5` PHYSICAL COHERENT_SYSMEM | USER, deny=0 | **no** | no | **Xid 32** pid 2097704 | PHYSICAL REFUSED (see sysmem caveat) |
| CE `0xC7B5` PHYSICAL **DST** LOCAL_FB (own page) | USER, deny=1 | **no** (own page intact) | no | **Xid 32** pid 2109287 (`DBG1 0x218e`) | PHYSICAL REFUSED |
| CE `0xC7B5` PHYSICAL **DST** LOCAL_FB (own page) | USER, deny=0 | **no** | no | **Xid 32** pid 2109384 (`DBG1 0x218e`) | PHYSICAL REFUSED |
| CE `0xC7B5` PHYSICAL LOCAL_FB, **2nd CE** (engine `0xa`, COPY1) | USER, deny=0 | **no** | no | **Xid 32** pid 2109492 | PHYSICAL REFUSED |
| 3D `0xC997`, compute `0xC9C0`/`0xC6C0`+QMD, video `0xC9B0`/`0xC9B7`/`0xC9FA` | any | — | — | — | **N/A by construction** — no physical-operand method exists (reading, below) |
| host/FIFO `0xC56F` GP_ENTRY / SEM_ADDR | — | — | — | — | **N/A by construction** (no physical operand) |

The second round (physical DST, COPY1) ran with **both demo VMs still running** (interactive +
Windows desktop), ~700 MiB free VRAM; the oracle coexists with the VMs and the VIRTUAL control still
delivered. `HCE_DBG1` distinguishes the fault: `0x118e` for a physical SOURCE, `0x218e` for a
physical DESTINATION.

The LOCAL_FB source/destination was a VRAM object the test owns (`GET_SURFACE_PHYS_ATTR` →
`addr=0x8130000`/`0x3510000` `aperture=0` VIDMEM, sane ~50–135 MiB FB offsets), filled with the
known pattern for a source, or stamped with a sentinel for a destination. The physical DESTINATION
is only ever **the test's own page** (owner-authorised 2026-10-08): had the hardware honoured it,
the single thing written would be that page. No address the test did not allocate was ever named.

The Xid lines (verbatim), all `HCE_DBG0 00000300` = the host-copy-engine `LAUNCH_DMA` (`0x300`) that
carried the physical operand, channel RC'd:

```
NVRM: Xid (PCI:0000:01:00): 32, pid=2096908, name=kf-phys-oracle, channel 0x00000007 ... HCE_DBG0 00000300 HCE_DBG1 0000118e   # phys_fb (SRC) deny=1
NVRM: Xid (PCI:0000:01:00): 32, pid=2097004, name=kf-phys-oracle, channel 0x00000007 ... HCE_DBG0 00000300 HCE_DBG1 0000118e   # phys_fb (SRC) deny=0
NVRM: Xid (PCI:0000:01:00): 32, pid=2097498, name=kf-phys-oracle, channel 0x00000007 ... HCE_DBG0 00000300 HCE_DBG1 0000118e   # phys_sysmem (SRC) deny=1
NVRM: Xid (PCI:0000:01:00): 32, pid=2097704, name=kf-phys-oracle, channel 0x00000007 ... HCE_DBG0 00000300 HCE_DBG1 0000118e   # phys_sysmem (SRC) deny=0
NVRM: Xid (PCI:0000:01:00): 32, pid=2109287, name=kf-phys-oracle, channel 0x0000005e ... HCE_DBG0 00000300 HCE_DBG1 0000218e   # phys_dst_fb (DST) deny=1
NVRM: Xid (PCI:0000:01:00): 32, pid=2109384, name=kf-phys-oracle, channel 0x0000005e ... HCE_DBG0 00000300 HCE_DBG1 0000218e   # phys_dst_fb (DST) deny=0
NVRM: Xid (PCI:0000:01:00): 32, pid=2109492, name=kf-phys-oracle, channel 0x0000005e ... HCE_DBG0 00000300 HCE_DBG1 0000118e   # phys_fb (SRC) COPY1 deny=0
```

Xid 32 is the PBDMA / host-CE pushbuffer error; here the engine rejected the physical-mode
`LAUNCH_DMA` on a `USER` channel. The host survived every RC: `nvidia-smi` healthy throughout, no
"fell off the bus", no fatal. Host Xid count went 86 → 90 (first round) → 93 (the three second-round
arms; the VIRTUAL control raised none), and both demo VMs stayed up.

### Attribution

The belt-off (`deny=0`) VIRTUAL control **delivered** the pattern on the same `USER` channel, so the
null physical result is the **physical operand**, not a broken rig. The physical arms faulted with
`deny=0` just as with `deny=1` → on this engine the **`USER` channel privilege alone** refuses the
physical operand; kayfabe's `DENY_PHYSICAL_MODE_CE` belt is not what produces the fault (but it is
free, defends the same thing, and makes the refusal explicit).

### Caveats (what ran vs what is inferred, kept apart)

- **sysmem arm is weaker than the FB arm.** `GET_SURFACE_PHYS_ATTR` on an OS-descriptor returned a
  VA-like value (`0x7ff8…`), not a plausible RAM physical address, so the sysmem source address is
  suspect. The arm still **faulted** (nothing delivered, Xid 32), which is a safe negative; but the
  authoritative honour-vs-refuse evidence is the **LOCAL_FB** arm, whose address is a real FB offset.
- **No privileged-channel positive control.** `kf-host`'s `born_user` reply check refuses any
  ADMIN/KERNEL channel by design, and no kernel-module patch path to force `PRIVILEGED_CHANNEL`
  exists on this host. Per the task the control was **skipped, not hacked**. "Physical is honoured
  on a PRIVILEGED channel" is therefore **inferred** from OGKM, not measured here.
- **3D/compute/video need no run.** ⊘ CORRECTED 2026-10-08 (superseding the earlier "untested,
  needs its own arm"): a read of the class headers (next section) shows these classes expose **no**
  physical-aperture method at all — every memory operand is a VA through the channel VAS. There is
  therefore no physical operand to submit, so this is a **reading** result (N/A by construction),
  not a run that is missing.

## GR, compute and video classes — reading (no physical operand exists)

The owner's channel policy turns on the GR classes, because Windows' D3D and compositor channels
are GR (3D + compute). An enumeration of the Ada class headers in `third_party/ogkm` (595.84 tree)
for every method that takes a memory address with an aperture/TARGET field or a physical mode:

| class | what the header has for memory operands | `_APERTURE`/`_PHYSICAL`/`VID_MEM`/`SYS_MEM` count |
|---|---|---|
| 3D `0xC997` (ADA_A) | `SET_NOTIFY_A/B`, `SET_REPORT_SEMAPHORE_ADDRESS_*`, `PEER_SEMAPHORE_RELEASE_OFFSET_*`, I2M `LAUNCH_DMA`/`OFFSET_OUT`, texture/sampler pool bases, shader-local-memory bases — **all plain UPPER/LOWER VA pairs**; `LAUNCH_DMA` has only `DST_MEMORY_LAYOUT` (blocklinear/pitch) | **0** |
| compute `0xC9C0` → `0xC6C0` (ADA_COMPUTE_A) + `cla0c0qmd.h` | `SET_REPORT_SEMAPHORE_*`, I2M `LAUNCH_DMA`/`OFFSET_OUT`, `SET_SHADER_LOCAL/SHARED_MEMORY_*`, QMD `CONSTANT_BUFFER_ADDR_*`/program address — **all VA** | **0** |
| NVDEC `0xC9B0`, NVENC `0xC9B7`, OFA `0xC9FA` | semaphore and surface addresses are VAs | **0** |
| host/FIFO `0xC56F`/`0xC86F` | GP entry + `SEM_ADDR` are VAs; aperture fixed by the instance block | no per-entry physical operand |

⇒ **The copy engine (`0xC7B5`) is the only engine on this GPU whose push-buffer methods can express
a physical-aperture operand** (`SET_SRC/DST_PHYS_MODE` + `LAUNCH_DMA_SRC/DST_TYPE`). On GR, compute
and the video engines there is no physical-mode bit, no `SET_*_PHYS_MODE`, and no `VID_MEM`/`SYS_MEM`
aperture enum on any address method: a guest-authored physical address cannot even be encoded, so
the engine resolves every operand through the channel's VAS. Nothing to honour, nothing to refuse.
(ZCULL/PM/preemption-buffer bases are set through RM controls / ctxsw, not as user-class push
methods, so they are not a guest-reachable physical operand either.)

Not run: a GR/compute VIRTUAL control (would only re-confirm the VAS path the CE control already
shows); and whether a PRIVILEGED GR channel could use a physical operand — moot, since the method
does not exist in the class.

## Verdict per engine class

- **Copy engine (`0xC7B5`), physical LOCAL_FB operand:** **physical refused/faulted by hardware**
  (Xid 32) on a `USER` channel, with and without the DENY belt. **Cleared for forward-unknown** (CE
  physical FB operands).
- **Copy engine (`0xC7B5`), physical COHERENT_SYSMEM operand:** **refused/faulted** (Xid 32), both
  belt states. Cleared, with the sysmem-address caveat above (re-run with a known-good sysmem
  physical address to make it as strong as the FB arm).
- **Copy engine (`0xC7B5`), physical DESTINATION (LOCAL_FB, own page):** **refused/faulted** (Xid 32,
  `HCE_DBG1 0x218e`), both belt states. Honour would have written only the test's own page; it did
  not — **cleared**. A physical DST is refused exactly as a physical SRC.
- **Copy engine, second instance (COPY1, engine `0xa`):** physical SRC **refused** (Xid 32) on a
  `USER` channel, same as COPY0 — the gating is per-channel, not per-CE-instance. Cleared.
- **3D (`0xC997`), compute (`0xC9C0`/`0xC6C0`+QMD), video NVDEC/NVENC/OFA (`0xC9B0`/`0xC9B7`/`0xC9FA`):**
  **N/A by construction (reading)** — no physical-operand method in the class; every operand is a VA
  through the channel VAS. Nothing to honour. No HONOURED method on any engine.
- **Host/FIFO (`0xC56F`) GP_ENTRY / SEM_ADDR:** **N/A by construction** — no guest-settable physical
  operand in a push entry; the pushbuffer-fetch aperture is fixed by the channel's instance block.

## Recommendation for the forward-unknown proposal (recommend only — no ruling written)

1. **For the copy engine: measured-safe.** An unprivileged (`PRIVILEGE_USER`) host channel faults
   (Xid 32, channel RC) on a physical-aperture CE operand regardless of the DENY belt. A forwarded
   unknown entry that carries `SRC/DST_TYPE=PHYSICAL` cannot reach host physical memory on such a
   channel; the VAS bound (`the_three_channel_kinds.md` §4a) catches everything else. Keep
   `DENY_PHYSICAL_MODE_CE` on the forwarding channel — it is free and explicit — but it is **not**
   the load-bearing defence; the channel's `USER` level is.
2. **GR/compute/video (Windows D3D + compositor channels): reading-safe.** These classes have no
   physical-operand method — a forwarded unknown entry on a GR/compute/video channel cannot carry a
   physical address, because the class provides no bit for one; every operand resolves through the
   channel's VAS. So the forward-unknown route does not need a per-run clearance for them the way CE
   did; the containment is the VAS, and there is no physical escape hatch to close. (If a future
   chip adds a physical-mode method to a GR class, re-run this oracle for it — the oracle and the
   header enumeration are the check.)
3. **Physical DST is refused exactly as physical SRC**, so forwarding cannot turn a copy's
   destination into a host-physical write either. The DENY belt is free and explicit; the load-bearing
   defence is the channel's `USER` level plus the VAS bound.
4. **The fault is the designed mechanism** (`the_three_channel_kinds.md` §5.3: forward a deliberate
   fault). Xid 32 here is exactly the robust-channel RC the design wants; the §5.3 *named-sentinel*
   refinement still applies, so a deliberate refusal is distinguishable in `dmesg` from a real defect.

## Reproduce

```sh
# GPU-free build check (any box):
cargo build -p kf-harness --bin kf-phys-oracle
# On a GPU host (native), per arm; deny=1 is kayfabe's production channel flag:
KF_GATE_GPU=0 kf-phys-oracle virt 1          # rig control, must deliver
KF_GATE_GPU=0 kf-phys-oracle phys_fb 1       # production: physical FB on a USER channel
KF_GATE_GPU=0 kf-phys-oracle phys_fb 0       # belt off, still USER
KF_GATE_GPU=0 kf-phys-oracle phys_sysmem 1
KF_GATE_GPU=0 kf-phys-oracle phys_sysmem 0
KF_GATE_GPU=0 kf-phys-oracle phys_dst_fb 1    # physical DESTINATION into the test's OWN page
KF_GATE_GPU=0 kf-phys-oracle phys_dst_fb 0
KF_GATE_GPU=0 KF_PHYS_ENGINE=1 kf-phys-oracle phys_fb 0   # the second copy engine (COPY1)
# after each: dmesg | grep 'NVRM: Xid'  and  nvidia-smi   (a channel RC is a result, not a failure)
```

A physical-operand arm cannot be written for GR/compute/video: those classes expose no physical
method (see the reading section), so there is nothing to submit — the check there is the header
enumeration, not a run.
