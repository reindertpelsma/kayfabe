# USERD IOVA spike — can a separate one-page OS descriptor give the Passthrough twin an adoptable USERD?

**STATUS: ANSWERED, 2026-10-08. The hypothesis is NOT supported: a separate one-page OS descriptor
is refused with the same status as a page inside the 8 GiB descriptor, in every case run. The
falsifier fired. Recorded here (the relay doc `V3_USERD_RELAY.md` is on the `claude/windows-*`
branches, not on this base); fold the "For V3_USERD_RELAY.md" paragraph below into it, above its text,
when those branches merge.**

## Hypothesis and falsifier (stated before the run)

The falsifier below is in the doc comment of `crates/kf-harness/src/bin/kf-userd-iova.rs`, committed
(`b775a6be`) before the first run; this README was written after the run.

- **HYPOTHESIS (inferred, untested).** Windows' USERD page has a host DMA address above 2^40
  (`0x7ff9_f969_b000`, run61) only because it sits inside kayfabe's single 8 GiB guest-RAM OS
  descriptor, whose one contiguous IOVA block cannot fit below 4 GiB (Linux dma-iommu tries a 32-bit
  IOVA first for PCI devices, then falls back to the device DMA mask, which RM sets to 47 bits). A
  SEPARATE tiny OS descriptor over just the USERD page would get a low IOVA and pass
  `kchannelIsUserdAddrSizeValid_GA100` (`addr >> 32` fits 8 bits, `addr < 2^40`).
- **FALSIFIER.** A channel birth whose `hUserdMemory` is a separate one-page OS descriptor fails with
  the same invalid-address status (`NV_ERR_INVALID_ADDRESS`, 0x1e).

## Platform and method

`[measured, 2026-10-08, rev b4f2b7d1 (harness source = b775a6be)]` RTX 4070 (AD104, arch 0x4), host
driver 595.91.07, kernel 7.0.0-34-generic, 30 IOMMU groups `DMA-FQ` and 1 identity. The Windows
experiment VM was running on the same GPU (not touched; ~2.1 GB VRAM in use, the harness needs
~64 KiB). One process per case (`scripts/bench/userd_iova_spike.sh`, full output `matrix.log`):
a harness-owned memfd of 8 GiB, OS descriptors over it (`HostRm::alloc_os_descriptor`, the call
`guest_ram_object` makes), the page's GPU-visible address read through
`NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR`, then `birth_channel` (CE engine, GPFIFO in a VRAM object,
`hUserdMemory` = the descriptor). The channel is born and freed, never scheduled, no work submitted.

## Result — pass/fail matrix (`matrix.log`, 16 cases)

| case | memfd offset of the USERD page | descriptor order | page IOVA | birth |
|---|---|---|---|---|
| ctl_vram (rig control, USERD in VRAM) | - | - | - | **born** (USER) |
| (A) big, one 8 GiB descriptor | 0 | - | `0x7ffc00000000` | REFUSED 0x1e |
| (A) big | 1 GiB | - | `0x7ffc40000000` | REFUSED 0x1e |
| (A) big | 4 GiB + 1 MiB | - | `0x7ffb00101000` | REFUSED 0x1e |
| (A) big | ~7.9 GiB (`0x1e6666000`) | - | `0x7ffbe6667000` | REFUSED 0x1e |
| (B) small, no big descriptor at all | 1 GiB | alone | `0x7ffdfe8ef000` | REFUSED 0x1e |
| (B) small | 1 GiB | small BEFORE big | `0x7ffdfe8d7000` | REFUSED 0x1e |
| (B) small | 1 GiB | small AFTER big | `0x7ffaffff9000` | REFUSED 0x1e |
| (C) small, upper page | `0x1f0005000` | alone | `0x7ffdfe8d6000` | REFUSED 0x1e |
| (C) small | `0x1f0005000` | BEFORE big | `0x7ffdfe8d5000` | REFUSED 0x1e |
| (C) small | `0x1f0005000` | AFTER big | `0x7ffaffff3000` | REFUSED 0x1e |
| (C') small | `0x1e6666000` | BEFORE / AFTER big | `0x7ffdfe8ef000` / `0x7ffafffed000` | REFUSED 0x1e (both) |
| repeat of (A) 1 GiB, (B) 1 GiB before/after | 1 GiB | - | all `0x7ff...` | REFUSED 0x1e (all) |

Every sysmem page's IOVA is 47 bits (`0x7ffa...` to `0x7ffd...`), far above 2^40. A one-page
descriptor with no 8 GiB descriptor anywhere in the process is no different from one inside it, and
creation order makes no difference. Within the 8 GiB descriptor the IOVA is one contiguous block:
`0x7ffc00000000 + memfd offset` (cases at 0 and 1 GiB), so the block is placed at the top of the
47-bit space.

dmesg after the cases: `NVRM: ... Assertion failed: Address not valid [NV_ERR_INVALID_ADDRESS]
(0x0000001E) returned from kchannelCreateUserdMemDescBc_HAL(...) @ kernel_channel.c:2234` and
`... returned from _kchannelAllocOrDescribeInstMem(...) @ kernel_channel.c:899` (RM's assert print is
rate-limited: it shows for 5 of the 15 refusals, the status came back to the harness in all 15).
The `kchannelCreateUserdMemDesc_GV100: physical addr size ... is incorrect` line the Windows run
quoted was NOT in this host's dmesg for any case. No Xid appeared in any case (the last Xid in dmesg
is 20:08:14, from another agent's raw-client job, before this matrix started at 20:19); `nvidia-smi`
answered after every case and the GPU memory use did not change.

## Why — measured vs inferred

- **Measured** (`dmesg -T`, 2026-10-08 20:09:30): `nvidia 0000:01:00.0: Using 47-bit DMA addresses`.
  That is the notice dma-iommu prints when its 32-bit-first IOVA attempt for this device has failed
  and it stops trying; it was printed 10 minutes BEFORE the matrix, by other users of the GPU. So on
  this host the device was already in the "full DMA mask" mode for every allocation in this matrix.
- **Inferred** (from dma-iommu's code shape, not from a run): the switch is a ONE-WAY, PER-DEVICE
  latch, not a property of one big descriptor. A fresh device (no latch yet) would hand a lone
  one-page descriptor a sub-4-GiB IOVA, and the original hypothesis would hold in THAT window; this
  matrix cannot test that window (it would need the GPU's IOMMU state reset, i.e. a driver reload or
  host reboot, on the owner's live desktop GPU: not done). The falsifier as stated (a separate
  one-page descriptor still fails) fired on this host in every case.
- **Inferred**: any kayfabe VM trips the latch itself: its guest-RAM object is 8 GiB (one contiguous
  IOVA block, larger than the 4 GiB 32-bit space), and so does any other process mapping more than
  4 GiB for the GPU (a second VM, the desktop). A Passthrough design that depends on a low IOVA
  would work only until the first big mapping on the device, so it is not a design to build on.
- **Not measured**: that the 40-bit rule is the only gate (no positive control: no sysmem page with
  an IOVA below 2^40 was available on this host). The refusal status and the IOVA width are measured;
  "refused BECAUSE of the width" rests on `kernel_channel_gv100.c:211-219` as read.

## For V3_USERD_RELAY.md (to fold above its text when it lands)

> *Correction/answer (2026-10-08, `traces/userd_iova_spike_20261008`, rev b4f2b7d1):* a separate
> one-page OS descriptor over the guest's USERD page does NOT make the page adoptable. 16 channel
> births on the RTX 4070 / 595.91.07 host: the VRAM-USERD control was born; all 15 sysmem births
> (page inside the 8 GiB descriptor at four offsets; a lone one-page descriptor; a one-page descriptor
> made before or after the 8 GiB one; two different pages) were refused `NV_ERR_INVALID_ADDRESS`, every
> page at a 47-bit IOVA (`0x7ffa...`-`0x7ffd...`). The host's IOMMU path was already in full-mask mode
> (`Using 47-bit DMA addresses`, logged before the run). The relay stays the way a Windows Passthrough
> twin gets its USERD; no early per-pool descriptor is built. The only host-side ways to give RM a USERD
> address below 2^40 are outside kayfabe's unprivileged reach (an identity IOMMU domain for the GPU,
> option (ii), already out; a lower device DMA mask, which is RM's) and not pursued.
