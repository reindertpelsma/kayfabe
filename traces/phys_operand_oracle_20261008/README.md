# Physical-operand oracle — does an UNPRIVILEGED host channel honour a PHYSICAL operand?

**STATUS: RESEARCH, 2026-10-08. Recommendation only; no ruling written. Blocks the owner's
Windows channel-policy decision (`OWNER_RULINGS.md` §U.3, `docs/design/the_three_channel_kinds.md`
§3).**

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
| 3D `0xC997` / compute `0xC9C0` physical-target methods | — | — | — | — | **UNTESTED** (reading only) |
| host/FIFO `0xC56F` GP_ENTRY / SEM_ADDR | — | — | — | — | **N/A by construction** (no physical operand) |

The LOCAL_FB source was a VRAM object the test owns (`GET_SURFACE_PHYS_ATTR` → `addr=0x8130000
aperture=0` VIDMEM, a sane ~135 MiB FB offset), filled with the known pattern. DST was always a
VIRTUAL VRAM object the test owns — never a physical destination, never an address the test did not
allocate.

The four Xid lines (verbatim), all `HCE_DBG0 00000300` = the host-copy-engine `LAUNCH_DMA` (`0x300`)
that carried `SRC_TYPE=PHYSICAL`, channel RC'd:

```
NVRM: Xid (PCI:0000:01:00): 32, pid=2096908, name=kf-phys-oracle, channel 0x00000007 intr1 00000004 HCE_DBG0 00000300 HCE_DBG1 0000118e   # phys_fb deny=1
NVRM: Xid (PCI:0000:01:00): 32, pid=2097004, name=kf-phys-oracle, channel 0x00000007 intr1 00000004 HCE_DBG0 00000300 HCE_DBG1 0000118e   # phys_fb deny=0
NVRM: Xid (PCI:0000:01:00): 32, pid=2097498, name=kf-phys-oracle, channel 0x00000007 intr1 00000004 HCE_DBG0 00000300 HCE_DBG1 0000118e   # phys_sysmem deny=1
NVRM: Xid (PCI:0000:01:00): 32, pid=2097704, name=kf-phys-oracle, channel 0x00000007 intr1 00000004 HCE_DBG0 00000300 HCE_DBG1 0000118e   # phys_sysmem deny=0
```

Xid 32 is the PBDMA / host-CE pushbuffer error; here the engine rejected the physical-mode
`LAUNCH_DMA` on a `USER` channel. The host survived every RC: `nvidia-smi` healthy throughout, no
"fell off the bus", no fatal. Host Xid count went 86 → 90 (exactly the four arms) and stayed at 90.

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
- **3D/compute untested.** Those classes expose aperture `TARGET` fields on
  semaphore/notifier/report/zcull/PM methods, but exercising them needs a GR/compute context not
  built in this time-box. Inferred (not measured): the engine gates them at the same channel level
  the CE result shows. Needs its own arm before any conclusion.

## Verdict per engine class

- **Copy engine (`0xC7B5`), physical LOCAL_FB operand:** **physical refused/faulted by hardware**
  (Xid 32) on a `USER` channel, with and without the DENY belt. **Cleared for forward-unknown** (CE
  physical FB operands).
- **Copy engine (`0xC7B5`), physical COHERENT_SYSMEM operand:** **refused/faulted** (Xid 32), both
  belt states. Cleared, with the sysmem-address caveat above (re-run with a known-good sysmem
  physical address to make it as strong as the FB arm).
- **3D (`0xC997`) / compute (`0xC9C0`) physical-target methods:** **untested.** Not cleared.
- **Host/FIFO (`0xC56F`) GP_ENTRY / SEM_ADDR:** **N/A by construction** — no guest-settable physical
  operand in a push entry; the pushbuffer-fetch aperture is fixed by the channel's instance block.

## Recommendation for the forward-unknown proposal (recommend only — no ruling written)

1. **For the copy engine: measured-safe.** An unprivileged (`PRIVILEGE_USER`) host channel faults
   (Xid 32, channel RC) on a physical-aperture CE operand regardless of the DENY belt. A forwarded
   unknown entry that carries `SRC/DST_TYPE=PHYSICAL` cannot reach host physical memory on such a
   channel; the VAS bound (`the_three_channel_kinds.md` §4a) catches everything else. Keep
   `DENY_PHYSICAL_MODE_CE` on the forwarding channel — it is free and explicit — but it is **not**
   the load-bearing defence; the channel's `USER` level is.
2. **Do not open the route for GR/compute (or any engine with physical-aperture methods) yet.** Run
   this oracle per class first, or restrict forwarded entries to the copy engine.
3. **The fault is the designed mechanism** (`the_three_channel_kinds.md` §5.3: forward a deliberate
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
# after each: dmesg | grep 'NVRM: Xid'  and  nvidia-smi   (a channel RC is a result, not a failure)
```
