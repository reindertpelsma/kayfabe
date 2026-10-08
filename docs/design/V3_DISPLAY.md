# V3 display — a virtual NVIDIA display the stock driver drives, scanned out by kayfabe

**STATUS: LIVE, 2026-10-08 (later) — §8.18: the three defects of §8.17 are fixed and measured** at kf3
`2a20e699` on the trusted host: the early-frame dead display (a late copy given up at 2 s; 0/13 dead
after the fix, 7/7 before), the `x11-dispsw` twin at host 595.91.07 (Cinnamon starts normally), and
the GPU-copy rung at 595.91.07 (offered; GNOME imports it: `gpucopy=169` of `sent=170`). ⊘ The
paragraph below says "worked around in the launcher, not fixed" and "Cinnamon runs in fallback mode":
both are superseded by §8.18.

**STATUS: LIVE, 2026-10-08 — the interactive broker window, §8.17.** Branch
`claude/broker-interactive-20261008`, kf3 binary `0e64a960`, broker nvkvm-pv `badf2d7`, trusted host
(RTX 4070, host 595.91.07, GNOME Wayland). One command (`scripts/bench/display/interactive.sh`) shows a
Linux guest from grub to the NVIDIA X desktop in a broker window; key, button, absolute and relative
input reach the guest (run p4); two clean reboots in one QEMU come back to the desktop. Found: a kf3
first-scanout-copy stall within ~2 s of QEMU start (worked around in the launcher, not fixed), and on
host 595.91.07 the `x11-dispsw` twin is refused, so Cinnamon runs in fallback mode. The 2026-10-06
status below is unchanged.

**STATUS: LIVE, 2026-10-06 — bounded SDR implementation update.** Branch
`codex/sdr-lut-20261006`, product `2aa8b92de6c6ab158f9bc8e788be792408a074a3`,
adds opt-in GPU ILUT/OLUT processing and truthful DIRECT10 declarations.
Linux AD104/open580.159.04 applies a nonidentity KMS gamma ramp with exact
agreement across 6,220,800 RGB components; removing it restores the original
frame. Windows still fails before submitting any display methods, with a fresh
saved display-buffer precondition failure pointing to absent TMO.
[Scope, lifetime/completion policy and evidence](V3_SDR_COLOR.md).
Default behavior and master promotion remain unchanged; this does not qualify
HDR, active TMO, arbitrary colour pipelines or Windows compatibility.

**Historical diagnostic below, superseded for opt-in SDR by the update above.**

**STATUS: LIVE, 2026-10-06 — color support clarification.** The verification
claims below remain scoped to their named lanes; they do not qualify arbitrary
LUT/color transforms. Linux guest open580.159.04 on AD104, product `8bbcd7f3`
(repaired Windows branch plus bounded diagnostic), runs Weston13 and Sway1.9
with NVIDIA rendering. The fixed SDR scene reaches the virtual console
pixel-exact. OGKM binds real default identity ILUT/OLUT surfaces in that path.
A custom KMS output gamma table is accepted but leaves console pixels unchanged.
In a separate guest-only KMS-refusal control, Sway keeps the image displayed and
the gamma-control client receives failure; it supplies no shader replacement.
Evidence and limitations: [`traces/linux_color_20261006/README.md`](../../traces/linux_color_20261006/README.md).
These results allow investigation of a bounded identity SDR subset; they do
not prove its general equivalence or Windows compatibility. Optional color
features may be withheld/refused under OWNER_RULINGS §H. Guest-local LUT state
is distinct from the physical host monitor's settings, and is not inherently
privileged. Constructor declarations and successful UPDATEs alone are not color
implementation evidence. No product capability/refusal policy is changed by
this diagnostic.

**STATUS: VERIFIED, 2026-10-04 — candidate 2 product `9d82f259`, KF3 ABI 18:**
master candidate 1 plus broker/maxfps, HMP AioContext fix and asynchronous console
cursor update. Exact-source GPU merge bar passes; host apps 71/71 and guest
61/65 + 6/6 probes match candidate 1, including the four known failures rerun
alone. Display verification, including D4's actual present cadence, is complete
with the explicit limitations in §8.16 and
`traces/v3_candidates/cand2_20261004/README.md`. Product CI and full slow dispatch
pass; final docs/evidence-head CI is required before promotion. `x11-dispsw`
remains off by default under §N. Historical branch statuses below are superseded.

> **STATUS: VERIFIED, 2026-10-04 — candidate 1 at `0ac157b2`.** The B5 context-DMA
> latch fix (`c1ca7945`) passed four full unload runs (14/14 arms each) on GA106
> box 54049598, with B0/B1, X11 A/B, the merge bar and the existing app baseline
> also passing. Evidence recovered after the earlier handoff:
> `traces/v3_candidates/cand1_20261004/`. §4.11.13's pending hardware re-test is
> superseded. Broker/max-fps work remains on its own branches.
> The statuses below are dated history where they differ.


**Historical broker/maxfps branch status (before candidate 2):**

> **STATUS ADDENDUM 5, 2026-10-04 (branch `v3-maxfps`, cut from `v3-broker` `82f98f42`) —
> `display-max-fps` (`OWNER_RULINGS.md` §M and its decisions D1–D5) is BUILT IN CODE and GPU-free
> tested locally; NOTHING of it has run on a box.** Every head's emulated vblank tick is capped
> (unset: 75 Hz with the EDID byte-identical); tearing flips count against the cap (D1); copies
> made without a flip happen at the console head's tick, only while someone watches, and are sent
> only when a GPU checksum of the composed frame changed — a `screendump` asks for an on-demand
> copy (D2); values above 75 are refused by name (D3). The status line and a line of its own carry
> `fps[...]` with per-path rates and `over`. **KF3 ABI 16** (the registry is in `kf3.h`). The X11
> half of §M (D4: the cap paces X11 vsync through the guest's own vblank consumer) needs
> `x11-dispsw`, which this branch does not carry, and a box run — §8.16.

> **STATUS ADDENDUM 4, 2026-10-04 (later) — the two reviews of the console cursor and the
> `badf2d7` relay, fixed in code on `v3-broker`, GPU-free-tested locally and in CI; NOT run on a
> box.** (1) The console cursor (§8.13) now follows the frame the console SHOWS (no image beside a
> frame that still composes one; no hide before the frame carries it), is paced to one DEFINE per
> 16 ms, never moves the pointer except in hover under an absolute pointer (GTK warps the HOST
> pointer otherwise), retries what QEMU did not apply, and hands QEMU PREMULTIPLIED pixels — what
> VNC's Cursor With Alpha encoding carries (the box's VNC digest match was graded under kayfabe's
> own straight convention). (2) The relay (§8.15): a dma-buf commit refused by name no longer
> counts toward the silent-drop detector; the verdict table evicts volunteered rows before the
> relay's own; a descriptor the check refuses backs its rung off and the frame takes the next one
> (it used to refuse every frame on that rung); the check runs before anything is spent and once
> per backing; the HELLO line no longer claims "no /dev/udmabuf" before the first frame; the status
> line carries `dmabuf_trips`, `carrier_refused`, `carrier_unchecked`, `formats_unasked`, graded
> by `broker_lane.sh` (`BRK_COUNTER_GRADE`). (3) The composition word's log line is bounded (§8.14).
> **Box checks to re-run:** `BRK_VNC_HOVER`/`BRK_VNC_XTERM`/`BRK_VNC_GRAB` (§8.13 — the pixels,
> the grader and the grab hide all changed), the hover/grab/ungrab regressions with a VNC client
> attached (one cursor at each transition), and one GPU-copy run for `BRK_COUNTER_GRADE` and the
> rung lines (§8.15). The §8.12 host-pointer path itself is unchanged.

> **STATUS ADDENDUM 3, 2026-10-04 — built in code on `v3-broker`, GPU-free-tested locally and in CI;
> NOTHING of it has run on a box.** (1) The relay handles the six behaviours of nvkvm-pv's broker
> at `badf2d7` (wire unchanged; §8.15) — run locally against that REAL broker, including one that
> cannot read `/proc/self/fdinfo`. (2) The compose kernel's XOR blend (`OWNER_RULINGS.md` §O; §8.14):
> an XOR cursor is now composed, visibly, in every mode — its semantics are a DEFINITION (NVKMS never
> programs XOR), and the committed hand-written PTX is executed on the host against a CPU reference.
> (3) QEMU's own console gets the guest cursor through its cursor API while a cursor-capable broker
> hovers (§8.13) — **a coordinator decision the owner may revisit**; without a broker the console
> keeps the composed cursor. (4) The cursor's composition word is logged once per change beside an
> alpha census of its pixels, so the next box run settles §8.12's premultiplied/straight question
> (§8.14). KF3 ABI stays 12. Evidence of the local runs: `traces/v3_display/broker_20261004/`.

> ⊘ **CORRECTED 2026-10-04 (§8.16): "`v3-dispsw-exp` takes 13 when it merges", below, is wrong** —
> 13 is that branch's own number, already in box binaries; a merge takes a NEW number. The registry
> (`qemu/hw/misc/kf3/kf3.h`): 11 master, 12 `v3-broker` (and `v3-windows`), 13 `v3-dispsw-exp`, 14
> reserved (broker-on-13, `v3-cand-1`), 15 `v3-viommu`, 16 `v3-maxfps`.

> ⊘ **MERGED 2026-10-03 — `v3-broker` took `master` (the boot display, KF3 ABI 11).** The broker
> branch had numbered its own steps KF3 ABI 11 (3a), 12 (3c) and 13 (the GPU-copy rung); after the
> merge the whole broker surface is **ONE bump above master's 11: KF3 ABI 12** (`kf3_realize(…,
> display, gop, display_broker, …)`; `v3-dispsw-exp` takes 13 when it merges). Where the text below
> says ABI 13 for the broker, read 12; its "ABI 11"/"ABI 12" for 3a/3c are the same bump. The 3d
> cursor layer is composed only on an ARMED composition (never on the boot, preserved or blank
> picture), and its layer marker moved to `u32::MAX - 1` because the boot layer took `u32::MAX`.

> ⊘ **CORRECTED 2026-10-03, later the same day (the adversarial review of `v3-broker`; the fixes are
> on the same branch, nothing run on a box):** the last sentence below no longer holds. The console's
> `ui_info` hook (3c resize) is now installed **only with `display-broker` set**, so with it unset
> GTK and VNC never re-mode the guest and the console path is M2's — except the 3d cursor layer,
> which still composes into the console (a guest pointer now shows on VNC and in a screendump). The
> broker peer check no longer trusts the owner of the socket's directory, and reads QEMU's
> effective uid at each connect. All review findings and their fixes: §8, the correction above its
> STATUS line.

> **STATUS ADDENDUM 2, 2026-10-03 (night):** the hover-mode host cursor (`OWNER_RULINGS.md` §O) is
> built in code on `v3-broker`, GPU-free-tested locally and in CI: with a `CAP_CURSOR` broker that
> is not grabbed, kf3 sends the guest's cursor image as `CMD_CURSOR` and stops composing it; under
> grab, and for an XOR cursor, it composes as before (§8.12). ⊘ Since run on box 54032077
> (2026-10-03, kf3 at `c38032f3`, broker `9cb736f`): hover, hide and grab graded against the guest's
> own cursor, the GPU-copy rung (rung 0) carrying a KDE/X11 desktop on the NVIDIA DDX, the fallbacks,
> E1 for all three slot attribute sets, E2's explicit-yes control, E3 — §8.12, evidence in
> `traces/v3_display/broker_20261003/`.

> **STATUS ADDENDUM, later on 2026-10-03:** the broker's GPU-copy rung (`OWNER_RULINGS.md` §L) is
> built in code, GPU-free-tested, on the same branch — kayfabe's own VRAM frame slots, a block-linear
> dma-buf for a compositor on the same GPU, `display-broker-vram=auto|on|off`, KF3 ABI 13 (§8.11;
> ⊘ 12 since the merge with master, the note at the top).
> Nothing of it has run on a GPU.

> **STATUS: DISPLAY STEP 3 BUILT IN CODE, GPU-FREE — 2026-10-03 (branch `v3-broker`; nothing run on
> a box).** All four sub-steps are in code: 3a (frames + input), 3b (reconnect, pacing), 3d (the
> head's cursor composed as the top layer) and 3c (resize: authored EDID + a hotplug the register
> drainer posts). The VMM-agnostic crate `kf-broker`, its OS doors in `kf-linux-raw`, the kf3 glue
> (KF3 ABI 12, properties `display-broker` / `display-broker-uid`, the console's `ui_info` hook),
> `kf_disp` (cursor, EDID, the internal hotplug state), `kf-abi` (the LIST `POST_EVENT`) and a narrow
> hotplug-registration seat in `kf-rm`. With `display-broker` unset nothing changes for the broker;
> the cursor layer and the resize hook apply to the console (VNC/GTK) too. Design, deviations, local
> runs and the pending box tests: **§8**.
> **STATUS 2026-10-03 (late) — display step 1 (the boot display): BUILT on branch `v3-gop` (both halves
> plus the owner's §K ruling), nothing run on a GPU box.** `v3-gop` carries `v3-gop-rom` and `v3-gop-kf3`
> squashed onto `master` with every finding of their review fixed (§4.11.12's ⊘ block): **no compiled
> binary is committed** — `crates/kf-gop-image/build.rs` builds `firmware/kf-gop` during the ordinary
> cargo build for `<arch>-unknown-uefi` and embeds it; the ROM is arch-neutral (`EfiMachineType` from the
> PE; x86 port I/O behind `cfg(target_arch)`; framebuffer writes are aligned volatile 32-bit stores; CI's
> `aarch64` job builds the driver for `aarch64-unknown-uefi`). First box step: still **B0a** (§4.11.9).
>
> (superseded the same day, kept as written) **STATUS 2026-10-03 (evening) — display step 1 (the boot display): BOTH HALVES BUILT, nothing run on a
> GPU box.** The firmware, the ROM container and the local stand-in on `v3-gop-rom` (§4.11.6–§4.11.8); the
> kf3 integration on `v3-gop-kf3` (§4.11.12): the ROM BAR (KF3 ABI 11), the BAR1 seed, the boot layer and
> fn 65's console region, all behind the device property `gop` (default off; with it off every path is
> today's, unit-tested). CI green at `37740a4b` (run 37132057723). First box step: **B0a** (§4.11.9).
> ⊘ Open from the owner's ruling of the same day (`OWNER_RULINGS.md` §K): the GOP blob is still a
> committed binary — build.rs is to compile it instead (§4.11.6).
>
> (superseded the same day, kept as written) **STATUS 2026-10-03 — display step 1 (the boot display's GOP
> option ROM): first half BUILT on branch `v3-gop-rom` — the firmware, the ROM container and a local
> stand-in test (§4.11, its own STATUS line). The kf3 integration is the second half. The ⊘ notes below
> dated 2026-10-03 fold that design's corrections into the older text.**

> **NEXT — owner direction 2026-10-01 (design only, nothing built): (1) a STOCK guest display with no
> guest-side tweaks, then (2) a VMM-agnostic display BROKER instead of QEMU's UI — both before Windows.**
> What the M1–M3 lanes still change inside the guest (`scripts/bench/display/`), and what each needs:
> - `modprobe nvidia-drm modeset=1 fbdev=1` by hand after boot → test a packaged driver loading at boot
>   (Ubuntu's `modprobe.d` already sets `modeset=1`); the bench loads the driver over ssh.
> - ⊘ **CORRECTED 2026-10-03 (the GOP design's review) — the next bullet's premise is wrong on today's
>   bench.** With SeaBIOS and `-vga none` the guest kernel already makes kf3 the boot VGA device
>   (`traces/v3_display/m2c_20260930/run_m2c_serial.log.gz:289`, *"vgaarb: setting as boot VGA device"*;
>   `run_m2c_dmesg_after.log:56`, *"vgaarb: deactivate vga console"*, printed only for `vga_default`,
>   `drivers/pci/vgaarb.c:171-173`), and Xorg already marks it primary
>   (`traces/v3_display/m3b_20260930/hook/Xorg.0.log:57`: `PCI:*(0@0:2:0) … BIOS @ …/131072`, the 0xC0000
>   shadow of `arch/x86/pci/fixup.c:381-397`). The BusID pin most likely came over from nvkvm-pv, where
>   QEMU's stdvga was primary (`nvkvm-pv docs/internal/mint-guest-desktop.md:671-676`). ⇒ Test Xorg
>   without the pin first (box test **B0a**, §4.11.9, no build). The ROM is still needed — Windows, any
>   picture before nvidia-drm, OVMF guests, guests with a second VGA device (§4.11.1). No legacy VGA path
>   for the release (§4.11.11).
> - X11's `xorg.conf` pins the BusID: the VM runs `-vga none` and kf3 shows no firmware framebuffer, so no
>   device is `boot_vga` and Xorg cannot choose (Wayland enumerates DRM and needs no pin) → **kf3 must be
>   the boot display**: a UEFI GOP (and a legacy VGA path) over a kf-disp linear framebuffer, handed over
>   to the NVIDIA driver when it loads. ★ Windows needs this regardless: its installer, boot and safe mode
>   run on the GOP framebuffer before `nvlddmkm` starts. The largest item (~1–2 weeks, estimate).
> - ⊘ **CORRECTED 2026-10-03 — two details of the next bullet** (the design and its review: §4.11):
>   - **Not `romfile=`, and not EDK2/uefi-rs.** PCIR must carry the host die's ids and class and the
>     descriptor is per VM, so kf3 ships one constant PE (`firmware/kf-gop`, Rust, zero dependencies) and
>     wraps it into a ROM per host at realize (`crates/kf-oprom`, §4.11.6). QEMU's `pci_patch_ids` would
>     also corrupt an EFI-only ROM wired as a class default (it rewrites byte 6, inside `EfiSignature`).
>   - **There is no fake-GSP answer for `uefiScanoutSurfaceSizeInMB`.** It is a CPU-RM `pGpu` field the
>     open code only zeroes (`ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:1019`,
>     `:1271`), CPU-RM reports it itself (`kern_mem_sys_ctrl.c:728-732`), and its consumer is compiled
>     for Windows only (`kern_bus_gm107.c:895-896`). What must agree with the ROM is the FB layout and the
>     region table fn 65 serves (§4.11.4).
> - ★ **How kf3 becomes the boot display — the owner's 2026-09-10 design (from the session; first written
>   down here 2026-10-02).** On real cards the PCI expansion ROM (the VBIOS) carries a UEFI GOP driver
>   that programs the display; the firmware's linear framebuffer is a BAR1 range (not PRAMIN), and GOP
>   itself is a firmware software interface in system RAM, not a BAR. Owner: *"why do we need [a vendor]
>   UEFI image … since the GPU is already initialized in kayfabe, the only thing we have to compile in
>   there as UEFI image is to put the structure to tell where"* the framebuffer is. ⇒ kf3 ships its OWN
>   small option ROM (QEMU `romfile=`): an EFI GOP driver (EDK2 or `uefi-rs`) that publishes one or a few
>   modes with `FrameBufferBase` in kf3's BAR1, software `Blt` over it, and no hardware programming —
>   kf-disp already owns the display model and scans that BAR1 range out. OVMF binds it to the device;
>   bootloader, efifb/simpledrm and Windows' boot screen draw into it; the NVIDIA driver takes over when it
>   loads. Legacy VGA (SeaBIOS) is optional. ⚠ The handover: Windows RM sizes a BAR1 "smooth transition"
>   from `uefiScanoutSurfaceSizeInMB` (`bSmoothTransitionEnabled = (uefiScanoutSurfaceSizeInMB != 0) &&
>   RMCFG_FEATURE_PLATFORM_WINDOWS`, `kern_bus_gm107.c:889`, found 2026-09-20) — the fake GSP's answer for it
>   must agree with what the option ROM set up.
> - X11 desktops need `GF100_DISP_SW` (owner choice A/B, `STATUS_AND_HANDOFF.md` §0). ⊘ *2026-10-03
>   (later): option A exists as the default-off experiment `x11-dispsw` — the note below the release-focus
>   block.* ★ *2026-10-03 (box): with it on, Cinnamon X11 and X11 vkcube work on the RTX 3060
>   (`traces/v3_display/dispsw_20261003/`); the owner's ruling is still open.*
> - The lightdm autologin is a bench convenience, not a display requirement.
> - Leftovers for daily use: cursor plane, scaled windows, 16-bit/YUV surfaces, mode lists and
>   resize (hotplug with a new EDID when the broker's window changes size).
> **Broker:** adopt nvkvm-pv's display broker protocol (`nvkvm-pv docs/reference/broker-protocol.md`,
> design + threat model `docs/internal/broker-design.md`): the VMM holds one unix socket; scanout buffers
> cross as dma-buf fds (`ATTACH`, `SCM_RIGHTS`) and come back as `RELEASE`; the broker sends `SURFACE`
> (window size — drives resize), keyboard/pointer/focus input, and capability bits; input never blocks
> on rendering. kayfabe's side: export kf-disp's scanout surfaces (host GPU memory) as dma-bufs through
> the host driver's dma-buf exporter (⊘ **superseded 2026-10-03 by `OWNER_RULINGS.md` §L**: never the
> guest's surfaces — kf-disp copies each finished frame GPU→GPU into a VRAM object kayfabe allocated,
> and exports that, §8.11), and inject the broker's input through the VMM's input device (the
> one VMM-specific part). It replaces the QEMU console as the product path (the console stays for tests
> and screendumps), works for VMMs with no display stack, and keeps the display-server connection out
> of the VMM (privilege separation, the broker's original reason).
>
> **2026-10-03 — the display path is the release focus (owner; `OWNER_RULINGS.md` §I).** The plan,
> with that day's review:
> - **Broker: reuse nvkvm-pv's broker process and protocol unchanged.**
>   - Port its QEMU-side relay (`nvkvm-pv src/qemu/nvkvm_display_relay.c`) into the kf3 device,
>     including input through `qemu_input_queue_*`. No QEMU patch is then needed.
>   - **Frames.** The broker accepts only dma-bufs (`ATTACH`). First send the existing de-tiled copy
>     (§4.6) as a memfd-backed udmabuf, linear XRGB8888; this needs `/dev/udmabuf`. Zero-copy export
>     with NVIDIA's modifier comes later, and works only for a host desktop on the same NVIDIA GPU.
>     ⊘ **SUPERSEDED the same day by `OWNER_RULINGS.md` §L:** no guest surface is ever exported; the
>     same-GPU path is a GPU→GPU copy into a VRAM frame object kayfabe owns (§8.11).
>   - Keep at least two copy targets, and reuse one only after the broker's `RELEASE`.
>   - A `SURFACE` resize sends a new EDID plus a hotplug event (§4.7).
>   - nvkvm-pv is Apache-2.0 and the owner's, so its code can come under kayfabe's dual license.
> - **The GOP framebuffer must lie inside a kf3 BAR.**
>   - Linux ties the firmware framebuffer to the PCI device whose BAR contains it. That device's
>     driver evicts it (`aperture_remove_conflicting_pci_devices`, `drivers/video/aperture.c`).
>   - ⊘ *CORRECTED 2026-10-03:* the next sub-bullet's last sentence. Xorg is not missing a boot VGA
>     device on today's SeaBIOS bench (the correction at the top). The firmware-default rule matters
>     under OVMF and with a second VGA device; the local stand-in shows it overriding a legacy device
>     (2026-10-03, `3dd574e5`, arm `linux_two_vga`, §4.11.8).
>   - The VGA arbiter picks the boot device by the same test (`vga_is_firmware_default`,
>     `drivers/pci/vgaarb.c:566-569`, Linux 7.1). That is what fixes Xorg's missing boot VGA device.
>   - kf-disp keeps scanning out the GOP region until the guest's first modeset, so the handover shows
>     no black flash.
> - **Speed.**
>   - BAR1 writes are write-combined, and boot consoles only write (simpledrm keeps a shadow buffer).
>     A BAR framebuffer therefore costs what it costs on a real card, with no VM exits.
>   - Reads are the slow direction (~48 MiB/s; memory note of 2026-09-11, bare-metal RTX 3060), and
>     scanout never uses them.
>   - A framebuffer in plain RAM is rejected. It breaks the ownership above, and the handover expects
>     VRAM (BAR1 = real VRAM views, owner 2026-09-14).
>   - ⊘ *CORRECTED 2026-10-03:* the next sub-bullet is withdrawn. Host RAM behind the guest's BAR1
>     framebuffer is sysmem standing in for vidmem (`THE_CONSTRAINTS.md:120-125`, items 18 and 22) and
>     breaks the FB-0 identity RM relies on (§4.11.2). It is not a fallback the design may take; it would
>     need an explicit owner ruling (`OWNER_RULINGS.md` A.11).
>   - Optional, only if boot is found to be slow: back the GOP region with host RAM during boot, and
>     move it into VRAM before the NVIDIA driver touches it through the GPU. ⚠ That RAM must be its
>     own memfd, never the window's scratch: since 2026-10-03 scratch is one small tile repeated
>     across the window (2 MiB for a BAR1 up to 2 GiB), so a framebuffer left on it would alias
>     itself every tile (`V3_P4_PORT_MAP.md` Q3). Retiring the boot framebuffer's store view
>     (`traces/v3_design_review_20261003/`) may return the part no guest placement covers to scratch
>     only because nothing reads that part afterwards.
> - **Driver unload.**
>   - ⊘ *SCOPED 2026-10-03:* the next sub-bullet holds only on guest kernels that export `screen_info`
>     (the bench's noble 6.8 does). On Linux 7.x nvidia.ko's console detection takes the BAR1-child
>     path, which gives width 0: NVKMS imports no console, sends no `NOTIFY_CONSOLE_DISABLED`, and has
>     nothing to restore (§4.11.3). Bare metal behaves the same.
>   - NVIDIA's modeset driver restores the boot console (`nvEvoRestoreConsole`,
>     `ogkm-580: src/nvidia-modeset/src/nvkms-console-restore.c:756`). It finds the console through
>     `NV0080_CTRL_CMD_OS_UNIX_VT_GET_FB_INFO`. kf-disp must follow that restore.
>   - The Linux console does not come back. nvidia-drm's eviction calls `sysfb_disable()`, which
>     unregisters the firmware framebuffer device and sets a flag nothing clears
>     (`drivers/firmware/sysfb.c:67-80`, `:156`). Bare metal behaves the same; a small guest module
>     could re-register the device.
>   - Windows hands the framebuffer to its basic display driver when the NVIDIA driver stops
>     (`DxgkDdiStopDeviceAndReleasePostDisplayOwnership`). kf-disp must keep scanning out what the
>     driver left programmed; that keeps the screen alive during Windows driver updates.
> - ⊘ **CORRECTED 2026-10-03 (the broker design's review):** the cursor was **not** composed — the
>   composition carried window layers only (`kf-disp/src/engine.rs` `Composition`; the M3 status block
>   says so), so "(built, §4.4–§4.6)" below overstated the code. Both broker backends hide the host
>   pointer while a guest frame shows, so without cursor composition the broker window shows no
>   pointer at all. ★ Display step **3d** (same day, §8.6) composes it in code; not yet on a box.
> - **The frame to capture** is, per head, the window surface latched at vblank plus the cursor
>   (built, §4.4–§4.6). Write the flip-completion notifier only after the frame was taken, and with
>   the broker never overwrite a copy before `RELEASE`.
> - **Order:**
>   1. the GOP option ROM; ★ *first half built 2026-10-03 (branch `v3-gop-rom`): the firmware, the
>      container and the local stand-in (§4.11); the kf3 integration is the second half;*
>   2. a stock guest display with no tweaks;
>   3. the broker;
>   4. the unload tests.
>
>   ⊘ *Corrected 2026-10-03 (later): the recommendation is now **A, guarded**, with B as the fallback
>   (`docs/OWNER_QUESTIONS_2026-10-03.md` item 2), and A is built as a default-off experiment so the owner
>   can rule on box data — the `x11-dispsw` note directly below. The sentence it corrects:*
>   X11 still waits on the `GF100_DISP_SW` choice (A/B; recommendation B).

> **★ 2026-10-03 (review fixes, box re-run) — THE TWIN NOW CARRIES THE GUEST'S SOFTWARE CLASSID OR IS
> REFUSED BY NAME; LIVE TWINS ARE CAPPED; THE NO-KERNEL-MAPPING BOUND IS PINNED WHERE IT IS ENFORCED**
> (branch `v3-dispsw-exp` at `d84086df`; the A/B pair re-run on vast 54044296, runs 9-10 of
> `traces/v3_display/dispsw_20261003/`: the outcome holds, every twin's number read back equal to the
> guest's, caps not hit; probe runs 11-13 show the classID fix on hardware, with the pre-fix binary as
> its negative control). It qualifies the box block directly below; where they differ, this one
> stands. Still default-off, still the owner's call.
> - **The FIFO software classID** (review MEDIUM). The guest's CPU-RM numbers every `ENG_SW` child of
>   a channel from that channel's own 16-bit counter BEFORE it RPCs the alloc (`kchannelRegisterChild`,
>   `ogkm-580: kernel_channel.c:3408-3453`; never rolled back), its client reads the number from that
>   same CPU-RM (`NV906F_CTRL_GET_CLASS_ENGINEID`, `kernel_channel.c:2950-2970`) and writes it into
>   `SET_OBJECT` (`kernel_channel_gm107.c:72-82`) — on a channel that runs as a HOST twin, whose RM
>   numbers its own children. The alloc RPC carries no number (`rpc.c:11140-11230`). So the twin served
>   the guest's `SET_OBJECT` only while every guest `ENG_SW` registration on the channel had exactly one
>   host counterpart, which nothing checked (runs 1-8 never read a number back). Now
>   (`crates/kf-qemu/src/dispsw.rs`): the plane mirrors the guest's numbering — every display-SW alloc
>   and every other `ENG_SW` channel class the guest sends (`0x9074`, `0x5080`, `0x007d`, `0xc076`,
>   observed by `kf_rm::chanlink` as `SoftwareObject`), accepted or refused — reads the host's number
>   back after each twin alloc (`HostRm::disp_sw_class_id`), and keeps the object, repays a gap with
>   throwaway host objects (at most 16 per act, `repaid=`), or refuses the alloc `NOT_SUPPORTED` by
>   name (`id_refused=`; a twin AHEAD of the guest, an unreadable number, a gap of more than 16 — repaid
>   16 at a time across the guest's next allocs). Tests: `kf-qemu` `dispsw::tests`,
>   `kf-rm` `chanlink::tests::other_software_classes_are_observed_with_x11_dispsw_and_otherwise_untouched`.
> - ⊘ **The one slip no physical RM can see — a known limit of option A.** A display-SW constructor that
>   fails AFTER its channel numbered it (`logicalHeadId` past the heads, a `displayMask` outside the
>   lit mask, a refused query: `disp_sw.c:74-98`) never sends the alloc, and its query names no channel;
>   that channel's guest numbering then runs one past ours. `kf-rm`'s display link counts it
>   (`DispSwPairing`; log `kf-rm: display: x11-dispsw: a GF100_DISP_SW constructor's query was not
>   followed by its alloc`). Its effect is confined to that guest channel: its later display-SW
>   `SET_OBJECT`s name a number its own twin lacks (host Xid 32 on its own twin, the m3c signature;
>   run 13 below forces it: 26 Xid 32 on the forcing client's channels, `unpaired=112` counted). A
>   real GSP-RM is sent the same RPC with no number, so it is inferred — not shown — to share the limit.
> - ⊘ **2026-10-03 (later, third review LOW) — the readback is HOST CPU-RM's number, and the host-side
>   twin of the slip above.** Every "= the guest's" in this block and in runs 9-13 is the number host
>   **CPU-RM** gave: `NV906F_CTRL_GET_CLASS_ENGINEID` has flags `0x10008` (no `ROUTE_TO_PHYSICAL`,
>   `ogkm-580: g_kernel_channel_nvoc.c:251-258`) and CPU-RM answers it from its own `pObject->classID`
>   (`kernel_channel.c:2966`, `kernel_channel_gm107.c:72-82`). The guest's `SET_OBJECT` on the twin is
>   serviced by host **GSP**, which numbers the twin's `ENG_SW` children from its own counter (the alloc
>   RPC carries no number). The two agree while every host display-SW alloc succeeds — `host_refused=0`
>   in every run, so GSP's number is inferred equal, not read. A host alloc that fails AFTER CPU-RM
>   registered the object (`dispswConstruct`'s query RPC, `disp_sw.c:73-80`, or the alloc RPC to GSP,
>   `alloc_free.c:916-927`) moves CPU-RM's counter and not GSP's, and the next twin's readback would then
>   say "equal" while GSP holds the number before. ⇒ Now (`crates/kf-qemu/src/dispsw.rs`,
>   `twin_watched`): after ANY refused host display-SW alloc on a twin — the first try, a repayment's
>   throwaway, or the realloc after repaying — the plane marks that twin, and every later display-SW
>   alloc on it is refused `NOT_SUPPORTED` by name with no host call (`after_host_refused=`), for the
>   twin's life. Tests: `kf-qemu` `dispsw::tests::the_readback_alone_keeps_a_twin_whose_gsp_number_is_one_behind`
>   (the known-positive: a model host whose CPU-RM and GSP number separately; without the mark the
>   twin is kept with GSP one behind), `a_refused_host_alloc_marks_the_twin`,
>   `a_marked_twin_is_refused_with_no_host_call`, and
>   `chan::dispsw_tests::the_host_refusal_mark_lands_on_its_own_twin_only`. Not run on hardware: it
>   needs a transient host GSP failure.
> - **Caps** (review MEDIUM): at most **16 live display-SW twins per channel and 1024 per VM**; past
>   either the alloc is refused `NV_ERR_INSUFFICIENT_RESOURCES` by name with no host call (`capped=`).
>   Chosen from runs 2-8: at most 4 live per channel (one per head of the 4-head virtual display) and 20
>   per VM (60-84 created per boot) — 4× and ~50×, while bounding one guest's host RM objects and host
>   RM's per-alloc child scan (`kernel_channel.c:3435-3446`). A cap refusal takes a guest number and no
>   host one; the next twin on that channel repays it. ⊘ *2026-10-03 (later, third review LOW): the
>   counts the caps compare had no test (a body of zeros kept CI green); they are now one function
>   over the plane's channel map (`chan.rs` `dispsw_live`), tested with two clients, three channels and
>   one channel freed, and fed into the cap
>   (`chan::dispsw_tests::the_live_count_sums_every_client_and_drops_a_freed_channel`).*
> - **The undo** (review LOW, the duplicate pre-check saw only twinned handles): `kf_gsp::Deferred` takes
>   an undo (`on_orphaned`), run once by `release_held` when an act succeeded and another link refused
>   the reply. A display-SW alloc whose handle names a NON-twinned guest object is refused by the object
>   seat after the act; the undo withdraws exactly the twin that act kept (`withdrawn=`). ⊘ *2026-10-03
>   (later): its body and its wiring are now functions with tests
>   (`chan::dispsw_tests::the_undo_withdraws_the_kept_twin_and_frees_it_once`,
>   `an_orphaned_display_sw_act_queues_exactly_one_withdraw`), as is the other-`ENG_SW` mirror step
>   (`another_eng_sw_object_advances_only_its_own_channels_mirror`). Still not covered by a test: the
>   one-line call sites inside `ChanPlane` (building a plane needs a host RM session).*
> - **The no-kernel-mapping bound, pinned where it is enforced** (review LOW): `kf-host`'s one NVOS46
>   flags builder, `nvos46_map_flags`, CLEARS `NVOS46_FLAGS_KERNEL_MAPPING_ENABLE` whatever its callers
>   pass (`the_kernel_mapping_bit_is_cleared_whatever_the_caller_sets` feeds every bit), and a source
>   gate keeps it the only NVOS46 literal in `crates/kf-*` and the only code naming the bit
>   (`only_the_one_builder_can_name_the_kernel_mapping_bit`). The old test only tried today's callers'
>   bits; it is gone.
> - **Status line:** `dispsw[twins= live= host_refused= no_twin= capped= id_refused= repaid= withdrawn=
>   other_sw= free_refused=]`. `live=` is now the plane's maps' own count, so it cannot drift from them.
>   ⊘ *2026-10-03 (later): `after_host_refused=` is appended at the end (the readback bullet above).*
> - **Wording corrected in the block below** (review LOW): (a) the probe counts entries into HOST
>   CPU-RM only. CPU-RM has no software methods for this class (`dispswGetSwMethods` is the
>   `NOT_SUPPORTED` stub, `g_dispsw_nvoc.h:450-452`), so the guest's display-SW methods are serviced by
>   GSP firmware, and m3c shows the guest does method the object (186 host Xid 32 with no twin). What
>   runs 1-8 show is that host CPU-RM's release path ran 0 times and no `SEMAPHORE_SCHEDULE_CALLBACK`
>   resolved a client — not that no client asked for a release; runs 1-5 had no drain positive control
>   at all. (b) "vsync is paced by the virtual display's flips" is an inference from 59.8-60.0 FPS with
>   0 release-path entries, not a measurement of what paces it. (c) The kernel-mapping candidate
>   (`73532bce`/`a5d31d5b`) was **tried and dropped**, not "the fix": it was never shown to fix anything
>   (no run had a release), it had no budget (33 868 KiB was one workload's peak, not a limit — its
>   bound was whatever guest RAM a guest maps into a display-SW space), and it was incomplete by design
>   (rows placed before the display-SW alloc were never kernel-mapped, so an early semaphore page would
>   still have been dropped).
> - **The re-run** (`d84086df`; `DISPLAY_X11_BARE=1`, `scripts/bench/display/dispsw_run.sh`; runs 9-10,
>   the trace README's dated section). Off (9): the X driver's `Failed to allocate display software
>   resources`, the Cinnamon segfault, X11 vkcube `RC=134`, bare-X fullscreen GL 2.5-2.7 FPS. On (10):
>   Cinnamon up with 0 crashes, X11 vkcube `RC=0` (IMMEDIATE, FIFO, long run), vsync glxgears 59.2-59.8
>   FPS (no-vsync 2 716), bare X 59.5 / 58.5-59.8 FPS and vkcube FIFO `RC=0`, host Xid 0, an empty
>   host-dmesg delta, `dispsw[twins=84 live=0 host_refused=0 no_twin=0 capped=0 id_refused=0 repaid=0
>   withdrawn=0 other_sw=0 free_refused=0]`. All 84 twins read back equal to the guest's number (21
>   channels, 1-4 each). Peaks: 4 live per channel, 20 in the VM.
> - **The classID fix on hardware** (`DISPLAY_X11_ENGSW=1`, `hook.sh` step 4c: a bench-only guest
>   shim allocates, before each display-SW object, a refused `GF100_TIMED_SEMAPHORE_SW` — case (b) —
>   or a display-SW object with an impossible head — case (a); runs 11-13). Case (b) on the fixed
>   binary (11): 20 twins read back one behind, repaid, kept at the guest's number — vsync glxgears
>   59.6 FPS, X11 vkcube FIFO `RC=0`, 0 host Xid. Case (b) on the pre-fix binary `c1cc4482` (12, the
>   negative control): **26 host Xid 32** on the shimmed clients' channels, glxgears 1.2 FPS, vkcube
>   FIFO `RC=134`. Case (a) on the fixed binary (13): the same failure as 12, as this block's known
>   limit says, with kf-rm's pairing counting `unpaired=112` — exactly the 112 constructors the shim
>   failed. ⇒ The guest DOES method its display-SW objects by the number its own RM gave, a wrong
>   number on the twin is an Xid 32 on that client's own channel, and with the right number the
>   methods are serviced (by GSP firmware: host CPU-RM's release path was still entered 0 times).

> **★ 2026-10-03 (box) — WITH `x11-dispsw=on` THE X11 DESKTOP WORKS ON HARDWARE, AND NO DISPLAY-SW
> RELEASE EVER REACHES HOST RM** (vast 54044296, RTX 3060 GA106, host + guest 580.159.04; the A/B pair
> at the branch's final code `c1cc4482`, runs 7-8 of `traces/v3_display/dispsw_20261003/`, with six
> runs before it at `e130cea3`, `a5d31d5b`, `6082f264` agreeing; still default-off, still the
> owner's call). It supersedes the "no box has run it" sentence and answers open items (a)-(d) below.
> - **Off (A):** the X driver logs `Failed to allocate display software resources`, Cinnamon X11
>   segfaults in `libnvidia-glcore` (fallback dialog), X11 vkcube aborts (`RC=134`), glxgears ~40 FPS,
>   and on a bare Xorg with no compositor fullscreen GL runs at 1.7 FPS and vkcube FIFO aborts.
>   **On (B):** Cinnamon X11 up with 0 crashes (panel and wallpaper in the host screendump), X11 vkcube
>   `RC=0` in IMMEDIATE and FIFO and in a Cinnamon window, vsync glxgears 59.8 FPS (no-vsync 2 493),
>   and on the bare Xorg glxgears 59.8 windowed / 60.0 fullscreen and vkcube FIFO `RC=0`. MAILBOX is
>   unsupported by the NVIDIA X11 WSI on and off. Host Xid 0, guest Xid 0, 0 `waiting for GPU progress`,
>   an empty host-dmesg delta, `dispsw[twins=80 live=0 host_refused=0 no_twin=0]`.
> - ⊘ *Corrected 2026-10-03 (review fixes block above): (a) counts HOST CPU-RM entries only — the
>   class's methods are serviced by GSP firmware — and "no release was asked" goes beyond it; (b) is an
>   inference.* **Answers:** (a) never exercised — host RM was asked for **no** release:
>   `dispswReleaseSemaphoreAndNotifierFill` / `semaphoreFillGPUVATimestamp` /
>   `notifyFillNotifierGPUVATimestamp` ran 0 times in all eight runs (`kfdsw_probe`, kretprobes inside
>   host RM; the same probe on the GSP event drain counted ~1.7 million entries per run, so it is not
>   blind; inside those drains 0 client/device lookups, so no release event died early either — that
>   half has no known-positive, see the trace README). (b) Not unthrottled: vsync is paced at the
>   virtual display's 60 Hz by its flips, not by display-SW releases. (c) No other cause: both failures
>   are gone with the object. (d) `0x90720101` was never sent (0 occurrences); no twin verb is built.
> - ⊘ **The review's HIGH prediction did not occur, and its mechanism is real.** Host RM writes a
>   display-SW semaphore/notifier only through the DMA mapping's kernel CPU mapping
>   (`method_notification.c:624-627`, `:349-351`), which exists only for a map made with
>   `NVOS46_FLAGS_KERNEL_MAPPING_ENABLE`; kayfabe never sets it (`kf-host`
>   `no_map_asks_host_rm_for_a_kernel_cpu_mapping` pins that ⊘ *— SUPERSEDED 2026-10-03 by `d84086df`:
>   that test is removed; the pins are `the_kernel_mapping_bit_is_cleared_whatever_the_caller_sets` and
>   `only_the_one_builder_can_name_the_kernel_mapping_bit`*). No client measured asked for a release,
>   so nothing was dropped. ⊘ *Corrected 2026-10-03 (review fixes block above): the pin is now the
>   builder itself (`nvos46_map_flags` clears the bit) plus a source gate — the old test tried only
>   today's callers' bits; and the runs show host CPU-RM's release path entered 0 times, not that no
>   client asked. The candidate below was tried and dropped, never shown to be a fix.* A candidate that
>   kernel-mapped every guest-RAM row of a display-SW space
>   (run 4, `a5d31d5b`) placed 5 986 rows, peaked at 33 868 KiB of host kernel `vmap`, refused none and
>   changed nothing observable; it is not shipped (`6082f264`). A vidmem row would have needed host BAR1
>   (256 MiB on this GPU). A client that DOES ask would have its release logged by host RM
>   (`KernelVAddr==NULL`) and dropped, and would wait on its own semaphore; the probe shows it as
>   `dsw_calls > 0` with `map_kva_null > 0`.
> - **The true security bound** (replacing *"releases only into addresses the TWIN client has mapped
>   … cannot flip, set a mode or touch host display state"* below):
>   1. kayfabe opens ONE host client for the whole VM (`crates/kf-qemu/src/device.rs`, `HostRm::open`,
>      leaked once), and every guest process's twin VA space lives in it. `CliGetDmaMappingInfo`
>      checks `(client, device, hVASpace)` (`mapping_list.c:391-447`), and `hVASpace` comes from GSP
>      firmware in the `SEMAPHORE_SCHEDULE_CALLBACK` event (`kernel_gsp.c:1128-1134`, closed source). So
>      the address check is CLIENT-WIDE, not per guest process; it is per VA space only if GSP names
>      the channel's own VA space, which nothing here can verify.
>   2. That check never matters for a write: with no kernel mapping on any kayfabe map, host RM has
>      no CPU address to release through, so a display-SW object makes host RM write NO memory on
>      the guest's behalf. ⊘ *Scoped 2026-10-03 (review fixes block above): host CPU-RM — the writer
>      `method_notification.c` is. What GSP firmware does with the class's methods is item 3.*
>   3. What stays exposed: the host object itself (one per guest object: 60-84 per boot here, all
>      freed), its head-0 vblank callbacks (on a host that drives a monitor the guest learns that
>      monitor's vblank timing, a side channel; on a headless host they run at once), and whatever GSP
>      firmware does with the class's methods. ogkm-580 defines no `NV9072` method
>      (`class/cl9072.h` has only the alloc params), so *"it cannot flip or set a mode"* is a
>      hypothesis, not a reading.
>
> **⊘ 2026-10-03 (later) — `GF100_DISP_SW` OPTION A IS BUILT AS A DEFAULT-OFF EXPERIMENT, PENDING THE
> OWNER'S RULING** (branch `v3-dispsw-exp`, cut from `v3-b0a`; device property **`x11-dispsw`**, default
> off; `OWNER_QUESTIONS_2026-10-03.md` item 2). ⊘ *Superseded the same day by the box block directly
> above:* **No box has run it: nothing in this note is a box
> result.** It qualifies the M3 block below (*"`kf-rm`'s rule stands: NOT twinned"*) for a device
> realized with `x11-dispsw=on` only. With the property off, every path is the path it was and that
> block stands as written (tests pin it: `crates/kf-rm/tests/display_seat.rs`
> `x11_dispsw_off_refuses_the_query_and_carries_no_alloc`, `x11_dispsw_changes_one_claimed_control_and_no_other`;
> the status line gains nothing, `kf-qemu` `chan.rs::dispsw_tests`).
> - **What `display=on,x11-dispsw=on` does.** `x11-dispsw=on` without `display=on` refuses to realize by
>   name (`Config::check`): the guest's own CPU-RM refuses the object first on a displayless device
>   (`disp_sw.c:67-71`), so the switch could do nothing.
>   1. kf-disp ANSWERS the display-SW constructor's `INTERNAL_DISPLAY_GET_ACTIVE_DISPLAY_DEVICES` (the lit
>      displays and the heads — m3c's answer) instead of `no_display_sw` (`DisplayModel::offer_display_sw`).
>      The guest's own CPU-RM then checks the guest's `logicalHeadId`/`displayMask` against it
>      (`disp_sw.c:83-98`) and RPCs the alloc (`RS_FLAGS_ALLOC_RPC_TO_ALL`, after the constructor:
>      `alloc_free.c:860` constructs, `:916` RPCs).
>   2. The channel link carries that alloc as `ChanStatement::DisplaySw {client, channel, handle}` —
>      it has no params field, so the guest's `NV9072_ALLOCATION_PARAMETERS` cannot reach the host.
>   3. The channel plane allocates `GF100_DISP_SW` under that channel's **passthrough twin** with
>      AUTHORED params `{logicalHeadId 0, displayMask 0, caps 0}` (`kf_host::HostRm::alloc_disp_sw`,
>      `DISP_SW_AUTHORED_PARAMS`). Host RM's constructor needs a display engine (`disp_sw.c:69`) and
>      head 0 < its heads (`:83`); `displayMask 0` skips its active-display check (`:92`). The class is
>      `RS_FLAGS_ALLOC_NON_PRIVILEGED` (`resource_list.h:1502-1511`).
>   4. **Refused `NOT_SUPPORTED` by name** — the status the X driver meets today — when the host
>      refuses the alloc (a host GPU without a display engine), when no passthrough twin holds the
>      channel, or when the device has no channel plane. ⊘ The link never leaves a display-SW object
>      without a host object: that is m3c (186 host Xid 32, 1.3 FPS GL, 2026-09-30).
>   5. **Freed** with the guest's own free; with its channel's twin (a free of the channel, its group,
>      its device or its client — host RM frees the object with its channel, and `HostRm::free` forgets
>      the subtree); hence at a guest driver unload / GSP re-init, which reaches the plane as the
>      guest's own frees, exactly as every channel twin does; at device teardown with the host client.
>   6. **Status line:** `dispsw[twins=N live=N host_refused=N no_twin=N]` after `disp[...]`, printed only
>      with the property on. ⊘ *2026-10-03 (review fixes block): six more fields, and `live=` is counted
>      from the maps.*
> - **The rule it changes (the owner's call).** *"NOT twinned: host RM's dispsw acts on HOST display
>   heads"* becomes *"twinned with an authored head; host RM releases only into addresses the TWIN
>   client has mapped"* — `dispswReleaseSemaphoreAndNotifierFill` validates against the calling client's
>   own DMA mappings (`CliGetDmaMappingInfo`, `disp_sw.c:146`), and on a GSP host that call arrives as
>   the `SEMAPHORE_SCHEDULE_CALLBACK` event for OUR client (`kernel_gsp.c:1110-1135`). The guest learns
>   the host's vblank timing (a minor side channel). It cannot flip, set a mode or touch host display
>   state. ⊘ *Corrected 2026-10-03 (box block above): "the TWIN client" is kayfabe's ONE host client,
>   so the check is client-wide; host RM writes nothing at all here (no kernel mapping); and the
>   no-flip sentence is a hypothesis.*
> - **Open — only a box answers** (⊘ *answered 2026-10-03, the box block above*): (a) whether host RM finds the guest's semaphore/notifier VA among the
>   twin client's mappings (kayfabe maps the twin's VA space with RM `MAP_MEMORY_DMA` calls, so it
>   should; if it does not, host RM refuses the release, `NV_ERR_INVALID_ADDRESS`, `disp_sw.c:152-153`,
>   and what the guest's client does then is for the box to show);
>   (b) the pacing: a headless host runs each vblank callback at once (`vblank.c:87, 209-243`), so X11
>   should run UNTHROTTLED (glxgears well above 60 FPS); a host that drives a monitor paces the guest at
>   that monitor's refresh; (c) whether Cinnamon's X11 crash in `libnvidia-glcore` and X11 vkcube's abort
>   (rc 134, `traces/v3_display/b0a_20261003/`) have no other cause; (d) whether any guest client sends
>   the object's one control, `NV9072_CTRL_CMD_NOTIFY_ON_VBLANK` (`0x90720101`, "an out-of-band version
>   of the … NOTIFY_ON_VBLANK method", `ctrl9072.h:52-84`). It is `ROUTE_TO_PHYSICAL | NON_PRIVILEGED`
>   (flags `0x48`, `g_dispsw_nvoc.c:176-190`), so it would reach us, and NOTHING here serves it: it
>   meets the unserviced ledger and is refused `NOT_SUPPORTED`. If the box shows it, it needs a twin
>   verb on the host object (unprivileged there too); it is not built because nothing has shown a
>   client sending it.
> - Carried with it: the driver matrix gains `NV9072_ALLOCATION_PARAMETERS` (the sweep of 2026-10-03 at
>   all 29 tags: one 12-byte layout, `traces/driver_matrix/ranges.tsv`), and `kf3_realize` takes
>   `x11_dispsw` — **KF3 ABI 13** (11 and 12 are taken by `v3-gop` and `v3-broker` for other signatures).
> - **The box test** (one GPU box with a display engine, e.g. the RTX 3060 of `b0a_20261003`; strictly
>   serial; the lane runs the per-revision binary, so build at this branch's head first):
>   ```
>   bash scripts/bench/build_kf3.sh <qemu-10.2.4-source-tree>
>   # A — control, the property off (today's behaviour; b0a run 3 is the reference):
>   DISPLAY_DESKTOP=1 KF_DEVICE=kf3 bash scripts/bench/display/lane.sh dsw_off
>   # B — the experiment:
>   DISPLAY_DESKTOP=1 KF_DEVICE=kf3 DISPLAY_KF3_EXTRA=x11-dispsw=on bash scripts/bench/display/lane.sh dsw_on
>   ```
>   ⊘ *2026-10-03 (box): the runs used `scripts/bench/display/dispsw_run.sh` (this lane plus the host
>   probe, START/EXIT lines) with `DISPLAY_X11_BARE=1` (a bare Xorg with no compositor); grade also on
>   observables a release would gate — vsync `DISPLAY_GLXGEARS` frames ≈ 60/s, `DISPLAY_X11_BARE_*`
>   (glxgears windowed/fullscreen, vkcube FIFO `RC=0`), `DISPLAY_HOST_XID=0` with an empty
>   `run_*_hostdmesg.log` — and on the probe's `DSW_TRACE_TOTALS` (`dsw_calls`, `map_kva_null`).
>   `DISPLAY_DESKTOP_SESSION=yes` alone is not a pass: run A reads `yes` over a crashed Cinnamon.*
>   ⊘ *2026-10-03 (review fixes): grade the status line's `capped=0 id_refused=0 free_refused=0` too,
>   and `software classID N = the guest's` on every `act display-SW twin` line; `hook.sh` step 4c
>   (`DISPLAY_X11_ENGSW=1`, mode `9074`) is the classID fix's own test — `0` host Xid and ~60 FPS there,
>   where the pre-fix code gives Xid 32 and 1.2 FPS (runs 11-12).*
>   **Pass (B):** `DISPLAY_DESKTOP_SESSION=yes`; `DISPLAY_DESKTOP_CRASHES 0`; `DISPLAY_VKCUBE … VKCUBE_RC=0`
>   (and the `DISPLAY_VKCUBE_PM0/1/2` lines' `RC=`); `DISPLAY_HOST_XID=0`; in `run_dsw_on_qemu.log` the
>   realize line `EXPERIMENT x11-dispsw ON` and a status line with `dispsw[twins=` > 0, `host_refused=0`,
>   `no_twin=0`; and neither its `unserviced=[…]` nor a `kf3: GSP REFUSED` line names `0x90720101`
>   (open item (d)). **Record, do not grade:** `DISPLAY_GLXGEARS` and `DISPLAY_GLXGEARS_NOVSYNC` (expected
>   unthrottled on a headless host), the `desk_1`/`desk_vkcube` screendumps, and the same lines from A for
>   the comparison. ⊘ A chip with no display engine on bare metal (GA100, GH100, GB10x) cannot take this
>   test at all — `display=on` refuses to realize there (§2.2). The B-fallback path (`host_refused` > 0,
>   the guest then exactly as in A) is reachable only on a host whose RM has no display engine on a
>   display-capable chip (a board with its display disabled); no such box has been named.

> **STATUS: M3 MET FOR THE WAYLAND DESKTOP; X11 DESKTOP PARTIAL — 2026-09-30 (branch `v3-display2`;
> display stays default-off).** RTX 3060 (GA106), host + guest 580.159.04, vast 53505783.
> - ★ **Merged with the doorbell fast path as `v3-mc22` (2026-09-30), KF3 ABI 10** (`traces/v3_mc22/`):
>   the merge bar 30/30, and M1/M2 again at the merged revision — pixel-exact 1920x1080, 120/120 flips —
>   with `doorbell-ioeventfd` off **and on** (vast 53004208, GA106).
> - **Mint's desktop on the virtual monitor** (`traces/v3_display/m3i_20260930/`): lightdm autologin
>   into Cinnamon's Wayland session — muffin drives the emulated display through nvidia-drm KMS and
>   renders with the NVIDIA EGL/GBM stack on the RTX 3060; `vkcube-wayland` (Vulkan) runs in a
>   Cinnamon window; both in the HOST screendump (`console_cinnamon_wayland*.png`); nothing crashed.
>   weston 13 likewise (`m3h`: `vkcube-wayland` exits 0 on its overlay window, visible in the dump).
> - **X11** (`m3f`, `m3h`): Xorg + the stock NVIDIA X driver set 1920x1080 on `KFB kayfabe (DFP-0)`;
>   `glxinfo`: `NVIDIA GeForce RTX 3060/PCIe/SSE2`, direct; glxgears vsync-locked 54-60 FPS; the
>   console image equals the X server's own root-window screenshot in all 2 073 600 pixels.
>   ⊘ **Open, one cause:** Cinnamon's X11 session segfaults in `libnvidia-glcore` (fallback dialog)
>   and X11 Vulkan presentation fails at `vkCreateSwapchainKHR`. Both need a `GF100_DISP_SW` object;
>   its methods are SOFTWARE methods RM services when the host engine traps them, but the guest's
>   channels run on the host GPU, whose RM has no such object — `[m3c]` offering it produced 186 host
>   `Xid 32` and 1.3 FPS GL. So it is refused by name (`kf_disp::model`, `no_display_sw`), and
>   `kf-rm`'s rule stands: *"NOT twinned: host RM's dispsw acts on HOST display heads"*
>   (`chanlink.rs:1304-1316`). ⊘ **OWNER DECISION NEEDED:** twinning it on the host with AUTHORED params (head 0,
>   displayMask 0) would make a headless host run each vblank callback immediately
>   (`vblank.c:87, 209-243`, *"call it now"* when the head's vblank interrupt is unavailable) — i.e. X11
>   would work unthrottled by dispsw — but it lets a guest's software methods act on the host's
>   display object, which the rule forbids. ⊘ *2026-10-03 (box): measured as the default-off
>   `x11-dispsw` — both failures gone, vsync paced at 60 Hz (not unthrottled), no release ever reaching
>   host RM; the x11-dispsw note above.*
> - **The console composes the head** (`m3h`): every enabled window back to front by `DEPTH`, placed
>   by its window-immediate `POINT_OUT`, blended per `SET_COMPOSITION_FACTOR_SELECT`, pitch or
>   block-linear, by one hand-written PTX kernel (`cuda/display/kf_scanout.ptx`, bring-up
>   self-test), into a device staging frame copied to the console — the probe stays pixel-exact.
>   The block-linear GOB order is MEASURED (`m3b`): `x[3:0] y[1:0] x[4] y[2] x[5]`, not the
>   often-quoted Tegra order. Not composed yet: the cursor channel (the console shows the pointer
>   only when the compositor draws it — ⊘ superseded 2026-10-03: display step 3d composes it in code,
>   §8.6), scaled windows (shown unscaled, clipped), YUV/16-bit layers.
> - Also measured on the way: `BUS_GET_INFO_V2` index `0x14` (`PCIE_GEN2_INFO`) is served with
>   `0x2d`'s word (the real GA106 answers both identically; the X driver's fatal "Failed to query PCI
>   info" was its refusal); `SYSTEM_GET_ACTIVE` reports the lit display (`Display Active: Enabled`).
> - Next: the §7 display-app list (`V3_GFX_TESTSET.md`), the cursor plane on the console, and the
>   owner's ruling on `GF100_DISP_SW` for X11 compositors / X11 Vulkan.

> **STATUS: M1 AND M2 MET ON HARDWARE, 2026-09-30 (branch `v3-display2`; display stays
> default-off).** RTX 3060 (GA106), host + guest 580.159.04, vast 53505783.
> - **M1** (`traces/v3_display/m1b_20260930/`, `m1c_20260930/`): the emulated NVDisplay engine (worker
>   thread, never a vCPU) consumes the core and window pushbuffers, arms, latches flips at a host vblank
>   timer and writes notifiers/semaphores only after the state they report is armed. Guest:
>   `/dev/dri/card0` + `renderD128`, `DVI-D-1 connected` 1920x1080@60 (7 modes), 4 CRTCs with primary /
>   overlay / cursor planes, fbcon at 1920x1080, **0** `waiting for GPU progress`, 120/120 flips at
>   59.98 Hz, no kernel WARN. Flip AWAKEN follows nvidia-drm's rule (only for a window that scanned a
>   surface on an active head before the update, and only when the new state asks for it).
> - **M2** (`traces/v3_display/m2c_20260930/`, proof of the copies in `m2b_20260930/`): each flip of the
>   window the console shows is copied by the GPU (`cuMemcpy2DAsync`, the display plane's own context
>   and stream) out of the store into page-locked frames a QEMU graphic console shows zero-copy
>   (`kf3_display_frame`, ABI 9 on `v3-display2` — ⊘ **10** since the `v3-mc22` merge, whose doorbell fast path had
>   taken 9 independently); the flip's notifier, its semaphore release and GET wait for that copy
>   to complete. **`screendump … kf0` is pixel-exact against the probe's pattern (1920x1080)**, flips
>   at 60.01 Hz, `nvidia-smi` `Display Active : Enabled`, 0 flip-event timeouts, 0 DRM WARNs.
> - Known limits: pitch surfaces only — block-linear and system-memory surfaces are refused by name
>   (the X driver's desktop surfaces need the block-linear copy: M3); one console, on the lowest running
>   head, showing its lowest enabled window (no overlay/cursor composition). A client that exits with
>   its framebuffer on the plane logs nvidia-drm's own "Flip event timeout" (no notifier exists for a
>   disabled layer; real GPUs log the same — `m1c` README).
> - Next: M3 (desktop session on the virtual monitor, GPU-accelerated, screenshot-verified).

> ⊘ **SUPERSEDED 2026-09-30 by the M1/M2 block above** (the `0xc67d:0` GPU-progress wait is gone
> since the engine consumes the core channel; `/dev/dri` and a connected output exist).
> **STATUS: DISPLAY-ON MEASURED, INCOMPLETE, 2026-09-29.** At exact `d883d0eb` on RTX 3060,
> the guest boots, `nvidia-smi` succeeds and the model records an accepted core channel.
> NVKMS then repeatedly waits for `0xc67d:0` GPU progress; no `/dev/dri` or connected output
> appears. Evidence: `traces/recovered_display_20260929/`. This is not an M1/scanout pass.
> The next display work is the engine/worker/notifier path in steps (2)/(3), not a fabricated
> completion. The default-off full merge bar below passed independently.

> **STATUS: INTEGRATION CANDIDATE, 2026-09-29.** The step-(1) lifecycle gap (a) below is fixed:
> the display control link no longer observes speculative allocs/frees. Its separate registry
> is notified inside the object seat only after `RmObjects::apply` succeeds. `ObjectLinks` now
> requires the composable `ObjectPolicy` type, so this acceptance boundary is structural.
> Whole-chain regression tests cover undeclared clients, conflicting allocations that would
> replace a live core channel, serialized/short requests, an absent object seat, an injected
> free refusal, successful parent/client frees and handle recycling. All `kf-rm` and `kf-disp`
> tests pass locally. RM alloc has no fragmented/large-RPC path: short allocs remain refused,
> not held. No transport claim set changed. The full combined hardware bar passed at `d883d0eb`
> (1683/0 tests, gates 9/9, fresh guest, thin 30/30; `traces/recovered_integration_20260929/`).
> Display remains default-off; no scanout/desktop claim is added. Gaps (b)/(c) below remain.

> ### ★ 2026-09-27 (later) — next step (1) DONE in code, GPU-free (`b11f96c6`, branch `local/display-step1` on `4c48ca0c`); NOT on the bench
> **What step (1) now does** (`crates/kf-rm/src/display.rs`). Still only with `display=on`; with the
> default (off) the display link is never built, so a default-off device answers exactly as before
> (`crates/kf-rm/tests/display_seat.rs` pins it through the whole served chain).
> - `DisplayPolicy` delegates to a `SharedDisplayModel` = `Arc<Mutex<kf_disp::model::DisplayModel>>`
>   (`kf-rm` now depends on `kf-disp`), built from the chip's display row, one 1920×1080 DVI-D monitor
>   (`Monitor::default_1080p`) and the control layouts **derived** for the guest driver's version
>   (`kf_disp::layout::for_version` — today `580.159.04` only). A guest driver without derived layouts
>   keeps the M0 answers and observes nothing (logged at realize).
> - It answers **every control the model claims**: the M0 five (byte for byte the M0 replies m0a
>   measured — a unit test compares them), `INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER` (`0x20800a58`), and the
>   30 NVKMS bring-up controls of §4.2 (A) — among them m0a's first refusal `0x730101` and the ledger's
>   `0x730107`, `0x730102`, `0x730151`. 36 controls in all.
> - **The FINN refusal set is the claim set**: a claimed control arriving FINN-serialized is refused
>   `NOT_SUPPORTED` by name, never decoded; one whose params are missing, `INVALID_ARGUMENT`. [E] No
>   display interface is FINN-serializable in 580 (`ogkm-580: src/nvidia/interface/rmapi/src/g_finn_rm_api.c:803-850`),
>   so only a crafted message meets this refusal.
> - It **observes** `GSP_RM_ALLOC` / `GSP_RM_FREE` (it returns `None`: the channel link and the object
>   seat still see both, and the object seat answers). A display-channel alloc is recorded in the model's
>   registry — instance, GET = PUT = `offset`, the pushbuffer stated before it; a free releases it: the
>   channel's own free, its display object's or device's free (parent edges remembered, at most 256),
>   its client's free. [E] The guest's RM sends one free per object (`ogkm-580: rs_client.c:785-843`
>   → `alloc_free.c:959-990`), children before their parent (`rs_client.c:1085-1092`).
> - **Hostile-guest bounds** added to `kf_disp::model`: `CHANNEL_PUSHBUFFER` is accepted only for this
>   family's channel classes at an instance the display has; an alloc whose params are not the derived
>   struct, or whose instance does not exist, is not recorded (it was read as instance 0); the statement
>   queue stops at 1024 and counts the rest. The link drains the statements into the QEMU log after it
>   drops the lock (no worker exists yet to take them). No vCPU takes the model's lock.
> - `kf_abi::oracle::CAPTURE_RELIANCE` gains `0x00730107` (`SYSTEM_GET_SUPPORTED`, a truncated C row):
>   NOT A READ, the model authors it from its own connectors.
> - `DisplayModel::claimed()` enumerates the claim set (6 internal + 30 named, ids from the derived
>   layouts); the link caches it at construction, so asking whether a control is the display's takes
>   no lock.
> - Crate tests (before → after): `kf-disp` 9 → 11, `kf-rm` 544 → 553, `kf-abi` 528 → 528, `kf-qemu` 10 → 10.
>
> **What remains of step (1)** — nothing GPU-free. On the bench (the M1 grade, §5): with `display=on`,
> NVKMS gets past `0x730101` and the unserviced ledger holds no NV0073 / NV5070 / NVC370 / NVC372 id.
> Known gaps, each small and bounded: (a) ⊘ *FIXED in the 2026-09-29 integration candidate above;
> historical diagnosis, corrected 2026-09-28 (review): the effect is wider than
> first written.* An alloc is observed before the object seat answers it, and a free likewise. An alloc
> the seat then refuses **replaces** a live channel's registry entry at the same `(kind, instance)` (new
> handle, GET/PUT and pushbuffer), after which the real channel's own free no longer matches it
> (`DisplayModel::free` keys on `(client, handle)`); a free the seat refuses still releases the entry.
> Only `display=on`, only a crafted guest, the harm stays in that guest's own display, and nothing reads
> the registry yet. The fix records on accepted object application, not before it. (b) a
> claimed control is answered whatever object it names (`hObject`'s class is not checked);
> (c) `kf-qemu` does not hold the model yet — step (3) must take the `SharedDisplayModel`
> (`DisplayPolicy::over` / `DisplayPolicy::model`) across `ReselectAtFn1` rebuilds of the chain, and move
> the statement drain from the link to the display worker. Steps (2)–(4) below are unchanged.

> ### ⊘ STOPPED 2026-09-27 (owner: weekly usage limit) — where this stands, and the next steps
> **Done (branch `v3-display`):** Phase 1 — this design, decision (a). M0 code (`5dbf670b`, device
> property `display=on`, default off): the chip display rows (`kf_chip::display`), the KernelDisplay
> init controls (`kf_rm::display`), display classes admitted as graph nodes, the display lane
> (`scripts/bench/display/`), guest display provisioning, a VNC+pixman kf3 build.
> **[M] measured, run `m0a` on vast 52837869 (RTX 3090 GA102, host+guest 580.159.04, kf3 `5dbf670b`),
> evidence `traces/v3_display/m0a/`:** with `display=on` the guest's **KernelDisplay comes up** (no
> `kdisp*` init failure; `numDispChannels` etc. accepted), compute unchanged (`nvidia-smi` OK), and
> **NVKMS stops at its first physical-RM query**: `NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2` (`0x730101`)
> refused ⇒ *"Failed to determine display common capabilities"* ⇒ nvidia-drm falls back to displayless
> (`card0` with 0 connectors). The ledger also names `0x730107` (GET_SUPPORTED), `0x730102`
> (GET_NUM_HEADS) and `0x730151` (MAP_SHARED_DATA). Pre-existing, not display: the RC watchdog's
> `0xc36f` channel alloc and `NV40_I2C` are refused and tolerated.
> **Written but not yet on the bench (after `5dbf670b`):** `kf_disp::layout` + `tools/derive_display_layouts.sh`
> (control layouts compiled from ogkm), `kf_disp::class` + `tools/derive_display_classes.sh` (per-family
> method/field/caps tables compiled from the class headers), `kf_disp::model` (answers for all ~30
> NVKMS bring-up controls of §4.2 (A), the pushbuffer/channel registry, `GET_CHANNEL_INFO` idle from GET==PUT).
> ⊘ *Superseded 2026-09-27 by the step-(1) note above: the model is wired (with `display=on`).*
> Unit-tested (`cargo test -p kf-disp`), **not yet wired**: `kf_rm::display` still answers only the M0 set.
> **Next, in order:** (1) ⊘ *done in code 2026-09-27, not on the bench (the note above):* make
> `kf_rm::display::DisplayPolicy` delegate to a shared
> `Arc<Mutex<kf_disp::model::DisplayModel>>` and observe display allocs/frees (`DisplayModel::alloc/free`);
> (2) `kf_disp::engine`: PUT → read the 4 KiB sysmem pushbuffer → decode → assembly/armed state per class
> (derived tables) → core notifier FINISHED + ARMED mirror at `0x688000` → window flips with acquire
> (EQ `0xf473f473`) / release-on-flip-away (`0xd00dd00d`) + WRITE_AWAKEN notifier; ctxdma resolution by
> scanning the guest's instance-memory hash table (§4.4); (3) `kf-qemu`: a display worker thread, the
> BAR0 display register file (W1C `EVT_STAT_*` applied in the trap, derived `RM_INTR_*`/`DISPATCH`,
> `CORE_HEAD_STATE` AWAKE, cursor `FREE`=4, the caps page at `0x640000` from the C373/C673 tables), the
> AWAKEN interrupt on vector `0x9a`; (4) M2: the scanout copy kernel (kf-cuda) + the QEMU graphic console
> (kf3.c) + `lane.sh` pixel-exact grade. Merge bar not run on this branch (display defaults off).

**STATUS: DESIGN, LIVE — decided 2026-09-27 (branch `v3-display`); Phase 2 STOPPED at M0 (see the note above); its next step (1) written and unit-tested 2026-09-27, not on the bench (the first note).** Phase 1 of the
owner's display roadmap item (`OWNER_RULINGS.md` §C.3, 2026-09-26: *"go full in on getting the DISPLAY to
work, then display apps / Mint desktop etc. that nvkvm-pv had working, on kayfabe"*). Nothing below is
measured on a kf3 guest yet except where a row says **[M]**; source readings are **[E]** with a
`file:line`; inferences are **[I]**. Sources read: ogkm `580.159.04`
(`/workspace/nvidia-gpu-passthrough/research_clones/ogkm-580.159.04`, cited `ogkm-580:`), ogkm `610.43.02`
(`ogkm-610:`), nouveau (Linux 7.3-rc3, `research_clones/nouveau-src`; 7.1-rc6 in `research_clones/linux`), nvkvm-pv
`368d2db` and its later branches,
kayfabe master `e9c37b1c`.

---

## 0. Short answer

**Build (a): emulate the display engine the guest's own driver expects — a virtual NVDisplay head with a
virtual monitor — so the stock `nvidia.ko` / `nvidia-modeset.ko` / `nvidia-drm.ko` bring up a real KMS
device, and kayfabe reads each flipped surface out of the store with its own GPU copy and hands the frame
to QEMU's display layer (VNC, SPICE, GTK, `screendump`).**

Why, in one paragraph: it is the only candidate under which **stock guest userspace behaves as on bare
metal** — lightdm → Xorg with NVIDIA's own DDX (Mint's default session), GNOME/KDE/weston/sway on NVIDIA
KMS through `libnvidia-egl-gbm`, gamescope, `nvidia-settings`, the cursor plane muffin needs, multiple heads
and hot-plug — with **no guest configuration** (the compliance principle, `OWNER_RULINGS.md` §A.8; the
owner on 2026-09-18: *"real KMS planes flow from guest to host, not displayless … in wayland no separate
config was needed"*, `docs/archive/the_display_plane_target_is_a_virtual_head.md`). Every owner rule holds
without an exception (§4.8): the display channels are **emulated channels** with no guest GPU work behind
them; the only host work is kayfabe's **own** copy of a surface; flip and vblank completions are driven by
a host timer fd and that copy's host completion; nothing blocks a vCPU; no VMM address reaches the guest;
the per-family facts (display IP version, class set, method and register layouts) are **generated from
ogkm**. The price is size: roughly **3–5 weeks** to a desktop on GA10x, then days per further family
(§5), against **days** for the cheapest alternative.

The cheapest alternative, **(b) a separate emulated display adapter plus PRIME render offload**, is kept
as the documented fallback and as **the display answer for the families that have no display engine on
bare metal either** (GA100, GH100, GB100/GB102/GB110/GB112 — `gpuFuseSupportsDisplay` is hard-wired
false for them, `ogkm-580: src/nvidia/generated/g_gpu_nvoc.c:1831-1835`): on those, bare metal also needs
another adapter for a screen, so (b) *is* bare-metal parity there (§2.2, §4.10).

---

## 1. What a desktop needs from the display plane, whatever the design

| need | what the guest does with it |
|---|---|
| a KMS device with **connectors, CRTCs, planes** (primary, overlay, **cursor**) and modes | compositors and Xorg enumerate outputs; muffin (Cinnamon) crashes without a cursor plane (nvkvm-pv `mint-guest-desktop.md:66-120`) |
| **atomic commit / page flip** with **flip-complete and vblank events** | every compositor paces on them; weston's fade-in never finished without `drm_crtc_vblank_on` (nvkvm-pv `8f0e551`) |
| a scanout path for **NVIDIA-allocated block-linear surfaces** (`DRM_FORMAT_MOD_NVIDIA_BLOCK_LINEAR_2D`, e.g. `0x0300000000606014`) | `libnvidia-egl-gbm` renders into these and flips them |
| **GPU-accelerated rendering** of the desktop and its apps | already built: the headless set is 38/38 (`V3_GFX_TESTSET.md`) |
| **frames out of the VM** | a VNC/SPICE/GTK window, a screenshot, or a stream |
| **cursor, several heads, hot-plug / resize** | window-sized guest desktops, multi-monitor |

---

## 2. The candidates

Legend for the fit column: ✔ holds by construction · ◐ holds with care · ✘ fails.

| | (a) emulated NVDisplay virtual head | (b) displayless NVIDIA + separate display adapter + PRIME | (c) nvkvm-pv's guest KMS module + present/broker, adapted | (d) `NVA083_GRID_DISPLAYLESS` (610+ vGPU build) | (e) headless compositor + in-guest remote desktop |
|---|---|---|---|---|---|
| **guest sees** | an NVIDIA GPU with a monitor on a DP/HDMI connector; `card0` = nvidia-drm with real connectors, cursor/overlay planes | NVIDIA render-only (`renderD128`, 0 connectors) **plus** a virtio-gpu (or bochs/ramfb) `card` with the connectors — a hybrid-graphics laptop | nvidia-drm displayless **plus** a kayfabe guest KMS module (`Virtual-1`) | nvidia-drm with heads backed by NVKMS's software displayless HAL | NVIDIA render-only; no KMS head at all |
| **guest change** | none | guest config (offload env, compositor multi-GPU, xorg.conf) | **a non-stock guest kernel module** | **a non-stock guest driver build** (`NV_GRID_BUILD`) | guest config (compositor, VNC/RDP server) |
| **Mint Cinnamon X11 (lightdm → Xorg)** | ✔ NVIDIA DDX, as bare metal | ◐ modesetting on the adapter, desktop composited on llvmpipe/virgl; apps via PRIME offload | ◐ as nvkvm-pv: weston + rootful Xwayland + Cinnamon (lightdm → Xorg glamor fails even on bare metal, `mint-guest-desktop.md:122-199`) | ✔ probably (DDX on displayless HAL) | ◐ Xwayland on a headless compositor |
| **Wayland compositor on NVIDIA KMS** | ✔ | ◐ only where the compositor supports render-GPU ≠ display-GPU (mutter/KWin/wlroots yes, weston/gamescope no) | ✔ (nvkvm-pv ran weston/KWin/GNOME) | ✔ | n/a (headless backend) |
| **frames out** | kayfabe's scanout copy → QEMU console (VNC/SPICE/GTK/`screendump`); zero-copy dma-buf later | the adapter's own QEMU path (stock) | guest module → kayfabe → QEMU console | **none** — NVKMS completes flips in software (`ogkm-610: nvkms-displayless.c:241-326`); an in-guest capture must stream | in-guest VNC/RDP/NVENC over the network |
| **desktop GPU-accelerated** | ✔ all of it | ◐ apps yes; compositor only on multi-GPU compositors | ✔ | ✔ | ✔ |
| **vsync / flip-complete** | host timerfd vblank + scanout-copy completion (§4.5) | the adapter's (QEMU fence) | host-paced, module-generated | guest software timer | compositor-internal |
| **cursor** | ✔ hardware cursor channel | ✔ (virtio-gpu cursor queue) | ✘ unless built (nvkvm-pv never did) | ◐ | ✔ composited |
| **multi-head / hot-plug** | ✔ up to the family's head count; hot-plug via an EDID we author | ✔ (virtio-gpu) | ◐ to build | ◐ | ✘ / n/a |
| **cost to a desktop** | **3–5 weeks** (GA10x), then days per family | **days** (no kayfabe code) | 2–3 weeks + a guest module to maintain | 1–3 weeks + a licensing/product decision | days (no kayfabe code) |
| **risk** | medium: large emulation surface, but every layout is published (§4.2) and bare metal is an oracle | medium: NVIDIA's cross-device paths in userspace; X11 stays half-accelerated | medium-high: NVIDIA userspace quirks with a non-NVKMS head (nvkvm-pv's egl-gbm hang) | high: gated on a build flag; frames never leave by themselves | low |
| **v3 rules** | ✔ all (§4.8) | ✔ all | ✘ compliance (non-stock module) | ✘ compliance (non-stock driver); ⊘ 580 has no NVA083 at all | ✔ all |
| **families** | every family whose bare metal has a display engine (TU10x, GA10x, AD10x, GB20x) — generated per family | ✔ family-independent | family-independent | 610+ only | family-independent |

### 2.1 (a) Emulate the display engine the guest expects — **chosen**

A guest GSP client decides it has a display from one fuse bit and one physical-RM control, then drives
it through RM controls (most routed to physical RM, i.e. to us), four kinds of display channels
(core, window, window-immediate, cursor) and a small BAR0 register vocabulary that ogkm publishes per
display IP version (`ogkm-580: src/common/inc/swref/published/disp/v0{3_00,4_00,4_01,4_02,5_01,5_02}/dev_disp.h`,
564 lines in all). §4 is the design. The previous estimate of *"3–6 months, dominated"* for this posture
(`archive/nvkvm/docs/design/display_plane_scoping.md` §0, 2026-08-12) was made against `NVA083` as the
alternative; §2.4 shows why that alternative does not exist for a stock guest, and §4.2 sizes the
surface from source rather than by analogy.

### 2.2 (b) Displayless NVIDIA + a separate display adapter — **fallback, and the answer for displayless families**

The guest gets a virtio-gpu (`-device virtio-vga` / `virtio-gpu-pci,blob=true` on the memfd RAM kayfabe
already uses) and keeps NVIDIA displayless (today's state, `V3_HEADLESS_GRAPHICS.md` §3). NVIDIA renders;
the result reaches the adapter through PRIME: the compositor (or NVIDIA's WSI) blits into a linear
**guest-sysmem** buffer the adapter scans out, and the GPU writes it through the sysmem mirror v3 already
has. [I] Linux ≥ 6.13 virtio-gpu can also import a foreign dma-buf as a guest blob
(`linux: drivers/gpu/drm/virtio/virtgpu_prime.c:292-340`), but a vidmem dma-buf would resolve to BAR1
guest addresses that QEMU can only read at CPU-BAR speed (~48 MiB/s, memory
`vidmem_cpu_reads_are_48_mibs_and_bar1_bounds_views.md`), so the copy to sysmem is the real path.

What it cannot do, and why it is not the primary: the NVIDIA DDX cannot run without an NVKMS display
device (VirtualGL added its EGL back end for exactly the GPUs that cannot run X — [I] external), so
**Xorg sessions are composited on the adapter's GL** (llvmpipe, or virgl on the host GL, which is not
kayfabe's path); weston and gamescope have no render-GPU ≠ scanout-GPU mode; and every NVIDIA-display
feature (DDX, `nvidia-settings` display pages, NvFBC, DRM leases, VRR) is absent. On bare metal a GeForce
with a monitor behaves like (a); only on the datacenter parts that have no display engine does bare metal
look like (b) — which is why (b) is the right answer **there** (§4.10).

### 2.3 (c) nvkvm-pv's approach, adapted

nvkvm-pv's display was its **own guest kernel module**: a `drm_simple_display_pipe` virtual head
(`nvkvm-pv src/guest/nvkvm_kms.c`: one `DRM_MODE_CONNECTOR_VIRTUAL`, no EDID, no cursor plane, a software
hrtimer vblank) whose flips were looked up to the host buffer behind them and handed to QEMU as a dma-buf
(GL zero-copy), a PBO readback (VNC), or a broker relay (`display-broker-findings.md`). In Mode 2 the stock
`nvidia-drm` owns `card0`; there is no nvkvm module to put the head in. Re-creating it would mean a new
non-stock guest module *and* inheriting NVIDIA userspace's assumptions about a non-NVKMS head (the
`libnvidia-egl-gbm` wait that nvkvm-pv only escaped by reporting `supports_sync_fd = 0`, nvkvm-pv
`18359b9`). What **does** carry over is the host half: the present modes (dma-buf to a GL UI, readback for
VNC), the pacing lessons, the broker protocol, and the measurement method (§5.3).

### 2.4 (d) `NVA083_GRID_DISPLAYLESS`

A complete NVKMS HAL for GPUs with no display engine (`ogkm-610: src/nvidia-modeset/src/nvkms-displayless.c`,
885 lines, `.coreChannelDma = { }`), fed by a GSP event we would own. Three facts rule it out as the design:
**580 has no such HAL in NVKMS at all** (`V3_HEADLESS_GRAPHICS.md` §3.1); in 610 it is reachable only from
a guest driver **built** with `NV_GRID_BUILD` (`ogkm-580: kernel-open/nvidia/os-interface.c:1445-1458`,
`gpu.c:1958-1984`) — a licensed vGPU package, not the stock driver; and its flips complete in a guest
software timer after polling the acquire semaphore (`nvkms-displayless.c:241-326`) — **no surface ever
leaves the guest**, so a VMM display would still need (a)'s scanout or an in-guest capture. It remains the
right tool for a *headless-compute vGPU product*, which is a different product.

### 2.5 (e) Headless compositor + in-guest remote desktop

Already works: the headless set runs weston/sway headless with NVIDIA GL and captures them
(`V3_GFX_TESTSET.md` H14, H24). An in-guest VNC/RDP/NVENC server (nvkvm-pv streamed XFCE at 1080p58 from a
T4 this way, `tested-platforms.md:177-182`) gets frames out over the network with **no VMM display code**.
It is a deployment recipe, not a display plane: no KMS head, no stock desktop session, nothing for the
display-phase list D1–D24. Kept as a product note.

---

## 3. The decision, and the reasons in rule order

1. **Guest userspace must work (compliance principle).** Only (a) and (d) give stock guest userspace an
   NVIDIA KMS device; (d) is not a stock driver. Under (a) nothing in the guest is configured for the VM.
2. **The display-phase list is written against an NVIDIA KMS head.** Of `V3_GFX_TESTSET.md` §7's D1–D24,
   the DRM-backend compositor rows (D2, D5–D8, D17–D21) and the SteamOS rows (D20–D24) need NVIDIA KMS;
   under (b) they either change meaning (a different card) or cannot run (gamescope, weston).
3. **Hostile-guest isolation is preserved.** Display channel allocation is `RS_FLAGS_ALLOC_PRIVILEGED`
   (`ogkm-580: src/nvidia/src/kernel/rmapi/resource_list.h:1270-1318`): only the guest *kernel* (NVKMS)
   can drive it. What kayfabe exposes is an emulation, never the host's display engine — the reason
   forwarding a display channel is ruled out (host-global hardware, `display_plane_scoping.md` §1.3) does
   not apply to emulating one.
4. **All families first-class.** The per-family inputs are generated: the display IP version per chip
   (`ogkm-580: src/nvidia/generated/g_chips2halspec_nvoc.h:139-155`), the class set per chip
   (`generated/g_gpu_class_list.c`), method offsets from the class headers, register offsets from
   `published/disp/v*/dev_disp.h`. Families without a display engine get (b), exactly as their bare metal.
5. **Bare metal is an oracle.** A GeForce host can run the same NVKMS/Xorg/compositor stack (a forced EDID
   on a headless box: nvkvm-pv drove the NVIDIA DDX on a vast RTX 3090 with `ConnectedMonitor "DFP-0"`,
   `broker-design.md` §7), and `nvdiff` can be extended to follow NVKMS payloads
   (`display_plane_scoping.md` §6.1). (b) has no bare-metal twin on a GeForce.

---

## 4. The design of (a) in v3

§4.2 is the inventory that sizes it; the rest is how each piece sits in the v3 planes.

### 4.1 What decides "this GPU has a display" in a stock GSP-client guest [E]

1. `gpuFuseSupportsDisplay` reads `NV_FUSE_STATUS_OPT_DISPLAY_DATA == ENABLE (0)` at `0x820C04` (GA10x/AD10x/
   GB20x) or `0x21C04` (TU10x); **datacenter chips return false unconditionally**
   (`ogkm-580: generated/g_gpu_nvoc.c:1820-1839`, `arch/ampere/kern_gpu_ga100.c:40-46`). kf3's BAR0 shadow
   serves 0 there today, i.e. *enabled*.
2. `ENG_CLASS_KERNEL_DISPLAY` is present for any GSP client (`gpu.c:6493-6500`) — no `engineCaps` bit.
3. `kdispStatePreInitLocked` asks physical RM `NV2080_CTRL_CMD_INTERNAL_DISPLAY_GET_IP_VERSION`
   (`0x20800a4b`); a failure removes the engine and its classes (`kern_disp.c:324-350`, `gpu.c:2177-2181`).
   ⇒ **Today kayfabe refuses it on purpose** (`crates/kf-rm/src/sweep.rs:705-714`), and that one refusal is
   the whole of today's displayless posture.
4. The IP version selects the kernel display HAL; the valid values per chip are generated
   (`g_chips2halspec_nvoc.h:139-155`): TU10x `0x04000000`, GA102–GA107 `0x04010000` (a real GA106 answers
   exactly this, `crates/kf-abi/src/oracle.rs:321-326`), AD10x `0x04040000`, GB202–GB207 `0x05020000`
   (GB10B `0x05010000`, GB20B `0x05030000`, GB20C `0x05040000`).
5. The guest's class DB then carries the family's display classes (`generated/g_gpu_class_list.c`):

| family | display | core | window | wimm | cursor | caps | SF user | NVKMS HAL |
|---|---|---|---|---|---|---|---|---|
| TU10x | `C570` | `C57D` | `C57E` | `C57B` | `C57A` | `C573` | `C371` | `nvEvoC5` |
| GA10x | `C670` | `C67D` | `C67E` | `C67B` | `C67A` | `C673` | `C671` | `nvEvoC6` |
| AD10x | `C770` | `C77D` | `C67E` | `C67B` | `C67A` | `C773` | `C771` | `nvEvoC6` |
| GB20x | `CA70` | `CA7D` | `CA7E` | `CA7B` | `CA7A` | `CA73` | `CA71` | `nvEvoCA` |

plus `NV04_DISPLAY_COMMON` (`0x73`), `NVC372_DISPLAY_SW` and (GA10x+) `NVC77F_ANY_CHANNEL_DMA`
(`nvkms-hal.c:150-200` maps the display class to the NVKMS HAL).

### 4.2 The emulation surface — the inventory

Three source audits (2026-09-27) sized it: the guest CPU-RM display path (ogkm-580
`src/nvidia/src/kernel/gpu/disp/**`), NVKMS + nvidia-drm (ogkm-580 `src/nvidia-modeset/**`,
`kernel-open/nvidia-drm/**`), and nouveau's GSP-mode display client (Linux 7.3-rc3,
`nvkm/subdev/gsp/rm/r535/disp.c` + `r570/disp.c`) — the one open driver that drives NVDisplay
*through* GSP-RM, i.e. a live statement of what a physical-RM display must answer. Citations are
`ogkm-580:` paths unless marked `nouveau:`.

**(A) Physical-RM controls and allocs (the fake GSP).** About 30 controls, almost all pure data
about a monitor that is ours:

| phase | controls / allocs | answer |
|---|---|---|
| KernelDisplay init | `INTERNAL_DISPLAY_GET_IP_VERSION` `0x20800a4b`, `_GET_STATIC_INFO` `0x20800a01`, `INIT_BRIGHTC_STATE_LOAD` `0x20800ac6`, `SET_STATIC_EDID_DATA` `0x20800adf`, `_WRITE_INST_MEM` `0x20800a49`; alloc `NV04_DISPLAY_COMMON`; `NV0073 SYSTEM_MAP_SHARED_DATA` `0x730151` at load (`kern_disp.c:324-628`) | row data; `NV_OK` — a `NOT_SUPPORTED` on the three fatal ones leaves a half-initialised engine (`gpu.c:2287-2288`) |
| interrupt table | `INTERNAL_INTR_GET_KERNEL_TABLE` must carry `MC_ENGINE_IDX_DISP` (2) with `pmcIntrMask 0` (`intr.c:1031-1090`, `intr_tu102.c:111-132`); GB20x also `DISP_LOW` (177) | **already served**: `authored::with_gsp_and_disp_rows`, vector `0x9a` |
| NVKMS device bring-up | `NV0073 SYSTEM_GET_CAPS_V2`, `SYSTEM_GET_NUM_HEADS`, `SPECIFIC_GET_ALL_HEAD_MASK`, `SPECIFIC_GET_VALID_HEAD_WINDOW_ASSIGNMENT` (windows 2h, 2h+1 → head h), `SYSTEM_GET_SUPPORTED`, `SPECIFIC_OR_GET_INFO`, `SPECIFIC_GET_CONNECTOR_DATA`, `SPECIFIC_GET_TYPE`, `DFP_GET_INFO`, `SYSTEM_GET_BOOT_DISPLAYS`, `SYSTEM_GET_ACTIVE`; `NV5070 SYSTEM_GET_CAPS_V2`; `NVC370 GET_LOCKPINS_CAPS`; allocs `NVC670` (display), `NVC372` (disp SW) (`nvkms-rm.c:1665-1882`, `nvkms-evo.c:5350-5540`) | a failure before the core channel = displayless fallback; after it = nvidia-drm fails to load |
| connector / EDID | `SYSTEM_GET_CONNECT_STATE`, `SPECIFIC_GET_EDID_V2`, `SPECIFIC_SET_EDID_V2` (store/clear custom EDID), `SPECIFIC_GET_PCLK_LIMIT`, `DFP_GET_DISPLAYPORT_DONGLE_INFO`, `SPECIFIC_IS_DIRECTMODE_DISPLAY` (`nvkms-dpy.c:244-3170`) | our EDID; DVI-D on SOR 0, protocol `SINGLE_TMDS_A`, CROSS_BAR clear (no `DFP_ASSIGN_SOR`) |
| mode validation / modeset | `NVC372 IS_MODE_POSSIBLE`, `SYSTEM_GET_HEAD_ROUTING_MAP` (echo), `SPECIFIC_DISPLAY_CHANGE`, `NVC370 SET_SWAPRDY_GPIO_WAR`, `GET_CHANNEL_INFO` (idle = `IDLE`), `GET/SET_ACCL` (`nvkms-evo3.c:3362-3488, 6984-7410`, `nvkms-modeset.c:1439-4210`) | possible; idle from the channel's own state |
| channels | per channel `INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER` `0x20800a58` (aperture, PA, size, class, instance — status ignored), then the alloc RPC with `NV50VAIO_CHANNELDMA_ALLOCATION_PARAMETERS`; FREE before destruct (`disp_channel.c:106-601, 785-863`) | GET = PUT = `offset` at alloc (nouveau:`rdisp` 167-196) |
| events | `POST_EVENT` for `NV2080_NOTIFIERS_HOTPLUG` / `DP_IRQ` (`kernel_gsp.c:484-539`); no vblank or supervisor event exists on dGPU | hot-plug (M5) |

**(B) BAR0 registers the guest reads or writes directly** (`published/disp/v03_00/dev_disp.h` +
deltas; all B-disposition today: shadow reads, trapped writes):

| range | what | model |
|---|---|---|
| `0x680000` core user page (+`0x688000` ARMED, 32 KiB read-only); `0x690000+w·0x1000` windows; `0x6B0000+w·0x1000` window-imm; `0x6D8000+h·0x1000` cursor PIO | PUT `+0x0` (write → post to the display worker), GET `+0x4` (worker publishes), window `GET_LINE` `+0x208`; cursor `Free` `+0x8` (must read ≥ 4), `SetCursorHotSpotPointOut` `+0x208`, `Update` `+0x200` | shadow + trapped write |
| `0x640000` caps page (`NVC673_DISP_CAPABILITIES`) | `SYS_CAP`/`SYS_CAPB` (heads/windows exist, contiguous from 0), `HEAD_CLK_CAP`, `SOR_CAP`, `SOR_CLK_CAP`, HDR/scaler caps, `IHUB_COMMON_CAPA/C` (`nvkms-evo3.c:5524-5978`) | static shadow values, generated from `clc673.h` layouts |
| `0x611C00+4h` / `0x611800+4h` / `0x611D80+4h` / `0x611EC0` / `0x611C30` / `0x611858` / `0x61185C` / `0x611A00` / `0x611868` | head timing status, event status (**W1C**), vblank enable, dispatch, `STAT_CTRL_DISP` (AWAKEN bit 8, WIN_SEM bit 9), AWAKEN window/core masks | status set by the worker; W1C applied **in the trap** (a pure bit operation, like the CPU interrupt tree) |
| `0x612078+h·0x800` `CORE_HEAD_STATE` | `OPERATING_MODE` must read AWAKE (2) for an active head or vblank callbacks never arm (`kernel_head.c:221-281`) | worker publishes |
| `0x6F0000` SF user (`NVC671`) | HDMI infoframes (the HDMI library maps it even on DVI) | accepted, ignored |
| `0x820C04` (TU `0x21C04`) fuse; `0x625F04` VGA workspace | display present; WPR layout | already served (0 = enabled) |
| CPU interrupt tree leaf for vector `0x9a` | the guest's ISR → `kdispServiceInterrupt` | existing `kf_trap::cpuintr` + MSI |

**(C) Memory the guest writes and the display engine reads** — all through the store or guest RAM:
- **Pushbuffers**: 4 KiB **sysmem** per channel (`nvkms-types.h:78`, `nvkms-rm.c:2594-2596`), NVDisplay
  DMA format (`crate kf_disp::pushbuf`).
- **Instance memory**: 64 KiB of FB at `rsvdMemoryBase` (`WRITE_INST_MEM`), a 1024-entry hash table then
  32-byte ctxdma objects (`w0` target/kind, `w1..w4` base/limit `>> 8`) that the guest writes itself and
  never RPCs (`disp_inst_mem_0300.c:94-360`). ⊘ The guest never zeroes it: kayfabe zeroes the range
  (its own store) when `WRITE_INST_MEM` arrives. Lookup = scan for (handle, channel number).
- **Notifiers**: core notifier word0 `STATUS[31:30]` = FINISHED (`0x80000000`) after a synchronous
  UPDATE (`nvkms-dma.c:278-324`); window notifiers with `WRITE_AWAKEN` in a sysmem surface.
- **Semaphores**: nvidia-drm turns every in-fence into a display semaphore (acquire EQ `READY`
  `0xf473f473`, release `DONE` `0xd00dd00d`, 16-byte slots) and **never waits on the CPU**
  (`nvidia-drm-modeset.c:157-466`, `nvkms-kapi-internal.h:49-68`): the display engine must hold the
  flip until the slot reads READY, and release DONE when the surface is flipped **away**.
- **Surfaces**: ctxdma base + `SET_OFFSET(i) << 8`, contiguous, pitch or block-linear by the ctxdma
  kind (C6x) — Blackwell C9x/CAx put the address in the method (`SET_SURFACE_ADDRESS_HI/LO_ISO`).

**(D) The completion path DRM sees** (`nvkms-evo.c:5204-5267`, `kern_disp_0300.c:543-760`,
`nvidia-drm-modeset.c:93-842`): a flip's window notifier is written with WRITE_AWAKEN, the engine sets
`EVT_STAT_AWAKEN_WIN` bit w + `STAT_CTRL_DISP.AWAKEN` and raises the display interrupt; the guest ISR
notifies the window channel's event, NVKMS sends FLIP_OCCURRED, nvidia-drm sends the DRM
flip-complete event. ⊘ 580 nvidia-drm has **no DRM vblank** (`enable_vblank` absent): vblank interrupts
matter only for NVKMS's own vblank callbacks, so first light needs AWAKEN, not vblank.

**(E) Hang hazards** (each loud in the guest's log, none silent): GET must advance (`nvEvoMakeRoom`
spins forever otherwise); a synchronous core UPDATE's notifier must reach FINISHED; `GET_CHANNEL_INFO`
must report IDLE when GET == PUT; the cursor `Free` register must read non-zero.

### 4.3 Display channels are EMULATED channels, executed by a display worker

- A channel alloc arrives as an RPC (`RS_FLAGS_ALLOC_RPC_TO_ALL`) preceded by
  `NV2080_CTRL_CMD_INTERNAL_DISPLAY_CHANNEL_PUSHBUFFER` (`0x20800a58`: aperture SYSMEM/FBMEM, physical
  address, size, class, instance — `ctrl2080internal.h:1343-1400`). kayfabe records a channel descriptor;
  it allocates nothing on the host.
- PUT is a BAR0 write into the channel's user area (`NV_UDISP_FE_CHN_ASSY_BASEADR_*`, `dev_disp.h` v03_00)
  → it traps like every BAR0 write, is posted to the display worker's queue, and the vCPU returns (§41's
  allowlist: a posted write plus a wake).
- The worker reads `[GET, PUT)` from the pushbuffer (guest RAM directly, or the store through the walker's
  bounded `read_store`), decodes methods with **per-family generated method tables**, updates the channel's
  *assembly* state, and on `UPDATE` promotes it to *armed* state (also mirrored into the user area's ARMED
  half, which NVKMS reads — `nvkms-hal.c:130-150`), then advances GET in the shadow page.
- Parsing guest bytes here is allowed and bounded: the channel is an emulated channel with no GPU work
  behind it (`THE_ARCHITECTURE_v3.md` §5), every method index is checked against the generated table, every
  length and address against the channel's pushbuffer and the store/guest-RAM spans, and an unknown method
  is an *error notifier*, never a guess. Only the guest kernel can allocate these channels (§3.3).

### 4.4 Surfaces, context DMAs, notifiers and semaphores

- NVKMS names surfaces through context DMAs bound into display instance memory; the CPU-RM writes the hash
  table and the ctxdma entries itself (`NV_UDISP_HASH_TBL_*`, `NV_DMA_*` in `dev_disp.h` v03_00) and tells
  physical RM where the instance memory is with `NV2080_CTRL_CMD_INTERNAL_DISPLAY_WRITE_INST_MEM`
  (`0x20800a49`, `ctrl2080internal.h:872-895`). kayfabe resolves a handle by reading those entries (bounded
  to the declared instance-memory size) and maps the result — FB offset or guest physical address, base and
  limit, kind (pitch/block-linear) — onto the store or guest RAM, refusing anything outside them.
  ⊘ *2026-10-04 (`v3-cand-1`, §4.11.13's ⊘⊘ block):* a window's ISO context DMA is resolved for its ARMED
  state and kept until the window latches again — as the hardware scans what it latched — so a guest
  that unbinds a context DMA while a window still scans it (NVKMS's teardown does) keeps its picture.
  Notifiers and semaphores still resolve at the moment they are written.
- Completion notifiers and semaphores are written by the worker into the same spans. A write is only ever
  made **after** the thing it reports has happened (§4.5).

### 4.5 Timing: vblank, flip latch and completion — all host events

> ⊘ **CORRECTED 2026-10-04 (§8.16): there is no `timerfd`** — here, in §0's table and in §4.8's
> audit row. Vblank is the display worker's own epoll DEADLINE (`kf_linux_raw` deliberately has no
> timerfd), and since `display-max-fps` each head ticks at the SLOWER of its raster's period and the
> cap's (`kf_disp::pace::Pacer`; unset, the cap is 75 Hz). Everything else in this bullet holds.

- Each active head has a **host `timerfd`** at the mode's refresh (from the programmed raster timings). On
  a tick the worker: latches pending window flips whose acquire semaphore reads READY (read at the tick —
  what the hardware does), writes their notifiers (WRITE_AWAKEN) and sets `EVT_STAT_AWAKEN_WIN` +
  `STAT_CTRL_DISP.AWAKEN`; sets the head's vblank status only if the guest enabled it
  (`0x611D80+4h`, NVKMS's own vblank callbacks); raises the display leaf of the CPU interrupt tree
  (existing `kf_trap::cpuintr`) — one `write(2)` to the MSI eventfd.
- **A surface is released only when kayfabe has finished reading it.** The scanout copy of the newly
  latched surface is queued on kayfabe's CUDA stream; the *previous* surface's release semaphore / flip
  notifier is written when that copy's completion arrives as a host event (`kf_cuda` `launch_host_signal`,
  an fd). The guest can therefore never reuse a buffer the host GPU is still reading, and nothing is
  reported done before the host observed it done — owner rule A.3 applied to display.
- Core-channel `UPDATE`s with no surface work (modeset, LUT, caps) complete at the end of the worker's own
  processing: emulated, nothing behind them.

### 4.6 Scanout and frames out

- **Copy:** a kernel on the walker's CUDA context (the store is already imported there,
  `crates/kf-cuda/src/walk.rs:1748-1760`) reads the latched surface — pitch or block-linear
  (GOB 64 B × 8, block height from the ctxdma/surface parameters), 32-bpp first — converts to XRGB8888 and
  writes a pinned host buffer. Bounds are checked in the kernel against the store and in Rust before launch.
  Cursor composited in the same kernel (first milestone) or sent as a QEMU cursor (later).
- **Present:** the kf3 QOM device registers a QEMU graphic console per head; a bottom half (thread-safe to
  schedule) swaps the finished buffer in and calls the console update. That serves VNC, SPICE, GTK/SDL,
  dbus and `screendump` with no further code. QEMU's `gfx_update` may defer to the next copy so a
  `screendump` always sees a finished frame.
- ⊘ **SUPERSEDED 2026-10-03 by `OWNER_RULINGS.md` §L** (no guest surface or store slice is ever
  exported; the broker gets a GPU copy into kayfabe's own VRAM, §8.11): **Later:** export the latched
  range of the store as a dma-buf with its NVIDIA modifier and hand it to a GL UI
  (`dpy_gl_scanout_dmabuf`, nvkvm-pv's GL zero-copy path), or to nvkvm-pv's broker protocol.
- ⚠ `build_kf3.sh` configures QEMU with `--disable-vnc` and without default features; the display build
  enables `pixman` and `vnc`.

### 4.7 Monitor, EDID, cursor, heads, hot-plug

- The monitor is kayfabe's: an EDID we author (preferred mode = the configured size, default 1920×1080@60),
  on the simplest connector NVKMS accepts (§4.2 decides TMDS vs DP). Resize = a new EDID + a hot-plug event
  (the QEMU UI's `ui_info` gives the window size).
- Cursor: the cursor channel is PIO — its writes trap and are posted like PUT; the image comes from a ctxdma.
- Heads: the family's real head count is advertised in the capability registers; only connected monitors
  get a QEMU console.

### 4.8 Constraint audit

| owner rule | how (a) meets it |
|---|---|
| host verbs authored, never forwarded | no guest display control or method reaches the host; the only host work is kayfabe's own CUDA copy of a surface from its own store |
| completions are host events, never forged | vblank from a host `timerfd`; a flip's release waits for kayfabe's copy completion fd; pure-state `UPDATE`s complete after kayfabe's own processing (emulated channel) |
| no blocking on a vCPU / under a vCPU lock | PUT and cursor writes post to a queue and return; the worker, timers and CUDA work run off-vCPU; shadow registers are written by the worker |
| VMM addresses never guest-chosen or visible | surfaces are guest FB offsets / guest physical addresses, translated inside kayfabe with bounds; nothing about the VMM is exposed |
| all families first-class; derive per die, maintain per family | IP version, class set, method and register layouts generated from ogkm; the monitor is ours, so there is no per-die display fact |
| hostile guest | channels are privileged allocs; every method, length and address bounded; a bad stream sets an error notifier; nothing guest-controlled sizes a host allocation (fixed per-head buffers) |
| no CPU executor for GPU work | the display engine has no GPU work; the scanout copy runs on the GPU |

### 4.9 What does not change

Compute, the memory plane, passthrough/translated channels, the headless graphics set: a display-enabled
guest adds KernelDisplay, NVKMS and one worker. The merge bar (crate tests, gates 9/9, thin guest 30/30,
CUDA ladder, headless set 38/38) must stay green with display on.

### 4.10 Families

- **TU10x, GA10x, AD10x, GB20x:** (a), generated per family; GA10x first (the bench), then AD10x (shares the
  C6 NVKMS HAL and the C67E window class), GB20x (CA HAL), TU10x (C5 HAL).
- **GA100, GH100, GB100/GB102/GB110/GB112:** displayless by the guest driver's own HAL — no emulation can
  give them a head without lying about the chip. (b) is their display, exactly as on bare metal.

### 4.11 Boot display: kf3's UEFI GOP option ROM (display step 1)

**STATUS: REVIEWED, FIXED AND RE-RUN, 2026-10-03 (late), branch `v3-gop-unload`, kf3 `06b307c4`** (box
54032077; `traces/v3_display/gop_final_20261003/`). The adversarial review of the B5 work (one MEDIUM, eight
lower) and the five minor findings of the `v3-gop` re-review are fixed; each correction is folded above
what it corrects (§4.11.3, §4.11.6, §4.11.12, §4.11.13). Measured at `06b307c4`: **B1** — no black frame
between the boot layer and the first armed head, from kf3's own lines (`DISPLAY_BOOT_HANDOFF
black_frames=0`; the boot layer's last frame stays 1 ms until head 3's window), probe pixel-exact, 120/120
flips, B2's checks as before; **B0** (`gop=off`) — pixel-exact, 120/120 flips, no seed; **B5** —
`DISPLAY_B5_VERDICT PASS arms=14 failed=0`, the lane's rc graded (it was always 0). The first try at
`445367a8` failed ONE arm, and the fault was the hook's (`qsince | grep -q` under `pipefail`), not kf3's —
the new verdict is what showed it. Locally: the Secure Boot deny arms with their positive control (4
PASS, 1 OBSERVED; the bite FAILs). Not run: GB20x or any non-GA10x box; B5 (d). The STATUS just below is
superseded where §4.11.13 says so (the handoff black, *"the log lines are bounded"*, the (a2) cause).

(superseded the same day where §4.11.13 corrects it) **STATUS: B5 (unload) MEASURED AND FIXED, 2026-10-03, branch `v3-gop-unload`** (box 54032077, kf3
`4a4b95f7`; `traces/v3_display/gop_unload_20261003/`): the guest's RM gives BAR1 up at every teardown and
kf3 now returns BAR1 `[0, G)` to its physical view then; nothing scanned is a black frame; a scanout
freed with `PRESERVE_HW` stays. B1 and B0 re-run at the same binary pass. The causes, the trigger and
what is not modelled: §4.11.13. ⊘ The STATUS below says *"nothing has run on a GPU box"*: B0–B3 ran the
same day (`traces/v3_display/gop_box_20261003/`).

**STATUS: BUILT, 2026-10-03 (late), branch `v3-gop`; nothing has run on a GPU box.** The two halves below
plus `OWNER_RULINGS.md` §K (no committed binary; arch-neutral ROM) and the review's fixes — what changed
against the text below is §4.11.12's ⊘ block, and each changed statement carries its own ⊘ note.

(superseded the same day, kept as written) **STATUS: BOTH HALVES BUILT, 2026-10-03 (branches `v3-gop-rom` and `v3-gop-kf3`); nothing has run on a
GPU box.** The kf3 integration — ROM BAR, BAR1 seed, boot layer, fn 72 → fn 65 console region, property
`gop` — is §4.11.12, which also lists what it changed against the design below. Box tests in order:
§4.11.9.

(superseded the same day, kept as written) **STATUS: FIRST HALF BUILT, 2026-10-03 (branch `v3-gop-rom`).** Built and tested without a GPU: the
GOP firmware (`firmware/kf-gop`, its release build committed and embedded in `crates/kf-oprom` per
the owner's decision of 2026-10-03, §4.11.6), the ROM container and packer (`crates/kf-oprom`), and the local
stand-in (`scripts/display/gop_standin.sh`, 11/11 arms at `3dd574e5`, §4.11.8). **Not built:**
everything inside kf3 — the ROM BAR, the BAR1 seed, the boot layer, the fn 72 → fn 65 region table.
§4.11.2–§4.11.6 are the design that integration follows; it lands on top of this branch. Nothing here
has run on a GPU box. The design was revised after an adversarial review on 2026-10-03; its
corrections to this document are folded into the NEXT block above, and into §4.11 below.

Sources: `ogkm-580` = `research_clones/ogkm-580.159.04`; Linux 7.1-rc6 (`research_clones/linux`);
QEMU 10.2.4 and 11.1.1 sources; EDK2 **edk2-stable202408**, which *is* local, under QEMU 10.2.4's
`roms/edk2` (⊘ the design said EDK2's source was not local; §4.11.7 reads it).

#### 4.11.1 Why the ROM, after the boot_vga correction

- The premise that no device is `boot_vga` is wrong on today's SeaBIOS bench (the ⊘ note at the top
  of this document): kf3 already is the boot VGA device and Xorg already marks it primary.
- The ROM is still needed for four things:
  1. Windows: setup, boot and safe mode run on the GOP before `nvlddmkm` starts;
  2. any picture on kf3 before nvidia-drm loads (OVMF, the boot loader, early kernel output, and a
     CUDA-only guest's console);
  3. OVMF guests, where without a GOP nothing promises kf3 gets the firmware framebuffer;
  4. guests with a second VGA device, where the firmware framebuffer decides (`vga_is_firmware_default`,
     `drivers/pci/vgaarb.c:566-572`).
- Item 4, locally: with a driverless VGA device that decodes I/O and memory in a lower slot, Linux 7.0's
  vgaarb first chose that device, then moved boot VGA to the device holding the EFI framebuffer
  (*"setting as boot VGA device (overriding previous)"*; 2026-10-03, `3dd574e5`, arm `linux_two_vga`).
- First box step: **B0a**, which needs no build (§4.11.9).

#### 4.11.2 Where the framebuffer lives

**kf3 BAR1, offset 0, size G**, with `pitch = align_up(4·W, 256)` and `G = align_up(pitch·H, 64 KiB)`
(0x7F0000 at 1920x1080; `kf_oprom::Geometry::for_mode`, unit-tested). Why each:

- **Offset 0.** `bPreserveBar1ConsoleEnabled = (fbBaseAddress == nv->fb->cpu_address)`
  (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osinit.c:1073-1079`); `NV_IS_CONSOLE_MAPPED` accepts only
  the BAR1 base or BAR2 base + 16 MiB (`ogkm-580: kernel-open/common/inc/nv.h:748-750`); kbus maps the
  console at BAR1 VA 0 with `MAP_OFFSET_FIXED` and 4 KiB pages before anything else
  (`ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:1084-1162`, flags `:1098`).
- **FB physical 0.** The console memdesc is all of `fbRegion[0]` (`mem_mgr_gm107.c:2068-2110`; the HAL
  serves TU102 … GB207), and a present console is region 0 (`arch/turing/mem_mgr_tu102.c:648-653`). Since
  store offset = guest FB address (`THE_CONSTRAINTS.md:1009-1026`): BAR1 offset 0 ↔ FB 0 ↔ store 0.
- **Size.** `ReservedConsoleDispMemSize = NV_ALIGN_UP(fbConsoleSize, 64 KiB)` (`osinit.c:1092`); the
  pitch rounding is NVKMS's (`ogkm-580: src/nvidia-modeset/src/nvkms-rm.c:4880-4884`). `nv_get_screen_info`
  (`ogkm-580: kernel-open/nvidia/nv.c:6172-6309`) has three paths: a registered fbdev on the BAR1 base;
  `screen_info` (compiled only if exported, `nv.c:6234`); the BAR1 resource's first `…fb`/`…FB` child
  (`nv.c:6271-6305`) — the only one on Linux 7.1, which exports `sysfb_primary_display` instead
  (`arch/x86/kernel/setup.c:216-217`). Every path yields at most G. The EFI stub sets
  `lfb_size = linelength × height` (`drivers/firmware/efi/libstub/gop.c:412`); locally the BOOTFB
  resource was exactly 4608 × 648 = 2 985 984 bytes at BAR + 0 (2026-10-03, `3dd574e5`, arm `linux`).
- **Order and passthrough.** `RmSetConsolePreservationParams` runs before `kgspInitRm` (`osinit.c:1993-1995`
  vs `:2024`); `GspSystemInfo.consoleMemSize` (`src/nvidia/src/kernel/vgpu/rpc.c:10585`) reaches us in
  fn 72, which arrives **before** fn 1 and fn 65 (`kernel_gsp.c:4141` vs `:4225`, `:4232`). Under KVM the
  guest is "passthrough" (`gpu.c:4711-4774`), so `RmDeterminePrimaryDevice` returns early
  (`osinit.c:983-991`): console preservation still runs (`:1031-1052`), and console access is **not**
  disabled during GSP/BAR1 setup (`:2003-2007`) — firmware-console writes keep arriving through BAR1.
  The display fuse already reads ENABLE wherever `display=on` works (`kern_disp.c:333`).

| item | value | why |
|---|---|---|
| BAR | PCI BAR1 (`kf3.c:817-818` at `v3-gop-kf3`) | the equality test above; Linux ties the firmware framebuffer to the BAR that contains it (§4.11.8) |
| offset | 0 | `osinit.c:1079`; `kern_bus_gm107.c:1131-1153` |
| size G | `align_up(align_up(4W,256)·H, 64 KiB)` | `osinit.c:1092`; `nvkms-rm.c:4880-4884` |
| backing at boot | store `[0,G)` **zeroed by a GPU write** (the verb that zeroes the roots, `crates/kf-qemu/src/device.rs:337-345`), then one host CPU view of store `[0,G)` placed at BAR1 offset 0 at realize, before any vCPU runs (`WindowOps::arm_store` + `place_view`, `crates/kf-qemu/src/mem.rs:344-357`) | real VRAM, the pages RM will call FB 0; no copy at handover; no exits; nothing from before the VM visible (kayfabe never scrubs the store otherwise, `crates/kf-host/src/lib.rs:1672-1730`) |
| refused | host-RAM backing | sysmem standing in for vidmem (`THE_CONSTRAINTS.md:120-125`, items 18, 22); breaks FB-0 identity; needs an owner ruling (`OWNER_RULINGS.md` A.11), never a fallback |

★ Built as designed on `v3-gop-kf3` (§4.11.12): `CpuWindow::seed`/`retire_seed`
(`crates/kf-mem/src/cpuwin.rs`) and the boot layer (`kf_disp::scanout::boot_layer`, `Shown` in
`crates/kf-qemu/src/display.rs`); the release at stop is the VA thread's (§4.11.12, deviation 2).

⊘ **CORRECTED 2026-10-03 (B5, §4.11.13) — the lifecycle below has one life; the guest's RM has many.**
The guest RM initialises the adapter at the first open and tears it down at the last close (five RM lives
in one B5 boot), and every teardown unmaps the console at BAR1 VA 0 and writes the BAR1-mode register back
to PHYSICAL. The seed is now placed AGAIN then (`CpuWindow::reseed`, on the VA thread) and retires again
exactly as below when an RM takes BAR1 back (`[measured b5f]` *"… ONE run, VA 0 -> store 0 … (seed life
N …)"*).

**The seed's lifecycle.** Today every BAR1 page shows the per-BAR scratch until the walker reports the
guest's BAR1 page tables (`MemPlane::build`, `mem.rs:1544-1566`; `CpuWindow::map`/`unmap`,
`crates/kf-mem/src/cpuwin.rs:163-240`; `Bar1Target`, `mem.rs:1126-1208`).
- The seed is `CpuWindow::seed(at=0, len=G, view)`, kept outside `placed`, so the walk/diff ledger is
  untouched.
- It retires in the existing batch-end hook `MapTarget::invalidate` (called once per batch after the
  maps whenever anything changed: `ledger.rs:202-206`, `apply.rs:528-538`), on the **first BAR1 batch
  that changes anything, at any VA** — the closest observable equivalent of BAR1 going virtual. (⊘ The
  first design retired it on a batch overlapping `[0,G)`; with C = 0 or a first mapping elsewhere that
  kept FB `[0,G)` visible while RM treats it as free heap.)
- In that batch: the guest's views are placed; every part of `[0,G)` no placement covers sinks to
  scratch; only then is the seed released (the scratch-first rule, `cpuwin.rs:224-230`).
- If RM preserved the console, its first BAR1 map is FB 0 → VA 0 (`kern_bus_gm107.c:1084-1092`): the
  new view shows the same store pages, so the handover is invisible. The walker must coalesce the
  4 KiB-PTE console map into one run, or host BAR1 fills with G/4 KiB views (box test B2 checks it).
- A seed never retired (the guest never loads nvidia.ko) is released in `kf3_unrealize`, after QEMU
  has unmapped the BAR.

**How kf-disp shows it before the first modeset.** New `Shown::{Armed(Composition), Boot(LayerPlan,
(W,H))}`; the loop chooses `Boot` **before** its `dp.scan` gate (`display.rs:1450-1452`) while no head has
ever been armed — otherwise GB20x, whose `ScanVocab` is `None` (`engine.rs:1208-1209`), would never show
it. `ScanState::start` takes `Shown`; the `Boot` arm skips `io.resolve`/`plan_layer`. The plan comes from
a new pure `kf_disp::scanout::boot_layer(geom)` (src 0, pitch P, W×H, opaque XRGB8888, extent ≤ G,
`W·H ≤ MAX_PIXELS`, refused by name otherwise). A sticky `boot_done` is set at the first armed head.

#### 4.11.3 What the guest RM does with it

⊘ **CORRECTED 2026-10-03 (B5, measured on box 54032077; §4.11.13) — three statements below.**
- *"efifb or simpledrm keeps drawing through BAR1 VA 0 for the VM's life"* holds only while some RM
  client keeps the adapter up. With no client (a CUDA-only guest between jobs) RM is torn down, its console
  mapping is gone and BAR1 is in physical mode, which kf3 now models for `[0, G)`; the console keeps
  drawing either way.
- *"which kf-disp follows as a window at FB 0"* (`modeset=0`): measured true (`window 6 store 0x0`), and
  NVKMS then frees its channels with `PRESERVE_HW` (`NV5070_CTRL_CMD_SET_RMFREE_FLAGS`), which kf-disp
  now honours. ⊘ *CORRECTED 2026-10-03 (late, the review of `v3-gop-unload`):* *"before, the scanout
  ended at the free"* was never measured — no kept run had X on NVKMS without the fix (§4.11.13) — and
  by the code before the fix an unclaimed free left the last frame frozen, not dropped.
- `modeset=1 fbdev=1`: the eviction also makes NVKMS drop its console surface
  (`nvRmUnmapFbConsoleMemory`, `ogkm-580: src/nvidia-modeset/src/nvkms-rm.c:4967-4998`), so removing
  nvidia-drm restores nothing: NVKMS shuts the heads down and the screen is black, as on bare metal.

- **Linux, nvidia.ko only** (a CUDA guest with a firmware console): the guest sends C in fn 72; region 0
  is the console (§4.11.4); `memmgrAllocateConsoleRegion` describes it (`mem_mgr.c:672-678`); `kbusInitBar1`
  maps it at VA 0 with 4 KiB PTEs (`kern_bus_gm107.c:1098`, `:1131-1155`); the seed retires; efifb or
  simpledrm keeps drawing through BAR1 VA 0 for the VM's life.
- **Linux + nvidia-drm `modeset=1 fbdev=1`** (`fbdev` defaults on, `nvidia-drm-os-interface.c:41-42`):
  NVKMS reads `VT_GET_FB_INFO` and imports the console (`nvkms-evo.c:5383`, `:9261`), then
  `drm_dev_register`, aperture eviction and `framebufferConsoleDisabled` (`nvidia-drm-drv.c:2026-2049`)
  lead to `NV0076_CTRL_CMD_NOTIFY_CONSOLE_DISABLED` → `kbusUnmapPreservedConsole` (`nvkms.c:4988-5009`;
  `nvkms-rm.c:4967-4995`; `kern_bus_gm107.c:1283-1310`): BAR1 `[0,C)` sinks to scratch, the memdesc stays
  reserved. Until `drm_client_setup`'s first modeset (`:2073`) kf-disp shows the last boot frame.
- **`modeset=0` + the NVIDIA X driver**: NVKMS imports the console only if `width != 0`
  (`nvkms-rm.c:4892-4902`) and restores it on last close with a core-channel modeset
  (`nvkms.c:1107-1118` → `nvkms-console-restore.c:756`), which kf-disp follows as a window at FB 0.
- **Guest-kernel scope.** Without an exported `screen_info` (Linux 7.1 here) path 3 gives width 0: no
  NVKMS import, no `NOTIFY_CONSOLE_DISABLED`, the BAR1 console mapping lives for the driver's life, and
  there is no NVKMS console restore. Bare metal behaves the same. The bench's noble 6.8 guest still
  exports `screen_info`. Which kernel first dropped it is not established.
- **Windows** (closed; not established): if the KMD sets `uefiScanoutSurfaceSizeInMB`, RM describes FB
  `[0,size)`, maps it at BAR1 VA 0 and allocates the rest from the top of BAR1 (needs BAR1 < 4 GiB,
  `kern_bus_gm107.c:983-1020`, `:1100-1110`). The layout is what it expects.

#### 4.11.4 Every kayfabe answer that must agree with the ROM

⊘ **CORRECTED AGAIN 2026-10-03 (late, `v3-gop`) — the next note's *"`consoleMemSize` is still kept and
decoded, and a non-zero value is LOGGED"* no longer holds.** With `gop=off` kf3 now attaches **no**
`SystemInfoCell` and **no** `ConsoleSeat` (`crates/kf-qemu/src/gop.rs`, `ConsoleWiring`): fn 72 is dropped
unread, exactly as before the boot display, and fn 65 is today's table with nothing decoded or logged
(test `gop::tests::gop_off_wires_no_cell_and_no_seat`). So B0 can no longer read C from the QEMU log
(§4.11.9's B0 note); rows 1–2 still apply with `gop=on` only.

⊘ **CORRECTED 2026-10-03 (as built, `v3-gop-kf3`; deviation 1 of §4.11.12) — rows 1–2 below apply with
`gop=on` only.** The reviewed design applied the region-0 rule whether or not the ROM is in use. As built,
with `gop=off` fn 65 serves today's table whatever fn 72 says: `consoleMemSize` is still kept and decoded,
and a non-zero value is LOGGED (*"GET_GSP_STATIC_INFO (gop=off): the guest preserves a firmware console
of … bytes"*), never acted on — so `gop=off` is byte-identical to today
(`crates/kf-rm/tests/console_region.rs::without_a_console_or_with_gop_off_the_reply_is_byte_identical`).
Without kf3's ROM nothing places a framebuffer at kf3's BAR1 base, and `primary_vga` is never set under
passthrough (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osinit.c:983-991`, consumed at `:1087-1090`), so C = 0 is
expected there; B0 reads the log line to check it. Rows 3–5 (NICE) are not built.

| # | answer | where | value |
|---|---|---|---|
| 1 | `GspStaticConfigInfo.fbRegionInfoParams` — **MUST** | today `fb_layout` (`crates/kf-chip/src/bar0.rs:403-434`) → the immutable `Arc<BoardFacts>` (`crates/kf-rm/src/lib.rs:47`, `crates/kf-qemu/src/device.rs:411-412`) → `StaticInfoPolicy::body`/`body_measured` (`crates/kf-rm/src/staticinfo.rs:173-197`, `:213-231`) | computed **per fn 65** as `fb_layout_with_console(fb_length, C)`: with C > 0, region 0 = `[0,C)` reserved (`bRsvdRegion`, `mem_mgr_gsp_client.c:91-101`), ISO yes, compressed no; region 1 = `[C, carve)`; carve-out and the BAR1/BAR2 PDE bases unchanged (`memmgrCalculateHeapOffsetWithGSP_TU102` takes its region-0 branch, `mem_mgr_tu102.c:672-680`). With C = 0: byte-identical to today. ⊘ Without it, region 0 (≈7.7 GiB) becomes the console memdesc and `kbusInitBar1` fails (`mem_mgr_gm107.c:2079-2101`; `kern_bus_gm107.c:1131-1140`) — read from source, not run |
| 2 | fn 72 `GspSystemInfo.consoleMemSize` — **MUST**, read lazily | fn 72 is NoReply and `boot.rs` returns before any policy sees it (`crates/kf-gsp/src/boot.rs:2080-2096`) | copy fn 72's body (bounded by the largest GspSystemInfo in the matrix) into a shared cell that survives `ReselectAtFn1`, like census and inbox (`device.rs:557-568`); decode at fn 65 with **the table that serves fn 65** — fn 72 precedes the fn-1 re-select, which does not compare GspSystemInfo (`crates/kf-abi/src/versions.rs:511-550`), and the offset differs by version (`generated/matrix.rs:2125`, `:2411`, `:2899` → 48/56/64). Refuse fn 65 by name unless C is a 64 KiB multiple, `C < carve`, `C ≤ bar1-size`; log C against G. A later fn 72 replaces the cell |
| 3 | `bIsGpuUefi`, `bIsEfiInit` — nice | `gsp_static_config.h:161-162`; read only by `unix_console.c:52`, `:95`, `:179` | TRUE with `gop=on`; kf-disp already answers `PRE_UNIX_CONSOLE` with `bReturnEarly` (`crates/kf-disp/src/model.rs:123-131`, `:820-827`) |
| 4 | `NV2080_CTRL_CMD_BIOS_GET_UEFI_SUPPORT` (0x2080080b) — nice | GSP-routed (`generated/g_subdevice_nvoc.c:1916-1928`) | `PRESENCE_YES \| IS_EFI_INIT_TRUE` with `gop=on` (`ctrl2080bios.h:417-480`) |
| 5 | `NV0073_CTRL_CMD_SYSTEM_GET_BOOT_DISPLAYS` — nice | `bootDisplayMask = 0` today (`model.rs:642-643`) | the connector's display id with `gop=on` (console-restore pass 2, `nvkms-console-restore.c:824-857`) |
| 6 | display fuse | already ENABLE wherever `display=on` works (`kern_disp.c:333`) | nothing new |
| 7 | NV_PROM VBIOS | `crates/kf-abi/src/vbios.rs:880-893` | unchanged (§4.11.6) |
| — | `uefiScanoutSurfaceSizeInMB` | CPU-RM | nothing to answer (the ⊘ note at the top) |
| — | `VT_GET_FB_INFO` | CPU-RM (`unix_console.c:332-381`) | agrees automatically |

⚠ **HIGH risk, from source:** shipping the ROM without row 1 breaks guest driver load. Rows 1–2 land
with the kf3 integration, together with the ROM BAR, never after it.

#### 4.11.5 Constraints, and how the design meets them

- **No traps in BAR1/BAR2/PRAMIN** (`THE_CONSTRAINTS.md:72-73`, §23): the seed is a placement in the
  existing BAR1 memslot; the ROM touches no BAR0. The ROM BAR is read-only RAM: reads never exit; a write
  exits to QEMU core and is discarded there (not a kf3 path). It becomes a memslot whenever the guest
  enables ROM decode (OVMF does, briefly; Linux's x86 fixup then replaces the resource,
  `arch/x86/pci/fixup.c:381-397`) — setup-time only; churn a guest causes by toggling is self-harm.
- ⊘ *2026-10-03 (B5, §4.11.13): plus one view (RM map + `mmap`) at each RM teardown and its retirement at
  the next RM init, on the VA thread; the budget term below already covers the overlap.*
- **Memslots are setup-only** (§16): no new BAR1 memslot; one `mmap(MAP_FIXED)` at realize and one at
  retirement, on the VA thread.
- **No blocking on a vCPU or under a lock** (A.4, §25): no new trap; fn 72 stash = a bounded copy on the
  GSP queue thread; seed and zeroing on the realize thread; retirement inside the existing `invalidate`;
  the boot layer on the display worker.
- **VMM addresses never guest-visible** (A.5): the descriptor carries a BAR index and an offset;
  `FrameBufferBase` is the guest-physical address OVMF assigned.
- **One store; vidmem is vidmem** (`THE_CONSTRAINTS.md:1009-1026`, items 18, 22): the seed is a store
  view; host RAM refused (§4.11.2). **The CPU never reads guest vidmem** (§38): GPU zeroing and GPU copy;
  the firmware's own reads are the guest's CPU, kept rare by the RAM shadow.
- **No hard-coded chip** (§12, A.7): the PCIR ids and class come from the host identity (`kf3.c:796-802` at `v3-gop-kf3`);
  the firmware names no vendor (it matches the descriptor's ids against config space). Displayless dies
  refuse `display=on` (`device.rs:518-525`); `gop=on` requires `display=on`.
- **§13 unsafe**: the packer is safe and pure; the firmware is the named exception under `firmware/`
  (owner default 2026-10-03; `firmware/README.md`), contained by CI (§4.11.6).
- **Host BAR1 budget** (`crates/kf-qemu/src/cardbudget.rs:1-50`): add G, briefly 2G during retirement,
  when `gop=on`; refuse by name.
- **Hostile guest** (A.9): the ROM is VMM-authored and read-only; the scanout extent is the authored G;
  C shapes only the guest's own region table. **No forged completions** (A.3): none involved.
- **Licence** (§G): every new file carries `SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later`.

#### 4.11.6 The ROM, its container, and how kf3 serves it

⊘ **CORRECTED 2026-10-03 (late, `v3-gop`; `OWNER_RULINGS.md` §K) — four statements in this section no
longer hold, each marked where it stands:**
- **Nothing compiled is committed.** `firmware/kf-gop/kf-gop.efi` is deleted, and no commit of `v3-gop`
  contains it. `crates/kf-gop-image/build.rs` builds `firmware/kf-gop` (`--bin kf-gop`, release, `--locked`)
  in a nested cargo under `OUT_DIR` for `<CARGO_CFG_TARGET_ARCH>-unknown-uefi`, with the environment
  scrub, recursion guard and kept `RUSTUP_TOOLCHAIN` of `crates/kayfabe-isolate-host/build.rs`, refuses the
  result unless `kf_oprom::pe::check` accepts it with the arch's machine, and embeds it as
  `kf_gop_image::KF_GOP_EFI`. A toolchain without the UEFI target is a named build error that prints
  `rustup target add <triple> --toolchain <t>`; `rust-toolchain.toml` lists `x86_64-unknown-uefi`. Why a new
  crate and not `kf-oprom`: `kf-oprom` is in the PURE set (whose rule counts build edges) and the firmware
  depends on it, so a build script there would run inside every build of the driver it builds.
- **The ROM is arch-neutral.** `EfiMachineType` is the PE's own COFF `Machine`; `pe::check` accepts
  0x8664 and 0xAA64 and refuses every other machine by name (`PeError::UnsupportedMachine`).
- **x86 port I/O is behind `cfg(target_arch = "x86_64")`**: the `debugcon` feature and the stand-in's
  test app are named build errors elsewhere; the release driver builds for `aarch64-unknown-uefi` (9 216
  bytes, machine 0xaa64, built locally 2026-10-03; CI's `aarch64` job builds it on every push).
- **The framebuffer is written by aligned volatile 32-bit stores only**: `blt` sees it as a
  `Framebuffer` whose one operation is a word store (never a slice, so never `memcpy`/`memset`), and
  `probe::place` refuses a framebuffer base that is not word-aligned.

**The firmware, as built** (`firmware/kf-gop`, zero external crates, `no_std`, `x86_64-unknown-uefi`;
release `kf-gop.efi` 9 216 bytes, ⊘ *(no longer committed or embedded by `kf-oprom` — the note above)*
committed as `firmware/kf-gop/kf-gop.efi` and embedded by
`kf-oprom`, see *How kf3 serves it*):
- `efi_main` installs the driver binding and ComponentName2 on its image handle and returns.
- `Supported` opens PCI I/O `BY_DRIVER`, reads config dwords 0x00 and 0x08, and accepts only a
  display-class controller whose `RomImage` holds a valid `KFGP` descriptor naming **that controller's
  own vendor and device** (`kf_gop::probe::decide`); everything else is refused by name (debug builds
  print the name).
- `Start` enables `EFI_PCI_IO_ATTRIBUTE_MEMORY` **only** (no I/O decode, no bus mastering; the previous
  attributes are saved), reads the BAR's base and length from `GetBarAttributes`, refuses a framebuffer
  that does not fit it, allocates a context and a RAM shadow (`EfiBootServicesData`), creates one child
  (the parent's device path + ACPI `_ADR` 0x80010100, the value QemuVideoDxe uses, `OvmfPkg/QemuVideoDxe/Driver.c:389`), installs device path,
  GOP, `EDID_DISCOVERED` and `EDID_ACTIVE` on it, opens PCI I/O `BY_CHILD_CONTROLLER`, and clears to black.
  `Stop` reverses it and restores the attributes.
- GOP: `MaxMode = 1`, `PixelBlueGreenRedReserved8BitPerColor`, `PixelsPerScanLine = pitch/4`,
  `FrameBufferBase` = BAR + offset, `FrameBufferSize` = G. `QueryMode` returns a pool copy. `Blt` does all
  four operations with the spec's parameter rules; writes go to the shadow and the framebuffer, reads come
  from the shadow; every GOP call runs at `TPL_NOTIFY`. ⚠ A loader that writes `FrameBufferBase` directly
  and later reads through `Blt` sees stale shadow pixels (cosmetic).
- Debug output on port 0x402 exists only in `--features debugcon` builds; the release build contains no
  port I/O (checked in CI with `objdump`).
- Every decision is in the safe library (`src/lib.rs`, `forbid(unsafe_code)`, 14 host tests; ⊘ 17 since
  `v3-gop`); the 77 (⊘ 78 since `v3-gop`: the framebuffer's volatile word store)
  relaxations sit in four `*_unsafe.rs` files, counted by the CI ratchet (`firmware/kf-gop:77`); a
  `#[repr(C)]` under `firmware/` outside `*_unsafe.rs` fails the ABI-quarantine gate.

**The container, as built** (`crates/kf-oprom`: pure, `no_std` + `alloc`, `forbid(unsafe_code)`):

| offset | content |
|---|---|
| 0x00 | `55 AA`, `InitializationSize`, `EfiSignature 0x0EF1`, `EfiSubsystem 0x0B`, `EfiMachineType` = the PE's `Machine` (⊘ was the constant 0x8664 until `v3-gop`), `CompressionType 0`, reserved, `EfiImageHeaderOffset` (0x16), `PcirOffset 0x1C` (0x18) |
| 0x1C | PCIR rev 3, length 0x1C: host vendor, device and class, `ImageLength`, code type 3, indicator 0x80 (last image) |
| 0x40 | `KFGP` v1 at `(PcirOffset + len + 0xF) & !0xF` (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_vbios_tu102.c:331`): magic, version, length, vendor/device echo, BAR, format (1 = XRGB8888), flags (0), offset, G, W, H, pitch, EDID length and EDID (≤ 256 bytes), checksum byte — layout in `crates/kf-oprom/src/desc.rs` |
| `EfiImageHeaderOffset` | the PE, byte-identical to the shipped `.efi`, **ending exactly on the last 512-byte block** |

- ⊘ *Deviation from the reviewed design, 2026-10-03:* the design put the PE at 0x200 with padding after
  it. EDK2 hands `LoadImage` `[EfiImageHeaderOffset, InitializationSize·512)`
  (`MdeModulePkg/Bus/Pci/PciBusDxe/PciOptionRomSupport.c:85-90`) and its Authenticode check hashes every
  byte past the sections that is not the certificate table
  (`SecurityPkg/Library/DxeImageVerificationLib/DxeImageVerificationLib.c:556-588`), so padding after a
  signed PE would break its signature on firmware that verifies option ROMs. kf-oprom pads **before**
  the PE. An unsigned linker output is already a multiple of 512 bytes, so for it the layout is the
  design's (PE at 0x200). OVMF does not verify option ROMs at all (§4.11.7), so the stand-in cannot
  tell the two layouts apart; the arm records that. ★ *Tested 2026-10-03 (late), F1 at `adbe6fcd`
  (§4.11.9): on a ROM-verifying OVMF the signed end-aligned ROM runs and the same PE with padding after
  it does not — the layout's premise holds (`traces/v3_display/gop_standin_20261003_v3gop/`). This
  closes the ⊘ "untested" sentence right after it* (folded 2026-10-03, the review of `v3-gop`: the two
  read as a contradiction). ⊘ *2026-10-03 (late, `v3-gop`; SUPERSEDED the same day by F1, the ★ note
  just before):* so the layout is **untested against a firmware that denies unsigned ROMs** — it rests
  on reading `DxeImageVerificationLib.c` alone. The stand-in's `sb_deny_*` arms test it on an OVMF built with
  `PcdOptionRomImageVerificationPolicy=0x04` when one is supplied (`OVMF_DENY_CODE`; test F1, §4.11.9).
- `parse` accepts PCIR revision 0 and 3 and reports compression: QEMU's own `efi-virtio.rom` (read
  2026-10-03) has two images, the EFI one with PCIR rev 0, length 0x18, compression 1, header offset
  0x38 — a golden test (`crates/kf-oprom/tests/efi_virtio_golden.rs`).
- `pack` refuses by name: a PE that is not PE32+/x86-64 (⊘ *`v3-gop`:* or AArch64)/subsystem 11 or lacks the ABI marker
  `KFGOP-ABI=1` (so a VMM never packs a descriptor version its firmware would refuse); a non-display
  class; a bad geometry (`pitch % 256`, `G ≥ pitch·H`, dimensions ≤ 16384); an EDID over 256 bytes; a ROM
  over 0xFFFF blocks.

⊘ *2026-10-03 (late, `v3-gop`):* `pack_kf_gop` and `KF_GOP_EFI` moved to `crates/kf-gop-image` (same
signatures; `kf_gop_image::TARGET` names the triple); `kf-oprom` keeps `pack(pe, …)` and the rest.

**The API the kf3 integration uses** (`crates/kf-oprom`, re-exported at the crate root):
`Geometry::for_mode(W, H)` → `BootFramebuffer { bar: 1, offset: 0, geometry }`;
`pack_kf_gop(&Identity { vendor, device, class: ClassCode::from_u24(class) }, &fb, edid) ->
Result<Vec<u8>, PackError>` packs the embedded driver `KF_GOP_EFI` (`pack(pe, …)` takes any PE, for
tests); `BootFramebuffer::fits(bar_len)`; `Descriptor::find(rom)` and `rom::parse` for tests. The
geometry and EDID come from kf-disp's `Monitor` (`crates/kf-disp/src/edid.rs:145-175`, 128-byte EDID).

**How kf3 serves it** (design, for the integration — ★ built on `v3-gop-kf3`, §4.11.12):
- ★ **DONE 2026-10-03 (late) on `v3-gop`** — the next bullet's *"⚠ Not done on either branch yet"* is
  closed: see the ⊘ block at the top of this section. kf3 needed no change for it, as predicted: Rust
  reads the embedded driver and hands C the finished ROM.
- ⊘ **CORRECTED by the owner the same day (`OWNER_RULINGS.md` §K, 2026-10-03) — the next bullet's
  "committed beside its source" is withdrawn.** *"We aren't going to put compiled stuff in the repo
  right? … the efi driver is compiled when building the repo."* build.rs compiles `firmware/kf-gop`
  during the normal cargo build (the nested-cargo pattern of `crates/kayfabe-isolate-host/build.rs`),
  `rust-toolchain.toml` lists `x86_64-unknown-uefi`, and no reproducibility compare is needed. The ROM
  stays arch-neutral for a later aarch64 (EFI machine type from the PE, x86 port I/O behind
  `cfg(target_arch)`, CI's aarch64 job builds `aarch64-unknown-uefi`). ⚠ **Not done on either branch
  yet**: `firmware/kf-gop/kf-gop.efi` is still committed and `kf_oprom::KF_GOP_EFI` still
  `include_bytes!` it. kf3 reads only `KF_GOP_EFI`, so the switch changes no kf3 code.
- ★★★ **OWNER DECISION, 2026-10-03 — it overrides steps 1 and 2 below and the *Install* bullet.**
  *"generate the uefi data in kayfabe and give it as blob in the rom. So there is no rom per gpu or
  similar, all is given as config data, just like cuda."*
  - The driver is **one constant `.efi` embedded in kayfabe**, the way the PTX kernels are
    (`crates/kf-cuda/src/display.rs:21`): the release build is committed beside its source as
    `firmware/kf-gop/kf-gop.efi` and `crates/kf-oprom` embeds it as `KF_GOP_EFI` (`include_bytes!`,
    feature `embedded-gop`, on by default and off for the firmware itself). No QEMU firmware file, no
    `romfile=`, no `qemu_find_file`, no `gop-image=` property, nothing per GPU to install.
  - CI's `firmware` job rebuilds it from source and fails if one byte differs. Nothing is normalised:
    `firmware/kf-gop/build.rs` links with `/Brepro` (the PE timestamp is a content hash) and
    `/DEBUG:NONE` (no debug directory, whose `.pdb` name carries cargo's per-path metadata hash). A
    fresh target directory and a second checkout path both reproduced it locally on 2026-10-03 at
    `3dd574e5` (sha256 `f11ab0b9…`, §4.11.8).
  - Everything per device is config data kf3 generates at realize: the ROM header and PCIR (the ids
    and class kf3 already presents) and the `KFGP` descriptor (BAR, offset, G, W/H/pitch, format, EDID
    from kf-disp). Rust calls `pack_kf_gop` and hands C the finished ROM — the new FFI becomes
    `kf3_option_rom(h, &rom, &rom_len)` (still **KF3_ABI 10 → 11**); C copies it in step 3's shape.
  - The PE is byte-identical on every host, so one future Secure Boot signature covers all of them.
1. ⊘ *superseded by the owner decision above:* C finds the PE with `qemu_find_file(QEMU_FILE_TYPE_BIOS, "kf3-gop.efi")`; `-L` or a `gop-image=`
   property overrides it.
2. ⊘ *superseded (C no longer handles the PE):* C passes the bytes to a new `kf3_option_rom()` (**KF3_ABI 10 → 11**, `qemu/hw/misc/kf3/kf3.h:9-14`,
   plus `crates/kf-qemu/tests/wire_mirror.rs`); Rust packs with the host identity, kf-disp's geometry and
   EDID; C copies the result.
3. C registers it in the shape of `pci_add_option_rom` (`hw/pci/pci.c:2626-2644`): `has_rom = true`,
   `memory_region_init_rom(pow2ceil(len))`, memcpy, `pci_register_bar(PCI_ROM_SLOT)`;
   `pci_del_option_rom` cleans up. vfio is **not** the model (`memory_region_init_io` with trapping reads,
   `hw/vfio/pci.c:1270-1274`).
4. Gates: `gop` defaults off; `gop=on` without `display=on` is refused by name; `rombar=0` means no ROM;
   a user `romfile=` wins (QEMU loads it after `pc->realize`). **Never** set a class-level `pc->romfile`
   for kf3: `pci_patch_ids` rewrites byte 6, which is inside `EfiSignature` (`pci.c:2484-2537`).
- **NV_PROM is unchanged**: kayfabe's VBIOS is one base image with `expansionRomOffset = 0`
  (`crates/kf-abi/src/vbios.rs:880-893`), a parse that is green today; nothing reads an EFI image out of
  PROM, and Hopper/Blackwell read no VBIOS image (`kf-chip/src/bar0.rs:332-338`).
- **Nothing in the guest reads the PCI ROM**: nvidia.ko only tests `IORESOURCE_ROM_SHADOW`
  (`nv.c:5105-5121`); RM disables the ROM through the BAR0 config mirror (`osinit.c:1271-1272`); the x86
  kernel replaces the ROM resource with the 0xC0000 shadow (`fixup.c:381-397`).
- **Install** (`V3_SWEEP_AND_INSTALL.md`): ⊘ *superseded by the owner decision above* — nothing is
  installed; the driver is inside the kf3 binary (`libkf_qemu.a` → QEMU).

#### 4.11.7 EDK2 behaviour: what the source says, and what the stand-in showed

Read in edk2-stable202408 (QEMU 10.2.4's `roms/edk2`); shown by the stand-in on 2026-10-03 at
`3dd574e5` on Ubuntu `ovmf 2025.11-3ubuntu7` and QEMU's `edk2-x86_64-code.fd`, and in CI runs
37127211692 (`2c6178fe`) and 37127710871 (`da5cc07f`) on Ubuntu `ovmf 2024.02-2ubuntu0.9`.

| behaviour | read in the source | stand-in, 2026-10-03 (`3dd574e5`; CI runs 37127211692, 37127710871) |
|---|---|---|
| The bus driver copies the ROM into `RomImage` before running its driver | `PciDeviceSupport.c:239-333` (`ProcessOpRomImage` inside `RegisterPciDevice`) | kf-gop decodes the descriptor from `RomImage` in `Supported`, all three builds |
| Only an EFI boot-service or runtime driver is loaded from a ROM | `PciOptionRomSupport.c:76-80`, `:701` | `build.rs`'s `/subsystem:efi_boot_service_driver` is honoured by lld-link: subsystem 11 (`kf-oprom pe`) |
| `LoadImage` gets `[EfiImageHeaderOffset, InitializationSize·512)` | `PciOptionRomSupport.c:85-90` | the payload is the PE byte for byte (unit test), and it loads |
| **ROM drivers are deferred until after End-of-DXE, and OVMF connects consoles twice** — before dispatching them and again after (*"GPU passthrough only allows Console enablement after ROM image load"*) | `OvmfPkg/Library/PlatformBootManagerLib/BdsPlatform.c:470-503` | QEMU's DEBUG edk2: *"3rd party image[0] is deferred to load before EndOfDxe"*. ⇒ A device OVMF has a built-in driver for is taken by it first: stdvga + a kf-gop ROM gets QemuVideoDxe (30 modes), kf-gop loads and finds the device owned (arm `stdvga_builtin_wins`). NVIDIA has no built-in OVMF driver, so kf3 is in `ati-vga`'s position, where kf-gop binds |
| A single-mode GOP drives the text console | — | *"GraphicsConsole video resolution 1152 x 648"*, *"Graphics Console Started"* (QEMU edk2 log) |
| **Option ROMs are not signature-checked, Secure Boot or not** | `PcdOptionRomImageVerificationPolicy\|0x00` (always trust) in `[PcdsDynamicDefault]`, `OvmfPkg/OvmfPkgX64.dsc:689`; set to 0x04 (deny) only under AMD SEV (`OvmfPkg/PlatformPei/AmdSev.c:468`) | with Secure Boot enforcing — the unsigned boot app is *"Access Denied -- rejected probably by Secure Boot"* in the same boot — the unsigned ROM runs (Microsoft-keyed and snakeoil VARS); the snakeoil-signed ROM and the tail-padded signed ROM run too (arms `sb_*`) |

#### 4.11.8 The local stand-in

★ **Run 2026-10-03 (late) at `adbe6fcd`, clean tree, with a ROM-verifying OVMF: 14 arms — 11 PASS,
3 OBSERVED, 0 SKIP, 0 FAIL** (`traces/v3_display/gop_standin_20261003_v3gop/`; F1 below). Same host as
the table below. CI run 37136448638 (`ac10c23f`, the same code before the history was squashed): 7 PASS,
3 OBSERVED, 4 SKIP (QEMU's edk2 and the deny-policy firmware are not on the runner), 0 FAIL.

⊘ **CORRECTED 2026-10-03 (late, `v3-gop`, the review of `v3-gop-kf3`) — the three Secure Boot arms
below that only observe are renamed, and none of them is a PASS any more.** `sb_ms_unsigned`,
`sb_snakeoil_unsigned` and `sb_snakeoil_tailpad` printed `verdict=PASS` whatever the ROM did, so no arm
could fail on the ROM outcome it was named for. They are now `sb_ms_unsigned_observe`,
`sb_snakeoil_unsigned_observe` and `sb_snakeoil_tailpad_observe`, with `verdict=OBSERVED` (counted
separately in `GOP_STANDIN_SUMMARY … passed= observed= skipped= failed=`); each still FAILs when it
cannot confirm Secure Boot is enforcing. Three new arms can fail on the ROM outcome — `sb_deny_signed`
(must start), `sb_deny_unsigned` and `sb_deny_tailpad` (must not) — on an OVMF built with
`PcdOptionRomImageVerificationPolicy=0x04`, named by `OVMF_DENY_CODE`/`OVMF_DENY_VARS`; without one they
SKIP and say why (test F1, §4.11.9). And the driver the stand-in packs is no longer "the committed blob":
it builds `firmware/kf-gop` itself, from the same source `kf-gop-image` embeds (CI's `firmware` job
checks the two builds are byte-identical). The UEFI target is listed in `rust-toolchain.toml`, so the
`rustup target add` line below is only needed for a toolchain installed before it.

```sh
rustup target add x86_64-unknown-uefi --toolchain 1.99.0
bash scripts/display/gop_standin.sh            # every arm; or name arms; --keep keeps the scratch dir
```

Host: QEMU 10.2.1, KVM, q35, 512 MiB, 1 vCPU; Ubuntu `ovmf 2025.11-3ubuntu7`; QEMU's
`edk2-x86_64-code.fd` from QEMU 10.2.4 (the same `.bz2` as QEMU 11.1.1's); the host kernel
`7.0.0-34-generic` with a busybox initramfs. The stand-in device is QEMU's `ati-vga` (1002:5046, class
0x0300, BAR0 = RAM the host reads back with QMP `pmemsave`). The ROM is the release driver packed by
`kf-oprom` for 1002:5046, BAR 0, an odd mode 1152x648 (pitch 4608, G 0x2E0000).
⊘ *Deviation from the reviewed design:* the design named QEMU's stdvga. On OVMF stdvga belongs to
QemuVideoDxe before any ROM driver runs (§4.11.7), so the stand-in uses a VGA-class device OVMF has no
driver for — which is also the position kf3 is in.

**Result, 2026-10-03, `3dd574e5`, clean tree: 11/11 PASS** (`traces/v3_display/gop_standin_20261003/`;
the release build it packed is the committed blob, sha256 `f11ab0b9…`):

| arm | what | result |
|---|---|---|
| `gop_ubuntu` | the test app's 21 checks (one GOP of ours, `MaxMode` 1, the descriptor's mode, `FrameBufferBase` = the BAR the bus driver assigned, both EDID protocols, `QueryMode`, `SetMode` 0 clears / 1 unsupported, `Blt` fill / buffer↔video with `Delta` and offsets / overlapping video→video, four refusals), then the framebuffer read back | 21/21; **648/648 lines byte-exact** at pitch 4608 |
| `gop_qemu_edk2` | the same on QEMU's edk2 | 21/21; 648/648 |
| `stdvga_builtin_wins` | stdvga + a kf-gop ROM | QemuVideoDxe binds (30 modes); kf-gop loaded, not started — recorded |
| `neg_wrong_id` | descriptor names device 0x5047 | refused: *"ids 1002:5046 but the descriptor names 1002:5047"*; no kf-gop GOP |
| `neg_no_descriptor` | descriptor magic broken | refused: *"KFGP: no KFGP magic"*; no kf-gop GOP |
| `linux` | OVMF → EFI stub → Linux 7.0 | `boot_vga` 1; BOOTFB `0x80000000-0x802d8fff` (BAR0 + 0, 2 985 984 bytes); simpledrm on it; sysfb's parent `0000:00:01.0`; fb0 1152x648 stride 4608; a pattern written through `/dev/fb0` is in the BAR **648/648 lines byte-exact** |
| `linux_two_vga` | a driverless VGA device (I/O+memory decode) at 02.0, kf-gop's at 03.0 | `boot_vga` moves to 03.0, the BOOTFB owner (*"overriding previous"*) |
| `sb_ms_unsigned` | Secure Boot, Microsoft keys, unsigned ROM | the ROM **loads and starts**; the unsigned app is *Access Denied* |
| `sb_snakeoil_signed` | Secure Boot, snakeoil keys, `sbsign`ed ROM and app | `secure_boot=1`; loads; 21/21 |
| `sb_snakeoil_unsigned` | the same keys, unsigned ROM | loads; 21/21 |
| `sb_snakeoil_tailpad` | the signed PE at 0x200, padding after | loads; 21/21 (no verification happens, §4.11.7) |

The same script is the CI job `firmware`'s last step. Run 37127211692 at `2c6178fe` (2026-10-03, GitHub
`ubuntu-latest`, KVM, QEMU 8.2.2, `ovmf 2024.02-2ubuntu0.9`, kernel `6.17.0-1022-azure`): 10 PASS,
`gop_qemu_edk2` SKIP (not installed there); that revision's Linux arm still had a weaker last check
(non-zero BAR bytes — which locally turned out to be OVMF's text, because fbcon had deferred its
takeover; replaced by the `/dev/fb0` pattern at `da5cc07f`). Run 37127710871 at `da5cc07f`, same
runner image: the same 10 PASS and 1 SKIP, the Linux arm's `/dev/fb0` pattern 648/648 lines
byte-exact on `6.17.0-1022-azure`.

⊘ *2026-10-03 (late, `v3-gop`): the next paragraph is history — there is no committed blob. Its
reproducibility result is what CI's `firmware` job now checks on every push in a new form: the driver
`kf-gop-image` embeds (nested build, scrubbed environment) equals a standalone build, byte for byte.*
**The committed blob is reproducible** (2026-10-03, `3dd574e5`): `firmware/kf-gop/kf-gop.efi`, 9 216
bytes, sha256 `f11ab0b9f61745a224388fdeda21d19c2c2d802d681a4f9d1c48ddcdd919ed9b`, rebuilt with the pinned
toolchain from a fresh target directory and from a second checkout at another path: identical both
times (`cmp`); the stand-in run above packed that same build. CI run 37128563465 (`65a22819`,
GitHub `ubuntu-latest`) rebuilt it from source on another machine to the same sha256, and its stand-in
ran 10 PASS and 1 SKIP (QEMU's edk2 not installed there).

**Not established locally** (box tests, §4.11.9): kf3's realize and ROM BAR; the seed through host BAR1
and the GPU zeroing; kf-disp's CUDA boot scanout; RM's console preservation with the new region table and
the walker's single run for it; seamless retirement; nvidia-drm eviction, NVKMS restore, Xorg and Windows;
memory types and bandwidth through host BAR1 (the design expects OVMF to map the GOP framebuffer UC); other families; a
firmware that does verify option ROMs (the padding layout, §4.11.6).

#### 4.11.9 Box tests, in order

Every result cites the kf3 binary's revision (`build_kf3.sh`'s `kf3-bins/<rev>/`). B0a needs no build.

- **B0a** (today's SeaBIOS bench, `display=on`, no ROM): while the lane holds the guest up, ask the kernel
  and Xorg without the pin.
  ```sh
  DISPLAY_HOLD_S=900 KF_DEVICE=kf3 bash scripts/bench/display/lane.sh b0a &
  scripts/bench/gssh_nv 'd=$(lspci -D -d 10de: | awk "NR==1{print \$1}"); cat /sys/bus/pci/devices/$d/boot_vga; sudo dmesg | grep vgaarb'
  scripts/bench/gssh_nv 'sudo systemctl stop lightdm; sudo rm -f /etc/X11/xorg.conf; sudo systemctl start lightdm; sleep 30; grep -E "PCI:\*|\(EE\)|NVIDIA\(0\)" /var/log/Xorg.0.log' > b0a_xorg.txt
  ```
  If Xorg starts on kf3, drop the BusID pin (`scripts/bench/display/desktop/xorg.conf.in`) independently
  of the ROM.
- ★ **2026-10-03 (`v3-gop-kf3`) — B0–B2 now have exact commands**, using what the integration built:
  `KF_FIRMWARE=ovmf` (`scripts/bench/boot_nvkvm.sh`: OVMF code read-only plus a per-run VARS copy beside
  the run's logs; unset = SeaBIOS, as before), `DISPLAY_KF3_EXTRA=gop=on` (appended to kf3's device
  line by `lane.sh`), and the device's own log lines in `/workspace/bench/run_<tag>_qemu.log`. The box
  needs Ubuntu's `ovmf` package (`sudo apt-get install -y ovmf`) and a kf3 binary of the revision under
  test (`bash scripts/bench/build_kf3.sh`). One boot serves B1–B3: the lane's hook loads nvidia.ko and
  then `nvidia-drm modeset=1 fbdev=1`.
- ⊘ *CORRECTED 2026-10-03 (late, `v3-gop`):* B0's last check below cannot fire any more — with
  `gop=off` kf3 keeps no fn 72 body and logs no C (§4.11.4's first ⊘ note), so the grep prints nothing
  whatever the guest sends. Drop it; C is read at B2 (`gop=on`). The rest of B0 stands.
- **B0** (OVMF, `gop=off`): boot, nvidia.ko, the M1/M2 probe pixel-exact; where OVMF places kf3's
  64-bit BARs; `boot_vga` without a GOP; whether the noble image has an ESP; and that the guest sends
  C = 0 without a ROM (§4.11.4's ⊘ note: no *"GET_GSP_STATIC_INFO (gop=off)"* line). In the local
  `linux_two_vga` arm Linux read `command=0x0007` (I/O + memory + bus master) on a driverless VGA device
  at init; whether OVMF or Linux set it is not established, so B0 checks kf3's decode without a GOP.
  ```sh
  KF_FIRMWARE=ovmf DISPLAY_HOLD_S=600 bash scripts/bench/display/lane.sh b0 &
  scripts/bench/gssh_nv 'ls /sys/firmware/efi >/dev/null && echo UEFI; lsblk -f | grep -i vfat; d=$(lspci -D -d 10de: | awk "NR==1{print \$1}"); cat /sys/bus/pci/devices/$d/boot_vga; sudo setpci -s $d COMMAND; sudo lspci -vv -s $d | grep -E "Region|Expansion ROM"'
  grep -a 'GET_GSP_STATIC_INFO (gop=off)' /workspace/bench/run_b0_qemu.log   # expected: nothing
  ```
- **B1** (`gop=on`): the console shows OVMF, GRUB and kernel text from the first seconds; simpledrm or
  efifb on BAR1 with BOOTFB at BAR1 + 0; the boot counters.
  ```sh
  KF_FIRMWARE=ovmf DISPLAY_KF3_EXTRA=gop=on DISPLAY_HOLD_S=900 bash scripts/bench/display/lane.sh b1 &
  for t in 3 8 15 30; do sleep 5; echo "screendump /workspace/bench/display/b1_shot$t.ppm" | socat - UNIX-CONNECT:/workspace/bench/run_b1.mon; done
  grep -a 'boot display ON\|option ROM registered\|display: boot layer\|boot\[frames' /workspace/bench/run_b1_qemu.log
  scripts/bench/gssh_nv 'grep -i -B1 bootfb /proc/iomem; sudo dmesg | grep -iE "efifb|simpledrm|vgaarb|BOOTFB"; d=$(lspci -D -d 10de: | awk "NR==1{print \$1}"); cat /sys/bus/pci/devices/$d/boot_vga'
  ```
  Pass: the shots show firmware and kernel text (not black); *"option ROM registered"* and *"boot display
  ON … G = 0x7f0000"* in the QEMU log; BOOTFB at the BAR1 base; `boot_vga` = 1; `boot[frames=N …]`
  growing in the status line.
- **B2** (RM adoption, the same boot after the hook loads nvidia.ko): fn 72 C == `align64K(H·pitch)` ≤ G;
  fn 65 has three regions; the first K_BAR1 change is **one run** VA 0 → store 0, length C; the seed
  retired in that batch; no LEVEL_ERROR *"cannot preserve console mapping"*; nvidia-smi works; no black
  frames across the module load.
  ```sh
  grep -a 'GET_GSP_STATIC_INFO\|boot display seed\|armed its first head' /workspace/bench/run_b1_qemu.log
  grep -ac 'cannot preserve console' /workspace/bench/run_b1_dmesg.log        # expected: 0
  ```
  Pass: *"the guest preserves a firmware console of 0x7f0000 bytes … 3 regions"*; *"boot display seed
  [0x0, +0x7f0000) retired at the first change: the console is ONE run, VA 0 -> store 0, 0x7f0000
  bytes"*. ⚠ *"⚠ N placements inside it — the console is NOT one view"* means the walker split the
  console's 4 KiB PTEs: N host BAR1 views (§4.11.10).
- **B3** (`modeset=1 fbdev=1`): eviction, `NOTIFY_CONSOLE_DISABLED`, `boot_done`, no black gap.
- **B4**: Xorg with no `xorg.conf`, plus Wayland; the M3 probes pass.
- ⊘ **CORRECTED 2026-10-03 — B5 RAN** (`traces/v3_display/gop_unload_20261003/`, §4.11.13; kf3
  `4a4b95f7`): (c) the console shows new text with no RM client, with one, and after a second teardown;
  (a) as first run was a Wayland compositor on simpledrm, not X (the image's lightdm autologin still named
  B3's `cinnamon-wayland`): it is now that arm, by name, and the X arm is (a2) — the NVIDIA X driver,
  `modeset=0`, X11 Cinnamon; both bring the text console back and keep it updating; (b) is black. The hook
  (`scripts/bench/display/unload_hook.sh`) prints each arm's expectation. (d) was not run.
- ⊘ **2026-10-03 (late) — B5 RE-RUN with a verdict** (`traces/v3_display/gop_final_20261003/`, kf3
  `06b307c4`; §4.11.13): every arm is judged against its expectation (`DISPLAY_B5_JUDGE`), and
  `DISPLAY_B5_VERDICT PASS|FAIL` decides the lane's exit (3 on a failing or missing verdict); the
  bullet above *"prints each arm's expectation"* graded nothing. Result: 14/14 PASS. `lane.sh` also grades
  every `gop=on` boot's handoff (`DISPLAY_BOOT_HANDOFF black_frames=0`, from kf3's *"console shows"* lines).
- **B5** (unload): (a) `modeset=0` + X restore, on the 6.8 guest; (b) `fbdev=1` rmmod gives black; (c) a
  CUDA-only guest keeps its console for the VM's life; (d) a 7.x guest arm records which
  `nv_get_screen_info` path ran.
- **B6** (performance): boot time; WC/UC bandwidth through host BAR1; scroll timings; the boot-copy cost.
- **B7** (budget): seed + transient in cardbudget; two kf3 devices; G > bar1-size refused.
- **B8** (families): TU116, AD104: B1–B3. GB205: B1, B2 and the boot layer; B3 only after M5. GA100 and
  GH100 refuse `display=on` and `gop`. A large-bar1 arm for static-BAR1 with a console
  (`kern_bus_tu102.c:444-500`).
- **B9** (hostile): ROM writes; remapping BAR1 VA 0; a forged fn 72 C.
- **B10** (Windows): install under OVMF with **`-no-reboot` and a QEMU restart per phase**; setup, boot,
  safe mode visible; nvlddmkm install and the Basic Display handover; a GOP framebuffer above 4 GiB;
  whether Windows sends `consoleMemSize`.
- ★ **Added 2026-10-03 (late, `v3-gop`) — no GPU needed:**
  - ★ *F1 RAN locally 2026-10-03 (late) at `adbe6fcd` (clean tree; `traces/v3_display/gop_standin_20261003_v3gop/`):
    `sb_deny_signed` PASS (verified, started, 21/21), `sb_deny_unsigned` PASS (did not run),
    `sb_deny_tailpad` PASS (did not run). Snakeoil keys; CI SKIPs these arms (no such firmware there).*
  - **F1** (a firmware that verifies option ROMs — the only test of kf-oprom's end-aligned layout):
    build OVMF from QEMU 10.2.4's `roms/edk2` with Secure Boot, SMM and
    `PcdOptionRomImageVerificationPolicy` 0x04 (`scripts/display/build_ovmf_deny.sh <edk2> <out>`, ~3 min;
    it names its two toolchain workarounds), then
    `OVMF_DENY_CODE=<out>/OVMF_CODE_4M.deny.fd OVMF_DENY_VARS=/usr/share/OVMF/OVMF_VARS_4M.snakeoil.fd bash scripts/display/gop_standin.sh sb_deny_signed sb_deny_unsigned sb_deny_tailpad`.
    Pass: the signed end-aligned ROM starts and passes every check; the unsigned and the tail-padded
    ROM do not start. Owner question 1 (Secure Boot) and the layout deviation of §4.11.6 rest on it.
  - **A1** (arm64 guest firmware): whether AAVMF runs PCI option ROMs at all (`OWNER_RULINGS.md` §K,
    "Unchecked") — with the `aarch64-unknown-uefi` driver CI already builds, on an AArch64 QEMU.

#### 4.11.10 Risks

- **HIGH, from source:** the ROM without the region-0 change breaks the guest driver's load (§4.11.4).
- Parsing fn 72 with the wrong table if decoded before fn 1 — closed by the lazy decode.
- Real GSP-RM's region-0 attributes are not knowable (closed firmware); verify the heap offset on the box.
- The walker must coalesce the console's 4 KiB PTEs into one view (B2).
- Stale shadow pixels for loaders that write the framebuffer directly (cosmetic).
- The seed costs G of host BAR1, briefly 2G. A host class of 0x0302 gets no `boot_vga` (`pci-sysfs.c:1720`).
- Linux 7.x guests: no NVKMS console import (bare-metal parity).
- A device OVMF has a built-in driver for would lose to it (§4.11.7) — none exists for NVIDIA ids.
- If RM does not adopt the console (C = 0), kf-disp scans store `[0,G)` until the first modeset, which can
  show the guest's own heap — as on hardware; the realize-time zeroing removes any pre-boot exposure.
- Windows behaviour is not established.

#### 4.11.11 Owner questions, and the defaults in force (2026-10-03)

The owner will confirm; until then the build follows these defaults.
1. ★ *2026-10-03 (late): F1 (§4.11.9) shows the ROM layout signs correctly — on a ROM-verifying OVMF a
   snakeoil-signed kf-gop ROM runs and an unsigned one does not — so option (b) below needs only the key
   and the signing step, not a layout change.*
   **Secure Boot:** documented **off** for the boot display; no signing infrastructure yet. ⊘ The
   question's premise ("an unsigned ROM will not run under Secure Boot-enforcing OVMF") does not hold
   for OVMF: it trusts option ROMs unconditionally except under AMD SEV (§4.11.7). Physical-machine-style
   firmware with a deny policy would need option (b) (a kayfabe key in a VARS template; the PE is
   constant per release, so one signature) or (c) (Microsoft third-party CA).
2. **§13:** `firmware/` is a named unsafe exception, outside the cargo workspace, never linked into the
   VMM (`firmware/README.md`; CI gate B, the ratchet and the ABI-quarantine firmware arm).
3. **Firmware for bench lanes:** OVMF only for the display lane; SeaBIOS lanes stay the measured
   baseline. No legacy VGA BIOS for the release (a code-type-0 VBE image is 1–2+ weeks).
4. **Warm reboot:** kf3 has no reset path (`kf3.c:965-977` at `v3-gop-kf3`): a guest reboot needs a QEMU restart
   (`-no-reboot`). Windows Setup's reboots go through restarts (B10).
5. **No host-RAM backing** for the boot framebuffer (§4.11.2).
6. ⊘ *2026-10-03 (late, `v3-gop`): with `gop=off` C is no longer even read (§4.11.4's first ⊘ note);
   the question below stands as asked — whether to apply the region-0 rule without kf3's ROM.*
   **`gop=off` and the console region** (added 2026-10-03, `v3-gop-kf3`): the default in force serves
   today's fn 65 table with `gop=off` whatever the guest's `consoleMemSize` (§4.11.4's ⊘ note) — the
   coordinator's "byte-identical with the property off" — where the reviewed design applied the rule
   regardless of the ROM. Apply it with `gop=off` too? Only a guest whose console sits at kf3's BAR1
   base sends C > 0, and without kf3's ROM none is expected to (B0 checks).

#### 4.11.12 The kf3 integration, as built (`v3-gop-kf3`)

⊘ **CORRECTED 2026-10-03 (late) — branch `v3-gop`: the review of `v3-gop-kf3` (one blocker, five minors)
is fixed, and this section's text is superseded where it says otherwise.** Each fix has a test that fails
without it (named in brackets; `kf3.c` has none of its own because CI does not compile it — its rule moved
into a header CI does compile).
1. **Blocker, `OWNER_RULINGS.md` §K** — no compiled binary; built from source by
   `crates/kf-gop-image/build.rs`; arch-neutral ROM; aligned volatile framebuffer stores; CI's `aarch64`
   job builds the driver (§4.11.6's ⊘ block). [`kf-oprom` `pe::tests::an_aarch64_driver_passes_with_its_own_machine`,
   `pack::tests::the_machine_type_is_the_pes_own`; `kf-gop` `blt::tests::the_framebuffer_sees_one_word_store_per_pixel_and_no_more`,
   `probe::tests::an_unaligned_framebuffer_is_refused`; `kf-gop-image` `tests/embedded.rs` and its three
   build-script tests; CI's "No compiled binary is committed" and "The embedded driver is the standalone
   build" steps.]
2. **The ROM BAR is registered after every fallible step** — `kf3_option_rom_build` moved from before
   the doorbell-ioeventfd block to just before `memory_listener_register`; nothing after it can fail.
   Deviation 3 below was false before the move and holds now.
3. **`gop=on` with `romfile=` (including `romfile=""`) or `rombar=0` is refused by name, before Rust
   realizes** (`qemu/hw/misc/kf3/kf3_gop.h`, `kf3_dev_realize`). It used to warn and register no ROM
   while the Rust half kept the whole boot-display posture and logged *"boot display ON — option ROM N
   bytes"* for a ROM nobody served. Step 8's *"a user `romfile=` or `rombar=0` wins, with a warning"* is
   withdrawn. [`crates/kf-qemu/tests/gop_rom_knobs.rs` compiles and runs the header's rule.]
4. **`gop=off` attaches no `SystemInfoCell` and no `ConsoleSeat`** (`crate::gop::ConsoleWiring`, one
   decision for the GSP state machine and fn 65). Step 5's *"one cell … given to the GSP state
   machine"* and *"What `gop=off` does add"* below are withdrawn for `gop=off`: it adds nothing.
   [`gop::tests::gop_off_wires_no_cell_and_no_seat`, built through the wiring `Device::realize` uses.]
5. **Supersession notes** for the committed-blob text: `V3_SWEEP_AND_INSTALL.md`, `kf-oprom`'s crate
   docs, `CI_V3.md`, `firmware/README.md`, and this document (§4.11.6, §4.11.8).
6. **The stand-in's observe arms** are named and judged as observations (`verdict=OBSERVED`), and the
   arms that can fail on the ROM outcome exist (`sb_deny_*`, test F1) — §4.11.8's ⊘ note. F1 ran locally
   at `adbe6fcd` and passes (§4.11.9).

**STATUS: BUILT 2026-10-03, CI green at `37740a4b` (run 37132057723: stable, aarch64 and firmware
jobs); not run on a GPU box.** Everything is behind the kf3 property `gop` (default off). kf3.c is not
compiled by CI; it was syntax-checked with `-Werror` against QEMU 10.2.4's configured headers (the
flags of `hw/display/ati.c`, pixman on), and the C seam by `tests/wire_mirror.rs` in CI.

**At realize, in order** (`crates/kf-qemu/src/device.rs`, `Device::realize`):

| step | what | where |
|---|---|---|
| 1 | the plan: the virtual monitor's preferred mode (`kf_rm::display::monitors()[0]`, 1920x1080) → `Geometry::for_mode` → BAR1 + 0, G = 0x7F0000, and its EDID. Refused by name: `gop=on` without `display=on`; G > `bar1-size` | `crates/kf-qemu/src/gop.rs` (`BootPlan::for_config`) |
| 2 | the host BAR1 budget + G | `crates/kf-qemu/src/cardbudget.rs` (`Demand::with_boot_fb`) |
| 3 | the option ROM: `kf_oprom::pack_kf_gop` (⊘ `v3-gop`: `kf_gop_image::pack_kf_gop`, the driver built from source) with the vendor, device and class kf3 presents | `BootPlan::rom` |
| 4 | store `[0, G)` zeroed by the walker's GPU write, 1 MiB per call, before any reader exists | `WalkKernel::write_store` |
| 5 | (⊘ `v3-gop`: with `gop=on` only — `ConsoleWiring`, item 4 of the ⊘ block) one `kf_gsp::SystemInfoCell`, given to the GSP state machine (`GspFsm::with_system_info_cell`) and, through the chain recipe, to every rebuilt `StaticInfoPolicy` (`ConsoleSeat`) — so it survives `ReselectAtFn1` | `crates/kf-gsp/src/sysinfo.rs`; `crates/kf-rm/src/staticinfo.rs` |
| 6 | the display plane's boot layer (`DisplayPlane::with_boot`, `kf_disp::scanout::boot_layer`) | `crates/kf-qemu/src/display.rs`; `crates/kf-disp/src/scanout.rs` |
| 7 | the BAR1 seed: one view of store `[0, G)` at BAR1 offset 0, on the realize thread | `crates/kf-mem/src/cpuwin.rs` (`CpuWindow::seed`) |
| 8 | the C device registers the ROM BAR in `pci_add_option_rom`'s shape: `has_rom`, `memory_region_init_rom(pow2ceil(len))` (or the user's `romsize`), the bytes copied, `pci_register_bar(PCI_ROM_SLOT)`; a user `romfile=` or `rombar=0` wins, with a warning naming it (⊘ `v3-gop`: refused by name before step 1 instead, item 3 of the ⊘ block) | `qemu/hw/misc/kf3/kf3.c` (`kf3_option_rom_build`), `kf3_option_rom` |

**While the guest runs:**
- **fn 72** is still never answered; its body is kept (bounded by the largest `GspSystemInfo` in the
  driver matrix, 952 bytes; a later fn 72 replaces it; `GspFsm::device_reset` keeps the cell).
- **fn 65** decodes `consoleMemSize` with the table that serves fn 65, after any fn-1 re-select, and with
  `gop=on` and C > 0 serves `kf_chip::bar0::fb_layout_with_console(fb_length, C)`: region 0 = `[0, C)`
  reserved, ISO, uncompressed, performance 0; region 1 the heap from C; region 2 the carve-out; both
  roots unchanged. Refused by name in the envelope (`NV_ERR_NOT_SUPPORTED`, the reason in the QEMU log):
  a fn 72 struct whose size is not the serving version's, a version or field not in the driver matrix, C
  not a multiple of 64 KiB, C at or past the carve-out, C larger than BAR1, a board table that is not
  kf-chip's layout. Logged on every fn 65 with a console: C against G and the region count.
- ⊘ *2026-10-03 (late, the review of `v3-gop-unload`; §4.11.13): the note below this one is CORRECTED —
  its black was chosen whenever a frame had been shown and nothing was scanned, so the boot layer →
  first-head handoff showed one black frame. `Shown::Blank` is now chosen only when a head's scanout
  that WAS shown is lost (no head lit; or a lit head with no window for 250 ms); until a head first
  scans a window — the handoff, GB20x, `gop=off` before its first window — there is no new frame and the
  last one stays.*
- ⊘ *2026-10-03 (B5, §4.11.13): "then never again" stands, but what follows is no longer "nothing": once a
  frame was shown, nothing to scan is `Shown::Blank` (black), and a scanout freed with `PRESERVE_HW` is
  `Shown::Preserved` until a head is armed again.*
- **The display worker** shows `Shown::Boot` — chosen before the window-vocabulary gate, so GB20x shows
  it too — until the first armed head, then never again (`choose_shown`); the Boot arm composes one
  authored layer with no context DMA. Counters in the status line: `boot[frames=N retired=+Tms]`.
- **The first BAR1 batch that changes anything** retires the seed inside `MapTarget::invalidate`: the
  batch's views are already placed, the uncovered rest of `[0, G)` sinks to scratch, then the seed's
  view is released. The log line says whether the console arrived as ONE run VA 0 → store 0 (box test
  B2) or as several views.
- **At stop**, a seed the guest never retired is released by the VA thread as it exits (scratch first).

**FFI:** `KF3_ABI` 10 → 11 — `kf3_realize` takes `gop`, and `kf3_option_rom(h, &rom, &rom_len)` returns
the ROM (`crates/kf-qemu/src/ffi_unsafe.rs`, `qemu/hw/misc/kf3/kf3.h`); the unsafe ratchet moved 44 → 46
for it, itemised in CI.

**Unit tests (CI runs 37131490782 and 37132057723):** `crates/kf-chip/tests/console_region.rs` (C = 0 is
`fb_layout` byte for byte; three regions; refusals); `kf-gsp` `boot::fn72_is_kept_for_fn65` and
`sysinfo::tests`; `crates/kf-rm/tests/console_region.rs` (byte-identical replies with no seat, no fn 72,
C = 0 or `gop=off`; three regions with `gop=on`; each refusal; the field's offset per version; ★ fn 72
before a fn-1 re-select decoded with the re-selected table — the provisional one refuses the guest's
struct); `kf-mem` `cpuwin::tests` (place → sink → release; a first change outside `[0, G)` retires it
too; a two-run console leaves no gap and is named; no change, no retirement; a refused sink keeps the
seed; no seed, no host verb); `kf-disp` `scanout::tests::*boot*`; `kf-qemu` `gop::tests`,
`cardbudget::tests::the_boot_framebuffer_is_one_more_g_of_host_bar1`,
`display::tests::the_boot_layer_shows_until_the_first_armed_head_and_never_again`, `tests/wire_mirror.rs`.

**`gop=off` is today's device:** no plan, so no ROM (`kf3_option_rom` answers −1 and kf3.c registers
nothing), no zeroing, no seed, no boot layer, no budget term (`gop::tests::gop_off_plans_nothing`,
`with_boot_fb(0)`); fn 65's reply is byte-identical whatever fn 72 says (kf-rm test above); the state
machine without a cell, and a window without a seed, behave as before (kf-gsp and kf-mem tests above).
What `gop=off` does add: the cell keeps fn 72's bytes, and fn 65 logs a non-zero C. ⊘ *`v3-gop`: no
longer — `gop=off` attaches no cell and no seat, so it adds nothing (item 4 of the ⊘ block).*

**Changed against the design above:**
1. `gop=off` reads C and does not act on it (§4.11.4's ⊘ note; owner question 6, §4.11.11). ⊘ *`v3-gop`:
   `gop=off` no longer reads C at all.*
2. The never-retired seed is released by the VA thread when the device stops (`kf3_unrealize` → stop),
   not inside `kf3_unrealize` itself: the window belongs to that thread. Scratch first, so the order
   against QEMU's BAR unmapping does not matter.
3. The ROM BAR is registered last in `kf3_dev_realize`, after every fallible kf3 step, because QEMU's
   realize-failure path does not unregister a ROM. ⊘ *Untrue at `v3-gop-kf3` (the doorbell-ioeventfd
   block could still fail after it); true since `v3-gop` (item 2 of the ⊘ block).*
4. The host BAR1 budget charges G for as long as `gop=on`, not only for the retirement moment.
5. `kf_disp::scanout::boot_layer` takes a `BootSurface` (kf-disp does not depend on kf-oprom); kf-qemu
   converts.
6. Region 0's attributes are ours: performance 0 like the other reserved region (GSP-RM's are closed).

**Not built:** §4.11.4 rows 3–5 (`bIsGpuUefi`/`bIsEfiInit`, `BIOS_GET_UEFI_SUPPORT`,
`SYSTEM_GET_BOOT_DISPLAYS`) — nice-to-haves; ⊘ *(built on `v3-gop`, item 1 of the ⊘ block)* the owner's §K follow-ups (build.rs instead of the
committed `.efi`; the arch-neutral ROM: EFI machine type from the PE, x86 port I/O behind
`cfg(target_arch)`, an `aarch64-unknown-uefi` CI build); a reset path (owner question 4); `-no-reboot`
and the Windows arm in the bench scripts (B10); SPDX headers on the existing kf3 overlay files
(`OWNER_RULINGS.md` §G, task I7).


#### 4.11.13 B5 — when RM, NVKMS or nvidia-drm lets go (measured 2026-10-03, branch `v3-gop-unload`)

⊘⊘ **CORRECTED 2026-10-04 (`v3-cand-1`) — the (a2) PASS below was TIMING, not design.** On box vmb (vast
54049598) arm (a2) failed 5 of 5 runs, with the candidate's kf3 (`8a682f1b`) and with master's
(`d4c3767b` = `789dee9f`'s code) alike (`traces/v3_candidates/cand1_20261003/display/`, `b5c1/` and
`b5_repeats/`): kf3 logged *"scanout REFUSED context DMA 0x10088 on channel 7: NotBound"*, then at the
`PRESERVE_HW` frees *"BLACK … (the scanout shown is lost: no head is lit)"* where 54032077 logged *"the
PRESERVED scanout"*, and the text console stayed black.
- **The guest's order — the same in the passing and the failing runs** (the run_h/b5h and b5c1 QEMU
  logs from the restore to the core free: the same display events in the same order; b5c1 adds five GSP
  `Free` RPCs before the refusal, and the refusal): NVKMS restores the console (window 6
  latches context DMA `0x10088`), sends window 6 a flip to NULL with **no UPDATE** (*"chn 7 PUT 0xfa4: 0
  effects"*), frees the console surface — RM clears `0x10088`'s hash entry and object from display
  instance memory while window 6 stays armed on it — and only then frees the window and core channels
  with `PRESERVE_HW`. That is `nvFreeDevEvo`'s order (`ogkm-580: src/nvidia-modeset/src/nvkms-evo.c:9101-9112`:
  `nvEvoRestoreConsole`, `nvEvoUnregisterSurface(…, skipUpdate = TRUE, …)`, `nvFreeLutSurfacesEvo`,
  `nvFreeCoreChannelEvo`; the surface's context DMA goes through `FreeSurfaceEvoRm` → `nvCtxDmaFree`,
  `nvkms-surface.c:115-154`, `nvkms-evo3.c:8042-8048`, and RM's `_ctxdmaDestruct` →
  `dispchnUnbindCtxFromAllChannels`, `src/nvidia/src/kernel/gpu/mem_mgr/context_dma.c:356-357`,
  `src/nvidia/src/kernel/gpu/disp/inst_mem/disp_inst_mem.c:861-930`; the channels keep their heads,
  *"to avoid shutting down the heads we just enabled"*, `nvkms-rm.c:2990-3017`).
- **What a real display engine shows in that gap: the console.** NVKMS frees the surface's context DMA
  first and preserves the heads after, and the restored console stays on bare metal (the table below,
  arm (a2)) — the engine scans the surface its ARMED state latched, not a fresh hash lookup per frame.
- **What kf3 did:** every scanout copy re-resolved each window's context DMA, and with nobody watching
  the console a refresh copy runs every 250 ms. On 54032077 the restore-to-free gap was 153 ms (+149459 →
  +149612 ms), no copy fell inside it; on vmb it was 346–383 ms (four repeats) and 2.2 s (b5c1), so one
  did, refused window 6, and — no longer whole — cleared `ScanState::last_plan`, which is what item 3's
  preserving free keeps. With nothing to keep, the core free went black.
- **The fix** (`crates/kf-qemu/src/display.rs`, `LatchedDmas`): a window's context DMA is resolved for its
  ARMED state and kept until that window latches again (any UPDATE, its channel's alloc or free, or new
  instance memory). The first copy after a latch resolves afresh, and the flip's completions and GET wait
  for that copy (§4.5's barrier), so a guest that idles its channel before freeing the surface — NVKMS
  does (`nvEvoClearSurfaceUsage` → `nvRMSyncEvoChannel`, `nvkms-flip.c:1229-1263`) — is always resolved
  before it unbinds. Refusals that stand: a window that LATCHES on a context DMA that does not resolve
  (refused at every copy, never kept), and a handle its armed state never resolved. Test (GPU-free,
  replays latch → copy → unbind → copy → frees):
  `display::tests::a_context_dma_unbound_before_the_preserving_free_keeps_the_console`; with per-copy
  resolution restored it fails, the free showing `Blank` instead of `Preserved`.
- **Re-run on hardware at `0ac157b2`, 2026-10-04:** four B5 runs, each 14/14,
  with no context-DMA refusal and the console preserved and updating. The full merge bar,
  app baseline, B0/B1 and X11 A/B also pass (`traces/v3_candidates/cand1_20261004/`).
  The earlier pending-run statement is superseded; see that record for the unrelated
  first-attempt churn-test failure and unchanged passing retry.

**STATUS: REVIEWED AND RE-RUN, 2026-10-03 (late) — kf3 `06b307c4`, `traces/v3_display/gop_final_20261003/`.**
The review's findings are folded below as ⊘ notes above what they correct. At `06b307c4` (box 54032077):
B5 `DISPLAY_B5_VERDICT PASS arms=14 failed=0` — (c), (c2), (c3) new text shown; (a) Cinnamon Wayland with no
Xorg and no fresh Xorg log, text console back and updating; (a2) X11 on the NVIDIA X driver with a fresh
log, NVKMS's restored console kept (*"+149612 ms the console shows the PRESERVED scanout … [window 6 store
0x0]"*) and updating; (b) fbcon shown, then black after `rmmod` (nonblack 0/1000; kf3: *"BLACK"* 250 ms after
the window left); five teardowns, five re-seeds, none refused. X's modeset left head 3 windowless for
20 ms: *"no new frame … held"*, no black (before: a BLACK frame). The handoff (X's first modeset in (a2)):
no black. B1 and B0 as in §4.11's STATUS. CI green at `06b307c4` (run 37148069930).
⚠ What none of these runs can show: GB20x keeping its boot layer (no window vocabulary) is argued from
`choose_shown` and unit-tested, not run.

(superseded the same day, kept as written) **STATUS: BUILT and MEASURED, 2026-10-03.** Box 54032077 (RTX 3060, host 580.159.04, guest noble 6.8 with
580.159.04, OVMF), evidence `traces/v3_display/gop_unload_20261003/`: the first run at `f20ab853`, the
diagnosis at `e2c6e1d5` (instruments only), B5, B1 and B0 at `4a4b95f7`. CI green at `4a4b95f7` (run
37142859887).

**The first run, and each arm's cause** (diagnosis run d1):

| arm | first run | cause, measured at `e2c6e1d5` (run d1, 2026-10-03) | bare metal |
|---|---|---|---|
| (c) nvidia.ko, no RM client | 40 lines on tty1 never shown | The guest RM has no persistence: it initialises the adapter at the first open and tears it down at the last close (five RM lives in d1's one boot). At each teardown it unmaps the console at BAR1 VA 0 (kf3: BAR1 `[0, G)` holds no guest view → SCRATCH), ≤ 1 ms later writes `NV_PBUS_BAR1_BLOCK = 0` (MODE PHYSICAL, target VID_MEM: `kbusStatePreUnload_GM107` → `kbusTeardownMailbox_GM107`, `ogkm-580: src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:746-787`), then sends fn 47 (`bInPMTransition = 0`). simpledrm's writes landed in kf3's scratch. Positive control: with a `/dev/nvidia0` holder keeping RM up, the same writes showed. | BAR1 physical: `[0, G)` is FB `[0, G)`, the console keeps drawing |
| (a) as first run | the session's last frame stays | Not X, not NVKMS: the image's lightdm autologin still named B3's `cinnamon-wayland` (muffin on simpledrm; `Xorg.0.log` was B3's; nvidia-modeset first loaded at (b); no display channel during the session). NVIDIA's EGL held RM up; at the session's end RM was torn down and the console's redraw went to scratch — (c)'s cause. | the text console comes back |
| (a2) (new) the NVIDIA X driver, `modeset=0` | — | ⊘ *NOT measured at `e2c6e1d5` — inferred* (corrected 2026-10-03, late, the review of `v3-gop-unload`: run d1's two a-arms ran neither X nor NVKMS — `modules=[nvidia_uvm nvidia]`, nvidia-modeset first loaded at 355.4 s in (b) — and d2, the only other run before b5f, already carried the fix and is not kept; by the code before the fix an unclaimed free left the last frame frozen, not dropped, so *"dropped the scanout"* below is also wrong). The mechanism, from source and from b5f (which ran it with the fix): NVKMS restores the console when X closes (`ReleaseModesetOwnership` → `RestoreConsole`, `ogkm-580: src/nvidia-modeset/src/nvkms.c:1107-1161`) and frees each channel after `NV5070_CTRL_CMD_SET_RMFREE_FLAGS(PRESERVE_HW)` (`nvkms-rm.c:2990-3017`; ROUTE_TO_PHYSICAL, so GSP-bound — it reaches kf3). kf-disp did not claim it, and dropped the scanout at the free. | the restored console stays |
| (b) `fbdev=1`, fbcon unbound, `rmmod nvidia_drm` | the last fbcon frame stays | `fbdev=1` makes nvidia-drm evict the firmware framebuffer and call `framebufferConsoleDisabled` (`kernel-open/nvidia-drm/nvidia-drm-drv.c:2031-2049`), and NVKMS drops its console surface (`nvRmUnmapFbConsoleMemory`, `nvkms-rm.c:4967-4998`). At `rmmod`, `nvEvoRestoreConsole` has no surface (`nvkms-console-restore.c:796-799`), fails, and NVKMS shuts the heads down (`:971-977`); it never sends `PRESERVE_HW` (`0x50700117` appears nowhere in d1's log). kf-disp measured the head left with no window — and then produced no further frame, so QEMU kept the last one. | black (no signal) |

**The fixes** (kf3 `4a4b95f7`):
1. **BAR1 back to physical at teardown.** The register drainer — never a vCPU — recognises the guest's
   write of its BAR1-mode register with MODE PHYSICAL (per die group from hwref: `NV_PBUS_BAR1_BLOCK`
   31:31 through Ada, `NV_VIRTUAL_FUNCTION_PRIV_FUNC_BAR1_BLOCK_LOW_ADDR` 9:9 behind the VF window from
   Hopper, the `kbusTeardownMailbox` HAL split of `g_kern_bus_nvoc.c:1825-1834`; `kf_chip::bar1mode`) and
   a fn 47 without `bInPMTransition` (decoded with the generated layout; `kf_qemu::bar1phys`), and only
   bumps a counter and wakes the VA thread (`Inbox::request_bar1_physical`). The VA thread, once no walk
   is in flight or pending, places the seed again (`CpuWindow::reseed`: one RM map + one `mmap` over
   scratch, no view released) — unless BAR1 changed since it noticed the request (an RM took BAR1 back
   first: its placements win, logged). The re-seed retires exactly as the first seed did at the next
   RM's first BAR1 change (place, sink the rest, release). A guest placement still inside `[0, G)` refuses
   the re-seed by name.
2. **A lost scanout is black** (`Shown::Blank`). ⊘ *CORRECTED 2026-10-03 (late, the review of
   `v3-gop-unload`, MEDIUM):* as first built — *"once a frame was shown, a moment with no armed head
   scanning a window presents one black frame of the last size"* — it flashed black at the boot layer →
   first-head handoff (`[measured b1f, b5f at 4a4b95f7]` *"+52936 ms the console shows BLACK"* 4 ms before
   head 3's window), followed GB20x's boot layer (no window vocabulary until M5) with black, and changed
   `gop=off`. Now (`crates/kf-qemu/src/display.rs`, `choose_shown`): black only when a head's scanout
   that WAS shown (an armed composition was chosen) is lost — at once when no head is lit (every head
   disarmed, or the core channel freed), after a 250 ms hold when a lit head scans no window (b5f's X
   modeset left one for 22 ms). Anything else with nothing to show is no new frame: the boot layer's
   last frame stays through the handoff, and on GB20x for good. **`gop=off` now:** QEMU's placeholder
   until a head first scans a window, as before; after that a lost scanout is black where it used to
   freeze the last frame (the bare-metal answer: a monitor with no scanout shows black).
3. **`PRESERVE_HW`** (`Shown::Preserved`): kf-disp claims `NV5070_CTRL_CMD_SET_RMFREE_FLAGS` (its layout
   derived by `tools/derive_display_layouts.sh`) and on a preserving free keeps the last armed
   composition's planned layers on the monitor until a head is armed again. ⊘ *CORRECTED 2026-10-04
   (`v3-cand-1`, the ⊘⊘ block at the top of this section):* "the last armed composition" was the last
   copy that composed every window whole, and a refresh copy between NVKMS's unbind of the console's
   context DMA and these frees refused the window and left nothing to keep; windows now keep the context
   DMA their ARMED state latched (`LatchedDmas`). ⊘ *CORRECTED 2026-10-03
   (late, the review of `v3-gop-unload`):* it *"marks the channels of the next free"* with ONE model-wide
   flag cleared after every `GSP_RM_FREE` — so another client's flag marked this client's free, and a
   child's free (the guest's RM sends one RPC per object, children first) spent it before the channel's.
   RM keeps the flag on the `DispObject` (`rmFreeFlags`, `ogkm-580: src/nvidia/src/kernel/gpu/disp/disp_objs.c:563-578`)
   for "the next RmFree() only" (`ctrl5070chnc.h:901-913`). Now the mark is the display object's the
   control names (`(hClient, hObject)` of the `GSP_RM_CONTROL`; NVKMS sends it to `displayHandle`, the
   channels' parent, `nvkms-rm.c:2815-2819`, `:3010-3013`); a channel's free reads its parent's mark; the
   free that read it — or freed the display object or its client — spends it; an unrelated free neither
   reads nor spends it; at most 64 marks. Where GSP-RM clears it is closed firmware (`ogkm-580` has the
   accessors and no caller).
4. **`unload_hook.sh`**: per-step guest uptime; (c2)/(c3) as the positive control; (a) and (a2) set their
   lightdm session explicitly and restore the image's after; each arm prints its expectation.
   ⊘ *CORRECTED 2026-10-03 (late, the review of `v3-gop-unload`):* *"prints its expectation"* graded
   nothing — the hook always exited 0 and so did the lane (`lane.sh` ended with an echo). Now each arm is
   judged (`DISPLAY_B5_JUDGE`), `DISPLAY_B5_VERDICT PASS|FAIL` lists every arm, the hook exits 1 on any
   mismatch or skipped arm, and `lane.sh` exits 3 on a failing or missing verdict. And the stale-evidence
   trap that misread the first (a) run was still open: `b5f/b5a_Xorg.0.log` reads *"Time: Sat Oct 3
   18:00:45"*, before b5f started (18:05:24) — it is d2's (a2) log, and b5f's (a) line printed its
   `x_driver=` from it. Each X arm now moves the old `/var/log/Xorg.0.log` aside and reads the file only if
   it is newer than the arm's start; device checks read only QEMU log lines written after the arm began.

⊘ *2026-10-03 (late), at `06b307c4` (`gop_final_20261003/run_h/b5h/hook/b5_device.log`, every teardown logged):*
for the four teardowns whose RM still held the console at BAR1 VA 0 the unmap and the register write
land in the same ms and the re-seed ≤ 1 ms after (t = 48.551/48.552, 76.086/76.087, 115.915/115.915,
149.751/149.752 s). The fifth, in (b), is different: `fbdev=1` had already replaced the console at VA 0 with
nvidia-drm's surface (store `0xa00000`, t = 196.517 s); `rmmod` unmapped it at t = 210.110 s, 406 ms before
RM's teardown wrote the register — with fbcon unbound, nothing drew into BAR1 then.

**The trigger, decided from source and measurement.** Two guest acts give BAR1 up, in this order
(`gpuStateUnload` then `gpuStateDestroy` → `kgspUnloadRm`, `ogkm-580: src/nvidia/arch/nvalloc/unix/src/osinit.c:2352-2375`,
`src/nvidia/src/kernel/gpu/gpu.c:3970-3975`): CPU-RM's `NV_PBUS_BAR1_BLOCK` write, then fn 47, after which
GSP-RM runs its own unload. Both are honoured and the second is a no-op (`[measured b5f]` *"already shows
its physical view"*). ⊘ *CORRECTED 2026-10-03 (late, the review of `v3-gop-unload`) — the reasoning
that stood here:* *"the register write is the primary one because the guest holds the console lock across
the whole teardown … fbcon's first write after it comes after fn 47's reply"*. The lock is real
(`os_disable_console_access` … `os_enable_console_access`, `osinit.c:2352`, `:2375`, = `console_lock()`,
`kernel-open/nvidia/os-interface.c:75-78`), but it orders only fbcon's drawing into simpledrm's SHADOW
buffer. The write that reaches BAR1 is the fbdev helper's damage worker — `drm_fb_helper_damage`
schedules `damage_work` and `drm_fb_helper_damage_work` blits the damaged rectangle later, on a
workqueue, outside `console_lock` (Linux 7.1-rc6 `drivers/gpu/drm/drm_fb_helper.c:268-276`, `:446-463`;
simpledrm's fbdev is `drm_fbdev_shmem`, `drivers/gpu/drm/sysfb/simpledrm.c:21`; the 6.8 guest's
generic shadowed fbdev takes the same path — the review's reading, not re-read here) — and a KMS
client's commit (muffin in arm (a)) is not under the lock either. **The real ordering:** a blit queued
before the teardown (or a client's commit during it) can land in BAR1 `[0, G)` between the console's
unmap and the re-seed, and lands in scratch: that rectangle stays stale until it is drawn again. The
margin is that window's length — `[measured b5f at 4a4b95f7]` the unmap and the register write in the
same ms, the re-seed ≤ 1 ms after it (for the 4 teardowns the instrument logged; the 5th's unmap was past
its bound, below) — not a lock. A real card has the same kind of window: `kbusStatePreUnload_GM107`
unmaps the preserved console before `kbusTeardownMailbox_GM107` writes the physical mode
(`kern_bus_gm107.c:746-787`, `:1278-1310`), and a CPU write to BAR1 VA 0 in between reaches no console.
⚠ The re-seed is asynchronous (the VA thread): the window is measured, not bounded.

**Why `[0, G)` and not all of BAR1.** On a real card physical mode is BAR1 `[0, bar1-size)` → FB. kf3
restores only the boot framebuffer: the firmware console is the only user of BAR1 in physical mode (a
Linux guest's efifb/simpledrm, Windows' Basic Display on the GOP), and a whole-BAR1 view would need
`bar1-size` more host BAR1 for the moment an RM takes BAR1 back (its new views are placed before the
physical view is released, scratch-first) — 128 MiB on the default device, against `G` (8 MiB at 1080p)
already in `Demand::with_boot_fb`. `[G, bar1-size)` stays scratch while no RM holds BAR1.

**Constraints.** No trap is added (the register write is an ordinary privileged-ring item); the drainer
only bumps an atomic; host verbs run on the VA thread; the re-seed replaces scratch and releases nothing,
and retirement keeps place → sink → release; no VMM address reaches the guest. Hostile guest: a write
storm costs one atomic per write and at most one re-seed per BAR1 change; the log lines are bounded.
⊘ *CORRECTED 2026-10-03 (late, the review of `v3-gop-unload`):* they were not all bounded, and the
bounded ones ran out too early. The fn-47 decode line printed once per fn 47; the *"console shows"*
digest named each window's context DMA and offset, so a page flip changed it and `[measured b1f at
4a4b95f7]` its 256 lines were spent in ~4 s of flips; and the BAR1 boot-range line shared one counter
with the re-seed lines, so `[measured b5f at 4a4b95f7]` it stopped at change #88 and the fifth teardown's
unmap was never logged (*"every teardown: SCRATCH → write"* held for 4 of 5). Now each family has its own
bound and one closing *"N … lines logged — later ones are not"* line: BAR1 boot-range view 256, printed
only when what `[0, G)` shows CHANGES (about two per RM life); re-seeds 128; BAR1-mode writes 64; fn-47
decodes 64; *"console shows"* 256, keyed without what a flip changes and compared as a hash (formatted
only when it changes). And the coalescing of the two triggers: ⊘ the change baseline was taken at the
FIRST request's notice and a later request merged in without refreshing it, so a register-write
trigger skipped for a BAR1 change swallowed fn 47's too; every newly noticed request now re-baselines
(`kf_qemu::bar1phys::PhysicalViewDue`, unit-tested).
`gop=off`: no boot range, so no re-seed and no boot-range or re-seed line (`[measured b0f at 4a4b95f7]` none).
⊘ *Scoped 2026-10-03 (late, the review of `v3-gop-unload`):* "no line" is those two families only — the
BAR1-mode write and fn-47 lines print with `gop=off` too, and request nothing (`[measured b0h at
06b307c4]`: two teardowns logged, t = 47.889 s and 97.157 s, `B0_SEED_LINES=0`).

**Not modelled / open:**
- Physical mode for BAR1 `[G, bar1-size)` (above). A guest that keeps a mapping inside `[0, G)` through
  its teardown keeps it (re-seed refused by name, its own console).
- ⊘ *CORRECTED 2026-10-03 (late):* ~~`Shown::Blank` also applies with `gop=off` once a frame was shown, and
  on GB20x (no window vocabulary until M5) the boot layer is now followed by black instead of its last
  frame.~~ GB20x keeps the boot layer's last frame (no head ever scans a window it can name), as before
  this branch. `gop=off` (with `display=on`) does change, and so do its logs: a lost scanout after a
  head first scanned a window is black (it froze the last frame), a `PRESERVE_HW` free keeps the
  preserved scanout, and the BAR1-mode write and fn-47 lines print (bounded); the BAR1 re-seed does
  nothing without a boot range. None of this was run on GB20x or any non-GA10x box.
- GSP-RM's behaviour on a channel free WITHOUT `PRESERVE_HW` is closed firmware; kf-disp shows black then.
- One *"Flip event timeout on head 0"* at `rmmod nvidia_drm` (in the first run and in b5f): nvidia-drm's
  last commit waits 3 s for a flip event kf-disp does not deliver. Not investigated here.
- (d) — a 7.x guest's `nv_get_screen_info` path — was not run.

---

## 5. Plan

Each milestone ends with a measured boot on a GA10x box, its evidence committed under
`traces/v3_display/<run>/`, and the merge bar unchanged (§4.9). The display plane is behind a device
property, **`display=off` by default** until M3, so no branch state can move the bar; M3 flips the
default and runs the whole bar with it on (`THE_CONSTRAINTS.md` §42/§44: a default is flipped with
its dependency chain, in one commit).

| milestone | the guest can | what is built | grade (lane `scripts/bench/display/`) |
|---|---|---|---|
| **M0** bring-up | boot with KernelDisplay present; `nvidia-modeset` loads; NVKMS `ALLOC_DEVICE` succeeds | `kf_chip::display` rows; `kf-rm` display link answering the init controls (IP version, static info, brightc, ACPI EDID, inst-mem, IMP, interrupt rows); the display classes in the class sets and the capability table | guest dmesg: no `kdisp*` failure, `nvidia-drm` registers `card0` **with a connector**; the unserviced ledger holds no display id; compute unchanged |
| **M1** a monitor | `modetest` / `kfdisp_probe list`: one connected DVI-D (or DP, §6.2) connector, EDID modes, primary + overlay + cursor planes per head | NV0073/NV5070/NVC370/NVC372 physical controls for a virtual TMDS output; the caps registers; the core channel: PUT trap → worker, decode, ARMED mirror, GET, completion notifier | `DISPLAY_CONNECTED=yes`, modes = the EDID's, `nvidia-settings -q dpys` names the monitor |
| **M2** first light | set a mode, flip, see it outside | window + window-immediate channels, ctxdma resolution through instance memory, head timing + `timerfd` vblank + interrupts, the scanout copy kernel (pitch + block-linear, XRGB8888), the QEMU console per head, VNC | `kfdisp_probe show`: `FLIPS=120/120`, `flip_hz` ≈ refresh; host `screendump` **pixel-exact** against the reference PPM; fbcon visible over VNC |
| **M3** desktop | Mint's own session: lightdm → Xorg (NVIDIA DDX) → Cinnamon | cursor channel; whatever the DDX and `libnvidia-egl-gbm` ask next (read off the ledger and the guest's logs, bare metal first) | desktop screenshot; `glxinfo -B` = NVIDIA (no `llvmpipe` anywhere); `vkcube` runs (Xlib WSI); a deterministic GL frame's screendump equals the guest's own readback of it |
| **M4** the display set | the `V3_GFX_TESTSET.md` §7 list on the virtual head | fixes the set finds | per-item verdicts vs bare metal on the same box (a forced-EDID head on the host) |
| **M5** breadth | Ada, GB20x, TU10x; several heads; resize via hot-plug; zero-copy GL present | per-family rows (generated), HPD events, dma-buf export to a GL UI | the lane per family box |

### 5.1 Measurements (recorded, not graded)
- **Frame rate and latency**, nvkvm-pv's method (`present-backpressure.md`, `NVKVM_PRESENT_TIMING`):
  per-second counters of flips latched, scanout copies completed, frames handed to the console, and
  dropped frames; the copy's GPU time from CUDA events; flip-to-console latency from the latch timestamp.
- **vblank period** measured from the guest (`kfdisp_probe` event timestamps) against the timer's.

### 5.2 Grading rules carried over (each paid for elsewhere)
- A display result cites the kf3 binary's revision (`boot_capture.sh`).
- `glxinfo`'s renderer string alone is not acceleration evidence; FPS against an llvmpipe control is
  (nvkvm-pv `mint-guest-desktop.md:744-745`).
- The frame is judged by its pixels (screendump vs reference), never by the absence of an error.
- Bare metal first: every desktop item runs on the host's GPU with a forced EDID before a guest
  failure is called a kayfabe defect.

## 6. Risks and unknowns, ranked

1. **The physical display semantics are GSP firmware's, which is closed.** What NVKMS and the CPU-RM
   expect back is readable (ogkm, nouveau's GSP path), what the firmware *does* is not. Mitigation: the
   compliance principle — serve what the open code checks; nouveau's GSP-mode client documents the
   minimum (§4.2); bare metal can be traced with `nvdiff` extended to NVKMS payloads.
2. **Connector type.** DVI-D over TMDS needs only an EDID; DP needs an emulated DPCD sink and link
   training (`NV0073_CTRL_CMD_DP_*`). DVI caps a mode at 165 MHz (1920×1200@60); larger sizes need
   HDMI 2.0 or DP. Start with DVI-D; add DP when a size needs it.
3. **Block-linear layouts per family.** Turing–Ada share the GOB layout; Blackwell adds kinds. The scanout
   kernel is checked against a host Vulkan render with a known modifier before a family is claimed.
4. **Timing-sensitive waits in NVKMS** (notifier BEGUN vs FINISHED, supervisor-driven waits, idle
   polls): a mismatch shows as an NVKMS timeout in the guest's log, not as corruption — loud.
5. **Store reads for scanout contend with guest work** on the same GPU: one 1080p copy is ~8 MB per
   frame; measured in M2.
6. **Driver-version skew** of NV0073 ids and layouts (r535 vs r570/580 renumbered `SYSTEM_*`):
   the layouts go through the driver matrix (`tools/drivermatrix`), never hand rows per version.

## 7. Evidence

- Source audits (2026-09-27): the guest CPU-RM display path (ogkm-580 `src/nvidia/src/kernel/gpu/disp`),
  NVKMS (ogkm-580 `src/nvidia-modeset`, `kernel-open/nvidia-drm`), nouveau's GSP display client
  (`research_clones/nouveau-src`, Linux 7.3-rc3, `nvkm/subdev/gsp/rm/r535/disp.c`), nvkvm-pv's display
  stack (`368d2db`, `main` `a1f8ec3`, `integration/candidate-2026-09-18` `13e1c9a`).
- Measurements: see §5 per milestone, under `traces/v3_display/`.

## 8. The display broker — display step 3

> ⊘⊘⊘ **CORRECTED A THIRD TIME 2026-10-03 — the third review found one major and one stale design
> line; both fixed on `v3-broker`, nothing run on a box.**
> - **Major: after the withdrawal, an OWED frame tore down a healthy broker connection.** The
>   relay's `attach()` turned a slot that no longer fits (`withdraw_all`, run on the worker's thread,
>   flips `broker_backed` for held and owed slots too) into `Sent::Failed`, which every caller reads
>   as a dead socket: an owed ATTACH (or the replay's) after a refused backing dropped the connection,
>   blamed the socket, and lost input until the reconnect — so "the relay keeps running, so input still
>   flows" (§8.2) was false there. The same path was reachable through the cross-thread window between
>   the relay's `fits()` and its ATTACH. And an owed COMMIT was still sent after the withdrawal,
>   contradicting "sent no frame after the refusal". **Now** an unfit slot is a refusal on every path
>   (live, owed, replay): the frame is refused by name (`REFUSED frame slot … withdrawn`), its slot
>   comes back, nothing more of it goes — no ATTACH, and **no COMMIT once the relay has observed the
>   withdrawal**, the owed COMMIT of a frame attached before it included (that frame is never shown,
>   like a superseded one) — and the connection stays ACTIVE. Tests that fail on the previous code:
>   `relay_machine.rs` `an_owed_frame_after_the_withdrawal_is_refused_and_the_connection_stays_up`
>   (owed ATTACH by writability and by the timer, owed COMMIT) and
>   `a_withdrawal_between_two_sends_refuses_the_frame_and_keeps_the_connection` (the worker withdraws
>   between the WINDOW and the ATTACH, between the ATTACH and the COMMIT, and during a replay).
> - **§L supersedes the zero-copy plan** (`OWNER_RULINGS.md` §L, 2026-10-03): no guest surface and no
>   slice of the store is ever exported to the broker. Each place this document planned "zero-copy
>   export of the guest's surface with NVIDIA's modifier" now says so in its own text (the top
>   NEXT block, §4.6, §8.2's expectations, §8.6's *Not built*). The replacement is a GPU→GPU copy into
>   kayfabe's own VRAM frame object (§8.11); the host-RAM rungs of §8.2 become its fallbacks.

> ⊘⊘ **CORRECTED AGAIN 2026-10-03 — the re-review of the fixes below found item 2 half-built and
> two bench gaps. All four are fixed here; nothing has run on a box.**
> - **Item 2 was false as built:** only a slot the worker REALLOCATED after a refused broker backing
>   was withdrawn. A slot that kept its broker memfd stayed offered, so the broker went on receiving
>   frames (`broker[sent]` kept rising) while the worker had logged that it would be shown nothing.
>   Now the first refusal withdraws the WHOLE ring (`FrameRing::withdraw_all`): no slot, whether
>   reallocated, kept, ready, held or requeued, is offered or sent again (§8.2, §8.7 (13)).
> - **The fix's call site had no test:** every test called the ring directly. The worker's choice is
>   now a GPU-free function (`display.rs` `broker_backing`) that a kf-qemu test drives with a refusal,
>   and the relay's own check in `fits()` has a test that fails without it: a held frame requeued
>   by a broker restart after the withdrawal is refused, not replayed (§8.2).
> - **The bench:** the branch now carries master's `4b077201` (`provision_host_driver.sh` purges the
>   packaged driver at any version and stops the host display manager), without which provisioning
>   fails on the vast "Ubuntu Desktop (VM)" template. §8.9 (5) now says how to bring that template's
>   desktop back on the new driver and how the broker reaches its session under the peer policy.

> ⊘ **CORRECTED 2026-10-03 — the adversarial review of this branch (the same day) found twelve
> defects; all are fixed in code here, each with a test that fails without its fix (run against the
> unfixed code or a bite-mutation, §8.8). Nothing has run on a box.**
> 1. **A permanent display freeze** (§8.3): an owed COMMIT superseded by a newer frame while the
>    latest commit and the superseded frame filled the cap of 2 — nothing was ever committed again.
>    The superseded frame now yields its slot.
> 2. **Stale pixels after a partial backing refusal** (§8.2): a slot refilled with the console's own
>    memory went on naming its previous memfd. It is now withdrawn from the broker. (⊘ Corrected
>    again above: that withdrew only the refilled slot; now the whole ring is withdrawn.)
> 3. **The peer policy admitted the owner of the socket's directory** (§8.5) — anyone, when the
>    directory is missing under `/tmp`. Now uid 0, QEMU's euid at each connect, and
>    `display-broker-uid` only.
> 4. **QEMU's euid was read once, at realize** — root's, before `-run-with user=`/`-runas` dropped
>    privileges (§8.5). The first attempt now runs from the main loop's timer, and every attempt reads
>    the euid.
> 5. **An out-of-range `display-broker-uid` was silently ignored**; it is refused by name at realize.
> 6. **The `REFUSED frame` line was logged per frame** (30–60 a second); now 1–4 and every 256th.
> 7. **Frames were refused on 64 KiB-page hosts** (1920×1080×4 is not 64 KiB-aligned); backings are
>    rounded up to whole host pages.
> 8. **The `ui_info` hook changed the GTK/VNC path without a broker**; it is installed only with one.
> 9. **Design §1.3's absent-tablet log was neither built nor listed**; it is built (§8.4).
> 10. **§8.9 named ABI 11**; the branch is ABI 12.
> 11. **The slots TOCTOU test asserted nothing for the console**, and had no known-positive (§8.3).
> 12. **The full-backlog test depended on `ulimit -n` and `somaxconn`** (§8.1).

**STATUS: LIVE — 3a, 3b, 3d and 3c BUILT IN CODE, GPU-FREE, 2026-10-03 (branch `v3-broker`).**
Local runs are §8.8; nothing has run on a box (renting needs the owner's approval). This section folds in the reviewed design (the adversarial review of 2026-10-03 applied);
where the code departs from it, §8.7 says so.

### 8.0 Decision

nvkvm-pv's broker process and wire protocol v2 are kept **unchanged** (`nvkvm-pv
src/common/nvkvm_broker_proto.h` at `368d2db`, vendored verbatim as
`crates/kf-broker/proto/nvkvm_broker_proto.h`). Its QEMU relay (`src/qemu/nvkvm_display_relay.c`,
2196 lines) is **not** copied into kf3.c; it is split along the line the architecture draws:

| piece | where |
|---|---|
| wire codec; the connection machine (connect, peer check, HELLO, replay, owed frame, reconnect, verdicts, rung, pacing, RELEASE accounting, reclaim); the frame ring | **`crates/kf-broker`** — new, light, safe code (workspace lints), VMM-agnostic, deterministic under a caller-supplied clock |
| `AF_UNIX` connect, `SO_PEERCRED`, `sendmsg`+`SCM_RIGHTS`, `recvmsg` without a control buffer, `UDMABUF_CREATE`, `fstatfs`, `fstat` ids | `crates/kf-linux-raw` (`unixsock_unsafe.rs`, `host_fd_unsafe.rs`) |
| frame backing (memfd + `cuMemHostRegister` + udmabuf), the relay seat, KF3 ABI 12 (⊘ corrected 2026-10-03: said 11; 3c added two entries, §8.7 (6); ⊘ 13 since the GPU-copy rung, §8.11; ⊘ 12 again since the merge with master's boot display, which took 11) | `crates/kf-qemu/src/broker.rs`, `display.rs`, `ffi_unsafe.rs`; `crates/kf-cuda` (registration) |
| fd handlers, the timer, `qemu_input_*`, the relative-pointer switch, the close policy | `qemu/hw/misc/kf3/kf3.c` (about 230 lines; QEMU 10.2 only) |

The relay runs on QEMU's **main loop** (one socket owner, as in nvkvm-pv). The display worker never
touches the socket: it publishes into the ring (a CAS) and writes one non-blocking eventfd. No lock is
shared between the worker and the main loop. Nothing new runs on a vCPU. ⊘ Narrowed 2026-10-03 by
§8.12: the cursor mailbox is a mutex BOTH sides only `try_lock`, so the rule is now "no lock is ever
WAITED on between the worker and the main loop" (a busy mailbox is retried on the next frame or
entry).

Usage:

```
-display none \
-device kf3-gpu,id=kf0,display=on,display-broker=/run/user/1000/nvkvm/display.sock \
-device virtio-keyboard-pci \
-device virtio-tablet-pci,display=kf0,head=0
```

**Who may be the broker** (§8.5): uid 0, QEMU's effective uid at each connect, and
`display-broker-uid` when set. A QEMU started as root with the broker in a desktop user's session
therefore needs `-run-with user=<that user>` (QEMU then runs as the broker's uid) or
`display-broker-uid=<the broker's uid>`; otherwise the relay refuses the listener, loudly, and keeps
retrying.

One broker and one socket per kf3 device (a broker serves one VMM and never displaces it). The broker
is **installed separately**, built from nvkvm-pv at a pinned revision (`368d2db`); it is not part of
kayfabe (owner default, 2026-10-03). Realize refuses by name: `display-broker` without `display=on`; a
relative path (which includes the `@name` abstract spelling); an embedded NUL; a path of 108 bytes or
more. A broker that is not running is **not** an error: the relay retries in the background and the VM
boots regardless.

⊘ **CORRECTED 2026-10-08 (run p4, kf3 `0e64a960`, §8.17): the next sentence's consequence did not
hold there.** Two clean guest `reboot`s in one QEMU process, broker connected, both came back: QEMU
alive, grub and the login prompt on the serial port, `nvidia-smi -L` answering, 0 Xid, no
`RmInitAdapter failed`, no WPR text, and the NVIDIA X desktop back in the frame. What a reset does NOT
redo was not examined. **Reset:** kf3 has no reset path — a guest reboot needs a QEMU restart (owner default). The broker
connection and the frame slots are VM-lifetime state. **Boot display / Secure Boot:** not involved in
this step (the boot framebuffer is the GOP option ROM's, step 1; Secure Boot stays off for it).

### 8.1 nvkvm-pv invariants kept, each with a test

| # | invariant | test (`crates/kf-broker/tests/relay_machine.rs` unless named) |
|---|---|---|
| 1 | fixed 24/40-byte records; a short read builds up to one packet; a short write is fatal; never resync | `a_short_read_builds_up_to_24_bytes`; `link.rs` (`n != 40` → fatal); `proto_mirror.rs` (layouts) |
| 2 | HELLO first, version 2, `CAP_DMABUF` required | `hello_must_come_first_with_version_2_and_dmabuf` |
| 3 | nothing waits; a 2 s handshake limit | `the_handshake_has_a_two_second_limit` |
| 4 | not a startup dependency; backoff 200 ms doubling to 5 s; loud once | `a_failed_connect_backs_off_200_400_to_5000` |
| 5 | replay WINDOW → ATTACH → COMMIT → CAPS | `the_replay_is_window_attach_commit_caps_with_the_frames_flags` |
| 6 | a newer frame during the replay COMMIT restarts the replay | `a_newer_frame_during_the_replay_commit_restarts_the_replay` |
| 7 | EAGAIN on ATTACH owes the ATTACH, on COMMIT only the COMMIT; retry on writability, 50 ms backstop; a stale WINDOW first; a newer frame replaces what is owed | `an_owed_attach_and_an_owed_commit_are_different_debts`, `a_newer_frame_replaces_an_owed_one` |
| 8 | per-connection state dropped on disconnect (one `Conn`, dropped whole) | `a_reconnect_forgets_the_connection_and_replays_the_latest_frame` |
| 9 | WINDOW only on a size change | `window_is_sent_only_when_the_size_changes` |
| 10 | counters: frames announced, sent, dropped, uncommitted, recovered, releases, reclaims … (`kf3_status` `broker[…]`) | — |
| 11 | EV_FORMAT matched by the echoed pair; an untracked pair ignored | `a_later_no_downgrades_the_rung_and_reclaims_its_frames` |
| 12 | unknown event types skipped exactly | `input_is_bounded_before_the_vmm_sees_it` |
| 13 | **new:** the ATTACH flags (F_SHM) travel with the frame on the live, owed **and replay** paths | the replay test above; `broker_loopback.rs` `shm_frames_and_an_shm_replay_after_kill_9` |

Fixes over nvkvm-pv, all built: (a) a later "no" downgrades the rung and reclaims the frames attached
under that pair; (b) the horizontal wheel is deliberately **not** added (QEMU 10.2's virtio and USB
pointers map no `WHEEL_LEFT/RIGHT`); (c) the peer-credential check (§8.5); (d) pacing — a COMMIT spends
the credit, `EV_FRAME` **or** the `RELEASE` of the latest commit returns it (the X11 XRender path never
sends FRAME), a 100 ms backstop is the last resort (`pacing_credit_returns_on_frame_release_or_the_backstop`);
(e) the replay carries F_SHM; (f) rung 1b. Also: **no CONNECTING state** (an `AF_UNIX` `EAGAIN` is a full
backlog, a failed attempt — `kf-linux-raw` `a_full_backlog_is_eagain_and_never_in_progress`; ⊘ corrected
2026-10-03: its listener now has a backlog of 1, as it needed more than `somaxconn` descriptors and
failed under `ulimit -n 1024`), and at
most 64 packets per call with the tail left in the socket
(`at_most_64_packets_per_call_and_the_tail_stays_in_the_socket`).

### 8.2 Frames and the rungs

With `display-broker` **unset**, frames stay `cuMemAllocHost` and there are 3 slots — the console path
is unchanged. With it **set**, each of **5** slots is a sealed memfd (`kayfabe-display-frame`, sealed
`SHRINK|GROW|SEAL`, never `WRITE`), mapped and page-locked with `cuMemHostRegister_v2` so the existing
asynchronous D2H scanout copy lands in it, plus a udmabuf over the same pages when `/dev/udmabuf` opens
(`root:kvm 0660`). The frame is registered **before** its descriptors enter the ring, so the ring never
names a backing the GPU does not write.

⊘⊘⊘ **CORRECTED A THIRD TIME 2026-10-03 (the third review): two sentences of the paragraph below
were false for an owed frame.** "The relay keeps running, so input still flows": an owed ATTACH (or
a replay's) after the withdrawal came back from `attach()` as `Sent::Failed` and DROPPED the
connection. "Sent no frame after the refusal": an owed COMMIT still went. Both hold now on every path
— an unfit slot is a refusal, never a socket failure, and the relay commits nothing once it has
observed the withdrawal (§8's top correction; the two `relay_machine.rs` tests named there).

⊘⊘ **CORRECTED AGAIN 2026-10-03 (the re-review): the paragraph below still overstated.** It
withdrew only a slot the worker REALLOCATED after the refusal; every slot that kept its broker memfd
(the GPU still writes it, and it was never refilled) stayed offered, so the broker kept receiving
frames, `broker[sent]` kept rising, and the worker's log line said the opposite. **As built now:**
the worker's first refused broker backing calls `FrameRing::withdraw_all` before any slot is
refilled. From that call on, no slot is offered to the broker or sent by the relay: not one that was
reallocated or kept, not the frame that was ready (it is dropped), and not a held frame requeued for
the replay of a restarted broker. A later install does not undo it, and the seat is never asked
again. Frames the broker already holds stay held until it releases them. The relay keeps running, so
input still flows. The per-slot `withdraw(slot)` is gone; `broker_backed(slot)` is now "carries a
broker backing, and the ring is not withdrawn". Tests, each shown to fail without its fix (§8.8):
kf-qemu `a_refused_broker_backing_withdraws_every_slot_from_the_broker` drives the worker's
decision (`display.rs` `broker_backing`, GPU-free, the seat's result an input) with a refusal;
`slots.rs` `a_withdrawn_ring_offers_the_broker_no_slot_again`; and `relay_machine.rs`
`a_withdrawn_ring_sends_the_broker_nothing_not_even_a_replay`, where the relay's own check in
`fits()` is what stops the replayed frame. A refusal on the replay path is now logged at the same
bounded rate as the live path's, and names the withdrawal. ⚠ No box test injects a refused backing;
the path is GPU-free-tested only.

⊘ **CORRECTED 2026-10-03 (the review of this branch): the next sentence was false as built** (and,
⊘⊘ per the correction above, the fix described here was only half of it). After a refusal the worker
refilled a slot with the console's own memory while the ring
still named the slot's previous memfd, which the GPU no longer wrote: once the mode fitted it again
the broker was sent those stale pixels, and until then the relay logged `REFUSED frame slot` for
every frame. That fix withdrew such a slot before refilling it (a per-slot `FrameRing::withdraw`,
since replaced by `withdraw_all`); any `REFUSED frame` line left is
rate-limited to the first four and every 256th (`a_refused_frame_is_logged_at_a_bounded_rate`). Each
backing is also rounded up to whole host pages (`kf_broker::frame_bytes`): 1920×1080×4 is not
64 KiB-aligned, and on a 64 KiB-page host every frame was refused.

A refusal (no `cuMemHostRegister`, or a driver that will not
pin these pages) is logged once by name; the console keeps its own frames and the broker is shown
nothing more: it is sent no frame after the refusal (the whole ring is withdrawn, above), and keeps
showing the last frame it received. A CPU copy is never the fallback. The descriptor: `XR24`, `offset 0`, `stride = width × 4`,
`LINEAR` or `MOD_INVALID`.

1. **LINEAR dma-buf** — udmabuf present, `CAP_MODIFIERS` set, verdict for (XR24, LINEAR) not "no" (an
   unknown verdict counts as yes).
1b. **Implicit-modifier dma-buf** — udmabuf present, `CAP_MODIFIERS` clear, and an explicit yes to
   `QUERY_FORMAT(XR24, MOD_INVALID)`. ⚠ That importers treat an implicit-modifier udmabuf as linear is
   a box question.
2. **F_SHM memfd** — always available; an X11 broker takes it only with `--present-mode=shm`, and a
   refusal is not reported back (the relay's log says so once).

⊘ **SUPERSEDED IN PART 2026-10-03 by `OWNER_RULINGS.md` §L** (broker frames are a GPU→GPU copy into
kayfabe-owned VRAM, never guest memory): the expectation below still describes the host-RAM rungs,
but on a compositor on the **same NVIDIA GPU** they are now §L's **fallbacks** behind rung 0, the
GPU-copy rung (§8.11: a block-linear dma-buf of a VRAM frame object kf3 allocated itself; no byte
crosses PCIe, and nothing of the guest's is exported). A compositor on another GPU or vendor keeps
rung 1 (the host-RAM LINEAR udmabuf), and F_SHM stays the last resort. "Zero-copy" below means
zero-copy *for the broker*: kf3 still makes the one GPU copy into the frame.

Expected: rung 2 on an all-NVIDIA Wayland desktop (NVIDIA's GL refuses LINEAR dma-bufs), rung 1
zero-copy on Intel/AMD compositors, rung 2 only without `/dev/udmabuf`.

Pinned memory with the broker on: 5 × 7.9 MiB at 1080p; after a 4K mode the slots grow and the 1080p
backings stay retired, so the worst case is 5 × (7.9 + 31.6) ≈ 198 MiB (the `kf-disp/src/scanout.rs`
comment says so). No host-RAM backing for the boot framebuffer (owner default).

### 8.3 The one state word, and reuse after RELEASE

`kf_broker::FrameRing` holds all occupancy in ONE `AtomicU32` — console front (bits 0-3), console ready
(4-7), broker ready (8-11), broker-held mask (12-16) — and replaces `ConsoleShare`'s word. Every
transition is a CAS; the worker's fill target is a slot named nowhere in the word. The broker holds at
most **2**, so at most four slots are occupied (front, last published, two held) and with five a fill
target always exists (`with_five_slots_a_fill_target_always_exists_…`; the interleaving test
`the_worker_never_picks_a_slot_another_thread_holds` races a real worker, relay and console thread).
⊘ Corrected 2026-10-03: the interleaving test asserted only the relay's marks, and nothing showed it
could fail. Both readers now mark a slot after the transition that gives it to them and unmark it before
the one that gives it back, so every mark is asserted, and `the_harness_catches_the_two_word_ring` runs
the same harness against a two-word model of the ring, which it must catch (locally: thousands of
violations in 200 000 rounds; the one-word ring: none).
`free_slot()` returns `Option`; `None` is counted (`scanout_no_slot`), never papered over with slot 0.
Descriptors are never closed while the device lives (retire-never-free, per slot at most two backing
generations in `OnceLock`s), so a recycled descriptor number can never be sent. RELEASE is matched only
against the id of the descriptor actually sent on that frame's rung (memfd and dma-buf inodes come from
different counters); identities are checked distinct across every slot and generation at install.

⊘ **CORRECTED 2026-10-03 — a fourth way a held frame comes back, and without it the display froze for
good.** When the broker stalls long enough for the socket to fill, an ATTACH can go while its COMMIT
meets `EAGAIN` (an owed COMMIT), and the next frame supersedes it. That frame is then never shown —
no COMMIT follows its ATTACH; the next one follows the newer frame's — and never RELEASEd, it is not
the latest commit, and the reclaim rule below needs a newer commit than it: with it and the frame on
screen filling the cap of 2, no frame could ever be committed again (the review's scripted probe:
`sent=1 blocked=61 reclaims=0`). A **superseded** frame now yields its slot to the live frame the way
the retained one does (`a_superseded_owed_commit_never_freezes_the_display`). Measured locally against
the real broker: a 40 s SIGSTOP commits ~2 frames a second (the cap of 2 and the 1 s reclaim), so the
socket did not fill in 40 s — the trigger needs a longer stall, and the scripted case is the
regression test.

**Reuse without RELEASE (owner question 6, implemented as the reviewed default).** "Reuse a copy only
after the broker's RELEASE" cannot be kept literally: RELEASE is advisory and a rejected ATTACH never
gets one. So a held frame is also reclaimed (counted, logged): on `EV_FORMAT x=0` for its pair; and
**1 s after its commit once a newer frame was committed**; and on disconnect. A `--persist` broker may
go on showing a slot overwritten after a disconnect. The worst case is a torn frame; the pages stay
valid (udmabuf and the registration pin them).

### 8.4 Input — broker packet → QEMU (kf3.c, ported from `relay_handle`)

⊘ **CORRECTED 2026-10-08 (run p4, §8.17): the GRAB row's "mouse-look is known not to work" is not
supported by a measurement.** Under the broker's CTRL+ALT+G the relay put the Virtio mouse in front
(`pointing device -> #5 QEMU Virtio Mouse (relative)`), sixteen REL packets summing (70, 30) arrived on
the guest's `QEMU Virtio Mouse` evdev as REL_X 70 / REL_Y 30, the guest X pointer moved by exactly
(70, 30) under libinput's flat profile, an ABS sent while grabbed was dropped by the broker, and the
release put the tablet back. kf3's input path is nvkvm-pv's `relay_handle` (`badf2d7`/main) with the
same QEMU calls (§8.17 lists the differences). What remains is a property, not a failure: buttons
and the wheel go to the console-bound tablet in both modes, so under grab motion and clicks come from
two guest evdev devices — an application that reads ONE raw evdev device would see motion without
buttons (inferred, no such application was run).

Rust bounds every value (`kf_broker::Input`) and kf3.c dispatches on kf3's own console:

| event | QEMU 10.2 call |
|---|---|
| KEY (evdev ≤ 0x2ff) | checked against `qemu_input_map_linux_to_qcode[_len]`, then `qemu_input_event_send_key_qcode(con, qemu_input_linux_to_qcode(x), down)` |
| BTN | LEFT/RIGHT/MIDDLE/SIDE/EXTRA → `qemu_input_queue_btn` + `qemu_input_event_sync`; others dropped |
| ABS | only with a non-zero range; clamped to `[0, w−1]`; `qemu_input_queue_abs` X/Y over `0..w0`, `0..w1`; sync |
| REL | consecutive packets summed (saturating); `qemu_input_queue_rel` X/Y; sync |
| WHEEL | vertical only: press and release `WHEEL_UP/DOWN` |
| GRAB | `qmp_query_mice` + `qemu_mouse_set`, preferring Virtio (`relay_set_relative`, copied; mouse-look is known not to work) |
| (connect) | ★ added 2026-10-03 (design §1.3, missed by the first build): at the first connection that passes the peer check, `qmp_query_mice`; with no absolute device, one warning naming `-device virtio-tablet-pci,display=<id>,head=0` (ABS events would find no handler). On the main loop, so a tablet listed after kf3 already exists |
| CLOSE | FORCE → `qemu_system_shutdown_request(SHUTDOWN_CAUSE_HOST_UI)`; otherwise `qemu_system_powerdown_request()`, with nvkvm-pv's repeat-ask message |
| SURFACE | 3c: clamped to 64..8192 and deduplicated in Rust, then `dpy_set_ui_info(con, …, true)` with the broker's refresh (mHz) when it has one — the console's `ui_info` hook does the rest (§8.6) |
| FOCUS / BYE | logged; POINTER, HELLO, CLIPBOARD ignored (no clipboard: CAPS bit 0 is clear) |
| RELEASE / FRAME / FORMAT | consumed by the relay |

### 8.5 Security

The broker stays a separate process holding the display-server connection and the grab; QEMU holds one
socket and imports nothing. The relay never binds, chmods or unlinks.

⊘ **CORRECTED 2026-10-03 (the review of this branch) — the peer policy below admitted a squatter, and
the default is now narrower.** The owner of a directory does not decide who can create the path when
anyone can create the directory: after a reboot `/tmp/kf3` is made by whichever local user runs
`mkdir` first (sticky `/tmp`), who then binds `display.sock`, is shown the guest's screen, types into
the guest and can force it off. The directory's owner is **no longer trusted**. Accepted uids are
**0, QEMU's effective uid read at each connect attempt, and `display-broker-uid` when set** — nothing
else (`only_root_the_vmms_euid_and_display_broker_uid_are_admitted`; on a real socket, as root,
`a_squatter_who_owns_the_sockets_directory_is_refused`, which the unfixed relay failed: it connected
and read the squatter's HELLO). The euid was read once, at realize — root's, before QEMU's
`-run-with user=`/`-runas` drops privileges in `os_setup_post` (QEMU 10.2.4 `system/vl.c:3850-3856`:
after `qmp_x_exit_preconfig` realizes the devices, before the main loop) — so a broker running as QEMU's final uid
was refused. Now `Relay::start` only ARMS the first attempt on the timer, which fires from the main
loop after the drop, and each attempt reads `geteuid()`
(`the_euid_is_read_at_each_connect_after_privileges_are_dropped`). A `display-broker-uid` other than
`-1` or `0..=4294967294` is refused by name at realize (`an_out_of_range_display_broker_uid_is_refused_by_name`).
A root QEMU with a desktop user's broker needs `-run-with user=` or `display-broker-uid` (§8.0).

The text as first built (superseded): **Peer check right after
`connect`, before a byte is read** (`SO_PEERCRED` is fixed at connect): accepted uids are 0, QEMU's
effective uid, `display-broker-uid` when set, and the **owner of the socket's directory** (owner question
1, implemented as the recommended default: that uid already decides who can create the path, so it
admits no squatter). A refusal is loud and retried with backoff. Events are read with `recvmsg` and **no
control buffer**: a descriptor the peer attaches is dropped by the kernel and `MSG_CTRUNC` is a protocol
violation. Outbound descriptors are dedicated, sealed frame copies — never guest RAM or the store — and
are never closed while the device lives. No clipboard (owner default).

### 8.6 Cursor composition (3d), resize (3c), and what is not built

**3d — built in code.** Both broker backends hide the host pointer while a guest frame shows, and
stock compositors on nvidia-drm use the cursor plane, so the composition's TOP layer is now the head's
cursor:
- `kf_disp::engine::CursorVocab` resolves, from the derived class table, the core channel's
  `HEAD_SET_CONTEXT_DMA_CURSOR(h, 0)`, `HEAD_SET_OFFSET_CURSOR(h, 0)` (256-byte units),
  `HEAD_SET_CONTROL_CURSOR(h)` (`ENABLE`, `FORMAT`, `SIZE`, `HOT_SPOT_X/Y`) and
  `HEAD_SET_CONTROL_CURSOR_COMPOSITION(h)` (`K1`, the two factor selects, `MODE`), and the cursor
  PIO channel's `SET_CURSOR_HOT_SPOT_POINT_OUT(0)`; `Engine::cursor_scan` reads them from the ARMED
  state (the point as signed 16-bit: the cursor may hang off the top or left edge). A family whose
  table lacks one composes no cursor (nothing else changes).
- `kf_disp::scanout::plan_cursor` makes it a pitch layer: `A8R8G8B8` only (NVKMS programs nothing else,
  `ogkm-580: src/nvidia-modeset/src/nvkms-evo3.c:6512-6524`), square 32/64/128/256 with pitch
  `max(256, size × 4)` (`:6531-6552`), placed at the point minus the hot spot, clipped on every edge,
  blended with the window factor numbering (`K1`, `K1_TIMES_SRC`, `ZERO`, `NEG_K1_TIMES_SRC`); `XOR`,
  a sysmem or block-linear surface, and bytes past the context DMA are refused by name. The context
  DMA is the core channel's (channel 0 in the hash key).
- The worker composes it last with the existing kernel (no kernel change: a pitch layer with alpha), and
  a cursor channel `Update` on the console's head starts a recompose at once; a new cursor IMAGE (a
  core update) is picked up by the refresh clock (≤ 33 ms watched).
- Tests: `kf-disp` `a_head_cursor_is_scanned_from_the_core_and_its_pio_point` (engine/tests.rs) and
  the three `plan_cursor` cases in `scanout.rs` (CI-compiled: `kf-disp` is not built on the dev host).

**3c — resize, built in code** (owner question 5's narrow seat, as the brief specified):

⊘ Corrected 2026-10-03: the hook is installed **only with `display-broker` set** (`kf3_gfx_ops_broker`
in kf3.c). Installed always, it changed the console path without a broker: GTK's `gd_configure` →
`gd_set_ui_size` → `dpy_set_ui_info` re-authored the monitor to the widget's size, startup size
included, and a VNC `SetDesktopSize` re-moded the guest (QEMU v10.2.4 `ui/gtk.c:1853-1865`,
`ui/vnc.c:2655-2660`). So "from VNC/GTK" in item 1 now holds only when a broker is configured too.

1. A resize hint reaches the console's `ui_info` hook (`kf3_ui_info`, installed only now, because it
   now does something): from VNC/GTK, or from the broker's `EV_SURFACE` — clamped to 64..8192 and
   handed to `dpy_set_ui_info(con, …, delay=true)` (every hint forwarded, as nvkvm-pv's relay does;
   QEMU coalesces for 1 s and calls the hook only on a change).
2. `kf3_display_ui_info` stores the request in one atomic and wakes the display worker.
3. The worker authors the monitor — `kf_disp::edid::Monitor::for_window(w, h, mHz, PCLK_LIMIT)`:
   clamped to 640..3840 x 480..2160 and 24..75 Hz, CEA 1080p60 or CVT-RB, scaled down at the same
   aspect ratio until the clock fits the connector's 165 MHz (DVI single-link; the broker scales the
   rest), with 1080p60 as the EDID's second detailed timing — and puts it behind connector 0
   (`DisplayModel::set_monitor`, deduplicated; a custom EDID the guest set still shadows it).
4. If the monitor changed and a hotplug registration is live, it queues the display id for the
   **register drainer** (the GSP queue's owner), which posts like `deliver_rc`: GSP lock, a LIST
   `POST_EVENT` (`bNotifyList = 1`, the bare `notifyIndex = NV2080_NOTIFIERS_HOTPLUG = 1`,
   `Nv2080HotplugNotification { plugDisplayMask = id }` at `eventData` = +29, the flexible array's
   offset), publish, requeue on `QueueFull`, the GSP stall vector raised outside the lock.
5. The guest RM resolves the pair, wakes NVKMS's callback, NVKMS asks the public
   `SYSTEM_GET_HOTPLUG_UNPLUG_STATE`, and kernel RM asks physical RM **`INTERNAL_GET_HOTPLUG_UNPLUG_STATE`
   (0x730401)** — which the model now claims, returning the pending plug mask and clearing it; NVKMS
   makes an unplug/plug pair, nvidia-drm raises a DRM hotplug, and userspace's reprobe reads the new
   EDID.
- **The registration seat** (`kf_rm::display::DisplayRegistry`, `osevent` untouched): an ACCEPTED
  `NV01_EVENT_KERNEL_CALLBACK_EX` (`0x7e`) whose `notifyIndex` is `HOTPLUG | NV01_EVENT_CLIENT_RM`
  records `(hClient, hEvent, hParent)` in the shared model (at most 4); only `notifyIndex` is read
  (`data` is a guest pointer). It is retired by the FREE of the event, its parent or its client, and
  all at once by fn 1 (`SET_GUEST_SYSTEM_INFO`, the first RPC of every GSP boot — a re-init, §40 Tier
  B). With no live registration a resize only changes the monitor, and the next probe reads it.
- Tests (CI-compiled): `kf-disp` `a_window_becomes_a_monitor_fitted_under_the_connector`,
  `a_new_monitor_is_reported_once_by_the_internal_hotplug_state`,
  `hotplug_registrations_retire_with_their_objects`; `kf-abi`
  `the_hotplug_list_post_carries_its_data_at_the_flexible_arrays_offset`,
  `a_list_post_refuses_an_index_at_or_above_maxcount`; `kf-rm`
  `the_hotplug_event_registers_and_retires`; `kf-broker` (SURFACE clamped and deduplicated).

**Not built:**
- ⊘ **SUPERSEDED 2026-10-03 by `OWNER_RULINGS.md` §L — never to be built:** "Zero-copy export of the
  guest's surface with NVIDIA's modifier (later, as before)." A shared RM object would let the guest
  change the bytes under the compositor, and no guest surface or store slice is ever exported. Its
  replacement is the GPU-copy rung (§8.11): the finished frame copied GPU→GPU into a VRAM object
  kayfabe allocated itself, exported as a block-linear dma-buf.
- The broker's clipboard.

### 8.7 Deviations from the reviewed design

1. **The latest commit is retained.** The reviewed design returned a frame on its RELEASE. The test
   backend (and the X11 path) release a frame at once, so nothing was left to replay after a broker
   restart. Now the RELEASE of the **latest** commit returns the credit but keeps the frame held until a
   newer one is committed (nvkvm-pv's retained frame); on disconnect it moves back to broker-ready, and
   the next connection replays it. It **yields** to a live frame that would otherwise wait on the cap of
   2, so retention never holds back a new frame.
2. **Reclaim is eager and needs ONE newer commit, not two.** Under the cap of 2, a stuck frame plus the
   frame on screen fill the held set, so a second newer commit can never happen; the rule as reviewed
   could not fire. A reclaim is attempted whenever a held frame becomes eligible, not only when a frame
   waits.
3. **The deadline for reclaim is 1 s after the frame's commit** (or its ATTACH, for one whose COMMIT was
   superseded), as reviewed; the "two newer commits" half is (2).
4. **`kf3_realize` gains `display_broker`** (the design kept `kf3_realize` unchanged): the worker must
   know at realize whether to back frames with memfds, before `kf3_broker_start` runs.
5. ⊘ Superseded 2026-10-03: the euid now comes from `geteuid` (`kf_linux_raw::effective_uid`, one
   audited block, ratchet 87 → 89 with a test-only `listen`), read at every connect attempt; a
   `/proc` read fails inside a `-run-with chroot=` without `/proc`. As first built: **The effective uid
   comes from `/proc/self/status`** (safe code) rather than `geteuid` (one more unsafe relaxation).
6. **KF3 ABI 12**, not 11: 3c added `kf3_display_ui_info` and the broker's `SURFACE` event kind on
   top of 3a's ABI 11 (the device refuses an archive of either other number).
7. **The loopback's SIGSTOP case grades "never blocks"** (every relay call returned within 50 ms, frames
   waited or were reclaimed) rather than owed/dropped counts: pacing commits at most ~10 frames/s to a
   stopped broker, which does not fill a socket buffer in 5 s. The owed/dropped paths are covered by the
   scripted-link tests (invariant 7).
8. **The rung-1b loopback case asks the real broker's `QUERY_FORMAT` answers** for (XR24, MOD_INVALID)
   directly: the test backend always sets `CAP_MODIFIERS`, so the relay never takes 1b against it. The
   relay's 1b path is `rung_1b_is_the_implicit_modifier_after_an_explicit_yes` (scripted link).
9. **The GSP re-init that retires the hotplug registrations is fn 1** (`SET_GUEST_SYSTEM_INFO`, the
   first RPC of every GSP boot), observed by the display link at the front of the chain — the
   reviewed design named "every GSP re-init" without a mechanism.
10. **The hotplug post goes to the newest live registration** (NVKMS makes one per GPU it drives;
    the bound is 4).
11. **The first connect attempt runs from the timer, not inside `kf3_broker_start`** (2026-10-03, the
    review): realize precedes QEMU's privilege drop.
12. **A superseded owed frame yields its slot** (2026-10-03, the review), beside the retained frame of
    (1); without it the display could freeze for good (§8.3).
13. ⊘ Corrected again 2026-10-03 (the re-review): **the first refused broker backing withdraws the
    whole ring from the broker** (`FrameRing::withdraw_all`, permanent for the device's life). The
    design's "the broker is shown nothing" needed state the design did not have. As first built
    (superseded the same day): a per-slot bit, cleared only for a slot refilled with console-only
    memory, which left every other slot offered (§8.2).

### 8.8 Local runs (dev host, 2026-10-03; no GPU)

★ **Added 2026-10-03 — the GPU-copy rung (§8.11), run locally (same rules; no GPU):**
- `cargo test -p kf-broker`: lib 22, `proto_mirror.rs` 4, `relay_machine.rs` 35 — all passed. Five
  bite-mutations of `conn.rs` each fail a rung-0 test: the explicit yes weakened to "not no"; the
  detector never run; the `EV_DEVICE` check dropped; a stale VRAM backing accepted; the extent check
  dropped.
- `cargo test -p kf-abi -- drmnv the_display_slot memory_allocation_params`: 7 passed.
- `cargo test -p kf-linux-raw --lib -- drm`: 7 passed, the fence check `UDMABUF-GATE: RAN` on a real
  udmabuf (kernel 7.0: idle, and a memfd refused).
- `cargo test -p kf-disp -- vramslot` 6 and `--test bl_pack_kernel` 2 passed; three mutations of
  `kf_bl_pack.cu` (a read past the row, a swapped GOB bit, the GOB column width) each fail the
  host-run kernel test.
- `cargo test -p kf-cuda --lib -- display` 5, `cargo test -p kf-host -- display_slot` 1 passed.
- `tools/drivermatrix/drmnv.py` over the 29 tags (headers fetched from GitHub at each tag): the
  interval of §8.11.
- kf3.c `-fsyntax-only -Werror` with QEMU's warning flags against `/workspace/bench/qemu-build`
  (its `config-host.h` now says `#undef CONFIG_PIXMAN`, so the check ran with a copy defining it —
  HEAD's kf3.c needs pixman too).
- kf-qemu is not built locally (owner rule F): CI compiles it.

⊘⊘⊘ **Added 2026-10-03 — the third review's fixes, re-run locally (same rules):**
- `cargo test -p kf-broker`: `relay_machine.rs` 27 passed (25 + the two withdrawal tests of §8's
  top correction).
- Both new tests FAIL against the previous `conn.rs` (`80271bec`, swapped in from a copy and
  restored): the owed-frame test at its first `active()` assertion (the connection was dropped), the
  between-sends test at its `types()` assertion.

⊘⊘ **Added 2026-10-03 — the re-review fixes, re-run locally (same rules):**
- `cargo test -p kf-broker`: 43 passed (lib 15, `proto_mirror.rs` 3, `relay_machine.rs` 25), with
  the withdrawal tests replaced by `a_withdrawn_ring_offers_the_broker_no_slot_again` and
  `a_withdrawn_ring_sends_the_broker_nothing_not_even_a_replay`.
- Bite-mutations, each failing its test: `fits()` without its `broker_backed` check (the replay
  sends the withdrawn slot: `the withdrawn slot 0 was replayed`); `withdraw_all` not setting the
  flag; `publish` ignoring `broker_backed`.
- `display.rs` `broker_backing` and its kf-qemu test, copied verbatim into a throwaway crate over
  `kf-broker` (kf-qemu itself is not built locally; a stand-in `ConsoleShare` over the same ring):
  passes, and fails with `withdraw_all` removed (the first fix's behaviour: the kept slot is still
  offered) and with the seat asked after a refusal. CI runs the real one.

⊘ **Added 2026-10-03 — the review fixes, re-run locally (same rules):**
- `cargo test -p kf-linux-raw --lib`: 131 passed. Under `ulimit -n 1024` the unfixed
  `a_full_backlog_is_eagain_and_never_in_progress` FAILED (`socket(AF_UNIX)` errno 24, `EMFILE`) and
  the fixed one passes.
- `cargo test -p kf-broker`: 43 passed (lib 15, `proto_mirror.rs` 3, `relay_machine.rs` 25).
- Each new test was shown to fail without its fix: a bite-mutation per fix (no superseded mark → the
  freeze test fails; `start` connecting at once → the euid test fails; the euid ignored → the policy
  test fails; `publish` ignoring the withdraw bit → the ring test fails; an unconditional `REFUSED`
  line → the rate test fails; the old `display-broker-uid` parse → the property test fails; the ring
  forgetting the console's front → the interleaving test fails on the console's marks). The squatter
  who owns the socket's directory was run against the UNFIXED relay in a throwaway worktree: it
  connected (`connected: 1, packets: 1`).
- Against nvkvm-pv's unchanged broker (`368d2db`, `--backend test`), as root: **7 passed** (the six
  above plus `a_squatter_who_owns_the_sockets_directory_is_refused`).
- kf3.c compiled `-fsyntax-only -Werror` with QEMU's own warning flags against the configured QEMU
  10.2.4 bench tree (`/workspace/bench/qemu-build`, pixman on) — a check shown to fail on a misspelled
  QEMU call. It is not a link or a run.
- kf-qemu is not built locally (owner rule F); CI compiles it and runs its tests.

Every cargo run under the shared flock, `-j2`, a throwaway target dir (owner rule F):

- `cargo test -p kf-linux-raw --lib`: 130 passed, including `UDMABUF-GATE: RAN` for both udmabuf tests
  (a udmabuf over a sealed memfd is a dma-buf, `fstatfs` = `DMA_BUF_MAGIC`, its id survives a dup, it
  maps the memfd's pages; an unsealed memfd is refused `EINVAL`).
- `cargo test -p kf-broker`: 34 passed (wire 3, slots 7, link 1, `proto_mirror.rs` 3, `relay_machine.rs` 20).
- `cargo test -p kf-cuda`: 47 passed (the registration itself needs a GPU).
- Against nvkvm-pv's **unchanged** broker built from `368d2db` (`git archive` into scratch;
  `make nvkvm-display-broker`, `--backend test`):
  `KF_BROKER_BIN=… cargo test -p kf-broker --test broker_loopback -- --ignored --test-threads=1`: **6
  passed** — a real 640x480 udmabuf attached LINEAR (`TEST attach: id=<our dma-buf ino> 640x480
  stride=2560 offset=0 XR24 mod=0x0000000000000000`), RELEASE carrying our id, FRAME pacing, scripted
  `f 1`/`p 1`/`k 30 1`/`b 272 1`/`a 100 200`/`w 1 0` decoded; the broker's QUERY_FORMAT answers (XR24
  INVALID yes, XR24 LINEAR yes, AB24 LINEAR no); F_SHM frames and an F_SHM replay to a broker restarted
  after `kill -9`; a rejected ATTACH (AB24 as shared memory) reclaimed after 1 s while good frames kept
  flowing; SIGSTOP for 5 s never blocking the relay, RELEASEs resuming after SIGCONT; a squatter
  listening as uid 65534 (python3) refused before any packet was read.
- The broker's own `make check`: 81 of 82; the failing case is its `SO_PEERCRED` one, which runs the
  test client as uid 65534 out of a build tree under a mode-0700 directory it cannot traverse here
  (environmental, not a broker or relay finding).
- GitHub CI (`stable`, `aarch64`) green at `78051779` (run 37126498993, 3a/3b core), `df35472f` (run
  37127120277, the kf3 glue), `5af2c04b` (run 37127763760, 3d) and `4ed1a002` (run 37128902174, 3c,
  KF3 ABI 12): it builds the workspace, runs kf-qemu's tests including `wire_mirror`, Clippy and the
  gates — including `UDMABUF-gate reached-count` (the runner's `/dev/udmabuf` is not permitted,
  so both gated tests print `SKIPPED` and are counted). CI does **not** compile kf3.c: every QEMU call in
  it was read against v10.2.4's headers.

### 8.9 Pending box tests (exact commands; renting needs the owner's approval)

Every result cites the kf3 binary's revision (§5.2). On the box, with the branch head checked out:

⊘⊘ Corrected again 2026-10-03 (the re-review): item 5 now covers the vast "Ubuntu Desktop (VM)"
template, whose own desktop provisioning stops and disables. It says how to start that desktop again
on the new driver and how the broker reaches the session under the peer policy of §8.5. Provision
from this branch at `3d4e8dac` or later, which carries master's `provision_host_driver.sh` fix
(`4b077201`); before it, provisioning failed on that template.

⊘ Corrected 2026-10-03 (the review): item 1 said ABI 11; the branch is **KF3 ABI 12** and the device
refuses archives of 10 and 11. Item 1 also gains GTK/VNC cases without a broker, item 10's VNC resize
without a broker now grades the OPPOSITE way, and items 11–12 are new.

1. **Build and the unchanged bar.** `scripts/bench/build_kf3.sh` (QEMU 10.2.4, ABI 12 — the device
   refuses archives of 10 and 11), then the merge bar with `display-broker` unset:
   `scripts/bench/v3_gates.sh` and `KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh <tag> 180` (30/30),
   and the display lane M1/M2 (`scripts/bench/display/`) — pixel-exact as before. Also with
   `display-broker` unset: a `-display vnc=…` boot where a VNC client asks `SetDesktopSize`, and (where
   GTK is built) a `-display gtk` boot — the guest's mode must stay what it was (no `resize … hotplug
   queued` line: without a broker there is no `ui_info` hook).
2. **The broker.** Its X11 and Wayland backends are compiled in only when their libraries are
   found (`pkg-config`; otherwise only `--backend test` works): `apt-get install -y pkg-config
   libwayland-dev wayland-protocols libxcb1-dev libxcb-dri3-dev libxcb-present-dev libxcb-render0-dev
   libxcb-xinput-dev libgbm-dev`. Then `git -C <nvkvm-pv> archive 368d2db src/broker src/common | tar
   -x -C /opt/nvkvm-broker && make -C /opt/nvkvm-broker/src/broker nvkvm-display-broker`, and
   `B=/opt/nvkvm-broker/src/broker/nvkvm-display-broker`. Then the relay's own loopback on the box:
   `KF_BROKER_BIN=$B cargo test -p kf-broker --test broker_loopback -- --ignored --test-threads=1`
   (7/7, `UDMABUF-GATE: RAN`; ⊘ corrected 2026-10-03: said 6/6 before the second squatter case).
3. **Registration on 580.159.04.** Boot with `-display none -device
   kf3-gpu,id=kf0,display=on,display-broker=/run/kf3/display.sock -device virtio-keyboard-pci -device
   virtio-tablet-pci,display=kf0,head=0` and the broker on `--backend test`
   (`nvkvm-display-broker --socket /run/kf3/display.sock --backend test --persist`). Grade: no
   "BROKER frame backing is REFUSED" line; the broker logs `TEST attach: … XR24 mod=0x0` at the guest's
   mode; `screendump … kf0` still pixel-exact; the FNV of the udmabuf's pages equals the console frame of
   the same serial (`KF3_DISPLAY_TRACE=1` prints the copy's FNV).
4. **Input reaches the guest.** On the broker's stdin: `f 1`, `p 1`, then `k <code> 1`/`k <code> 0` for a
   string typed into a guest terminal, `b 272 1`/`b 272 0`, `a 100 200`, `w 1 0`; grade by a file the
   guest wrote.
5. **Frames on Wayland and X11 brokers.** Headless weston (`weston --backend=headless`) or sway with the
   broker on `--backend wayland`; an Xvfb/Xorg session with `--backend x11` (and `--present-mode=shm`).
   Grade: frames presented; the rung chosen, as logged; a compositor screenshot equals the guest's
   screendump; `REUSE-IN-FLIGHT` stays 0 in the broker log.

   ⊘ Added 2026-10-03 (the re-review). **On the vast "Ubuntu Desktop (VM)" template**
   (`vms_enabled=true`; sddm, then Xorg, then KDE on the GPU), the host's own desktop is the X11
   compositor. Run this item after the others, because they want no host desktop. Nothing here has
   run on that template yet.
   - **a. Start the host desktop again, on the new driver.** `provision_host_driver.sh` stops AND
     disables the display manager for the driver swap, because Xorg holds `nvidia_drm`. It records
     the name in `/root/prov/host_dm_stopped`; a missing file means the box ran no display manager,
     so use weston or Xvfb as above. The name recorded is usually the alias `display-manager`, and
     disabling the unit can remove that alias, so start the real unit (start only, never enable;
     a reboot then comes back without it):
     ```
     dm=$(sort -u /root/prov/host_dm_stopped | head -1)
     [ "$dm" = display-manager ] && dm=$(basename "$(cat /etc/X11/default-display-manager)")   # /usr/bin/sddm -> sddm
     systemctl start "$dm"
     for i in $(seq 90); do pgrep -x Xorg >/dev/null && break; sleep 1; done
     ```
     Grade the desktop on 580.159.04: `nvidia-smi --query-gpu=driver_version --format=csv,noheader`
     prints `580.159.04`, `grep -E 'NVIDIA GLX Module +580\.159\.04' /var/log/Xorg.0.log` matches,
     and `nvidia-smi` lists `Xorg` among its processes. Run `systemctl stop "$dm"` again before any
     later merge-bar run on the same box.
   - **b. Find the session.** The X display and its cookie are in the environment of a process in
     the session. With a user logged in (KDE autologin), use the user's `plasmashell`:
     ```
     P=$(pgrep -o -x plasmashell)
     sv() { tr '\0' '\n' < /proc/$P/environ | sed -n "s/^$1=//p"; }
     XD=$(sv DISPLAY); XA=$(sv XAUTHORITY); U=$(stat -c %U /proc/$P); UU=$(stat -c %u /proc/$P)
     ```
     With only the sddm greeter up (no `plasmashell`), use its X server instead and way (ii) below:
     `XD=:0; XA=$(ps -o args= -C Xorg | grep -o -- '-auth [^ ]*' | cut -d' ' -f2)`. A Wayland
     session gives `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` from the same environment instead, and the
     broker then runs with `--backend wayland`. Give these variables to the broker's command only. Never
     export `DISPLAY` into the shell that runs `boot_capture.sh`.
   - **c. How the broker reaches the session, and how kf3 admits it.** The bench runs QEMU as root.
     The relay admits uid 0, QEMU's euid, and `display-broker-uid` (§8.5). The broker's own default
     allow-list admits uid 0 and the user who started it, so a root QEMU can always connect to it.
     - **(i) The deployment shape: the broker as the session's user.** The socket goes in that
       user's runtime directory, and kf3 must name the user's uid:
       ```
       install -d -o "$U" -m 0700 /run/user/$UU/nvkvm
       nohup runuser -u "$U" -- env DISPLAY="$XD" XAUTHORITY="$XA" $B \
         --socket /run/user/$UU/nvkvm/display.sock --backend x11 --persist \
         > /workspace/bench/brk_x11.log 2>&1 &
       KF3_DEV_EXTRA=display=on,display-broker=/run/user/$UU/nvkvm/display.sock,display-broker-uid=$UU \
         bash scripts/bench/boot_capture.sh brk-x11 -- -vga none -display none \
         -device virtio-keyboard-pci -device virtio-tablet-pci,display=kf0,head=0
       ```
       Expect the QEMU log's `relay to … (brokers accepted: uid 0, QEMU's effective uid at each
       connect, display-broker-uid <UU>)` line and 0 `REFUSED the listener` lines. A second boot
       without `display-broker-uid` must log `REFUSED the listener … uid <UU>` and keep retrying,
       with the VM unaffected: item 11's case on a real session.
     - **(ii) The broker as root, with the session's display:** `nohup env DISPLAY="$XD"
       XAUTHORITY="$XA" $B --socket /run/kf3/display.sock --backend x11 --persist >
       /workspace/bench/brk_x11.log 2>&1 &`, and `display-broker=/run/kf3/display.sock` with no
       `display-broker-uid`, since uid 0 is always admitted. The broker's `running as root and
       --drop-user was not given` warning is expected here.
   - **d. Grade.** As at the top of this item, plus `--present-mode=shm` as a second run. For the
     screenshot, as root: `apt-get install -y x11-apps imagemagick`, then
     `env DISPLAY="$XD" XAUTHORITY="$XA" xwd -root -silent | convert xwd:- /workspace/bench/brk_x11.png`.
     Crop the broker window's area (`xwininfo -root -tree` gives its geometry; the title is `nvkvm`)
     and compare it with the guest's `screendump` of the same moment. Items 6 and 7 run in this same
     session. `$B` is the broker built in item 2.
6. **X11 with scaling active:** the broker window resized away from the guest's mode (XRender path);
   grade: the frame rate is not capped at 10 fps (`broker[sent=…]` over 10 s ≈ the guest's flip rate).
7. **Pointer visible (3d):** in the Wayland and X11 broker runs of (5), move the guest pointer
   (`a <x> <y>` on the test backend's stdin, or the real compositor's pointer) and grade a
   compositor screenshot and the console's `screendump` for the pointer image at the point; the
   same on the console alone with `display-broker` unset (the cursor layer applies there too).
8. **Never holds the guest:** `kill -STOP <broker>` for 10 s while `kfdisp_probe` flips: ~60 Hz, 0 flip
   timeouts; `kill -CONT`; then `kill -9` and a restart: reconnect within the backoff and the last frame
   replayed (broker log `TEST attach`), VM unaffected. Added 2026-10-03: repeat with a STOP of 3 minutes
   (long enough for the socket to fill at ~2 frames/s; grade `broker[dropped=…]` or
   `broker[uncommitted=…]` > 0, then after `kill -CONT` `sent` keeps rising — the freeze of §8.3).
9. **Absent at boot:** boot with no broker; the VM boots, the console and VNC work; start the broker
   later: it attaches.
10. **Resize (3c):** with the broker on `--backend test` and a guest desktop up, resize the broker's
    window (`nvkvm-display-broker … --resolution auto`; the test backend announces a new SURFACE on
    `resize`) to 1600x900; one second later the QEMU log shows `resize 1600x900 -> monitor 1600x900 …
    hotplug queued` and `hotplug posted for display 0x100`; in the guest `modetest -c` (or `xrandr`)
    lists 1600x900 as preferred and 1920x1080; repeat at 2560x1440 (fitted to 1976x1110 under 165 MHz).
    Then `rmmod nvidia_drm nvidia_modeset` and reload: the log shows the registration retired (FREE) and
    re-registered, and no post ever names a dead pair (no `Bad sequence number` in the guest log). Compare
    the compositor's behaviour with bare metal under a forced EDID (`nvidia-settings`
    `CustomEDID`/`drm.edid_firmware`). ⊘ Corrected 2026-10-03: a VNC client's resize with
    `display-broker` unset must now NOT re-mode the guest (item 1); with it set, it does, as above.
11. **Peer policy under `-run-with user=`** (2026-10-03): start QEMU as root with
    `-run-with user=<u>` and the broker running as `<u>`, no `display-broker-uid`: the broker is
    admitted at the first attempt (no `REFUSED the listener` line). Then the broker as another uid:
    refused by name, retried; with `display-broker-uid=<that uid>`: admitted. `display-broker-uid=-2`
    refuses realize by name.
12. **No tablet** (2026-10-03): boot with the broker and without `virtio-tablet-pci`: exactly one
    `NO absolute pointing device exists` warning at the first connect; with the tablet listed after
    kf3 on the command line, none.

### 8.10 Owner questions (each implemented with the stated default; the owner confirms later)

1. Peer policy: ⊘ corrected 2026-10-03 — {0, QEMU's euid read at each connect} + `display-broker-uid`;
   the socket directory's owner is no longer trusted (§8.5). As first built: {0, QEMU euid, the socket
   directory's owner} + `display-broker-uid`.
2. Broker distribution: installed separately, pinned to nvkvm-pv `368d2db` — as above.
3. Clipboard: left out of step 3.
4. Native resizes above ~1920×1200@60 (DVI single-link): not in this step.
5. The 3c hotplug registration beside A.11's `osevent` rule: built as the separate, narrow seat the
   brief specified (§8.6); `osevent` and its pinned refusal of `0x7e` are untouched. Owner to confirm.
6. Reuse without RELEASE: the narrowed rule of §8.3, with the deviations §8.7 (1)-(2).
7. **(2026-10-03, §8.11) The GPU-copy rung's host requirements and VRAM.** It needs nvidia-drm
   `modeset=1` and QEMU access to the GPU's render node (the `render` group or the seat's ACL — a third
   device class after `/dev/nvidia*` and `/dev/udmabuf`), and holds 50 MiB of host VRAM per head at
   1080p, up to 230 MiB after a 4K mode, for the VM's life. Implemented default:
   `display-broker-vram=auto` (allocate at the first explicit yes for the block-linear pair); `on`
   fails realize on a refusal; `off` never allocates. Owner to confirm.
8. ⊘ **CORRECTED the same day (`22a3e10a`): (b) below is no longer kf-broker's proposal — nvkvm-pv's
   broker defines `EV_DEVICE` itself** (branch `broker-cursor-gpucopy`, its `nvkvm_broker_proto.h`):
   type 17 behind `CAP_DEVICE` (1 << 11), `x` = `DEVICE_F_KNOWN | DEVICE_F_RENDER` flags (0 = does not
   know), `y` = 0, `w0`:`w1` = major:minor, sent once after the handshake's priming FRAME and again
   whenever the display server reports another device; the same revision adds `CMD_CURSOR` (7, behind
   `CAP_CURSOR` = 1 << 10). kf-broker follows that header (§8.11). (a) needs no new type.
   **(2026-10-03, §8.11) The broker changes in nvkvm-pv**, append-only in protocol v2: (a) the X11
   backend sends the unsolicited `EV_FORMAT x=0` on a refused DRI3 import, as Wayland does; (b)
   `EV_DEVICE` (type 17 as kf-broker implements it, `x:y` = the compositor's DRM device — ⊘ superseded
   2026-10-03 by the correction at the top of this item: `x` = flags, `w0`:`w1` = the device); (c)
   optionally, an idle fence on X11 `PresentPixmap` and the XRender path's RELEASE after its composite,
   so RELEASE means GPU-idle. The coordinator took (a) and (b) as the owner's default; until the broker
   sends them, kf3 runs the acknowledgement detector with back-off, the LRU fill and the fence check.

### 8.11 The GPU-copy rung (`OWNER_RULINGS.md` §L) — rung 0 for a compositor on the same GPU

**STATUS: BUILT IN CODE, GPU-FREE-TESTED — 2026-10-03 (branch `v3-broker`, `bd37049f` + `b3ec2d21`;
`EV_DEVICE` re-read by nvkvm-pv's header at `22a3e10a`).
Nothing has run on a GPU or a box.** The design was read from source and revised the same day after
an adversarial review; the revision is what is built. Every claim below that needs hardware is
listed in *What has not run* and the box experiments E0-E6.

**The rung in one paragraph.** kf3 allocates its OWN VRAM frame objects ("slots") — never guest
memory, never a slice of the store — from a separate RM client, wraps each once as a dma-buf through
the same GPU's DRM render node, and imports the same object into the display CUDA context. Per frame
the compose kernel writes the staging frame as before (windows, blending, scaling, and the cursor —
composed in grab mode per `OWNER_RULINGS.md` §O; the hover-mode host cursor is the cursor message's
work, not this rung's), and a pack kernel writes it into a free slot in NVIDIA block-linear layout.
The relay ATTACHes that dma-buf with `DRM_FORMAT_MOD_NVIDIA_BLOCK_LINEAR_2D` read from
`GET_DEV_INFO` (`0x0300000000606014` on Turing … GB20x, 32 bpp, 16-GOB blocks). No byte crosses
PCIe; the guest's release semaphores still follow kf3's own copy (§4.6), so there is no new barrier.

**Built, by crate** (each GPU-free part tested locally and in CI):

| piece | where | test |
|---|---|---|
| the nvidia-drm/NVKMS private ABI as byte encoders; the modifier builder; **the ABI gate** — the rung is offered only at a host driver tag where `tools/drivermatrix/drmnv.py` compiled that tag's own headers and every value equals the transcription | `kf-abi/src/drmnv.rs`, `traces/driver_matrix/drmnv.tsv` | `the_rung_is_offered_only_at_tags_measured_equal_to_the_transcription` (incl. a mutated row refused) |
| `NV_MEMORY_ALLOCATION_PARAMS` `flags` (+8) and `attr2` (+28); the three candidate slot attribute sets S0/S1/S2 as setup data | `kf-abi/src/submit.rs` | `the_display_slot_attribute_sets_are_nvos32_fields` |
| render-node discovery by PCI address (sysfs), an open that checks the file IS that char device (`st_rdev`), the two nvidia-drm ioctls through `CharDevice::ioctl`, allowlisted, the import's size field checked against its buffer | `kf-linux-raw/src/drm.rs` | fixture sysfs + a `/dev/null` symlink as the node |
| `PRIME_HANDLE_TO_FD`, `GEM_CLOSE`, `DMA_BUF_IOCTL_EXPORT_SYNC_FILE` + `poll(0)` (ratchet 89 → 93) | `kf-linux-raw/src/drm_unsafe.rs` | the fence check on a real udmabuf (UDMABUF-gated: RAN locally on kernel 7.0, SKIPPED in CI) |
| slot geometry; `SLOT_MAX` = 36 MiB derived by exhaustive search; the GOB byte order as setup data; `bl_chunk_origin` = the exact inverse of `bl_offset` (h 0..5, all 32 chunks); `pack_reference` | `kf-disp/src/vramslot.rs` | six tests |
| the pack kernel: one warp per GOB, one thread per 16 bytes, the GOB bits as parameters; PTX by clang's NVPTX back-end (`make_pack_ptx.sh`, the source's FNV in the PTX header) | `cuda/display/kf_bl_pack.{cu,ptx}` | `kf-disp/tests/bl_pack_kernel.rs` runs the SAME source on the host against `pack_reference` (three bite-mutations each fail it); kf-cuda pins the PTX parameter list and the source FNV |
| slot import / clear / `compose_to_slot` (bounded by `BlPack::check`) / `compose_to_host` / `compose_signal` / `selftest_bl_pack` | `kf-cuda/src/display.rs` | `a_pack_launch_is_bounded_before_it_is_queued` |
| `alloc_display_slot` / `export_display_slot`: only a slot this session recorded, at its size, under a 64 MiB cap, is exported | `kf-host/src/lib.rs` | `only_a_recorded_display_slot_of_a_sane_size_is_exported` |
| two backings per slot, per-frame freshness, **withdrawal per kind**, the LRU fill with a fence exclusion mask, identities across kinds | `kf-broker/src/slots.rs` | five ring tests, incl. the cap argument with mixed backings |
| `Rung::Native`, the explicit-yes rule, the acknowledgement detector with timed back-off, `EV_DEVICE`, the `want_vram` signal | `kf-broker/src/conn.rs` | eight `relay_machine.rs` tests; five bite-mutations each fail one |
| `VramMode`, `plan()` (pack and/or D2H), `Provisioning` | `kf-broker/src/gpucopy.rs` | three tests |
| the realize probe, slot making with collision retry, the provisioning thread; the worker's adoption, fence check, pack, two demand signals | `kf-qemu/src/gpucopy.rs`, `display.rs` | CI-compiled; the decisions are kf-broker's tested functions |
| `display-broker-vram=auto\|on\|off`, KF3 ABI 13 (⊘ 12 since the merge with master) | `kf3.c`, `kf3.h`, `ffi_unsafe.rs` | `wire_mirror.rs` checks the C encoding against `VramMode` |

**The ABI interval, measured 2026-10-03** (`drmnv.py` over the 29 tags of `tools/drivermatrix/tags.txt`,
gcc over each tag's own headers fetched from the open-gpu-kernel-modules tag): equal to the
transcription at **575.51.02 … 615.71.09** (18 tags; the header is `nv_drm_common_ioctl.h` from
590.48.01, the layout unchanged). Refused at **535.309.01 … 570.148.08**: there
`drm_nvidia_get_dev_info_params` has no `mig_device`, so every later field sits 4 bytes lower — what
the DRM core would have zero-filled into a silent misread. The probe also requires nvidia-drm's own
`/sys/module/nvidia_drm/version` to equal the RM's version.

**How a frame chooses its copies** (`kf_broker::gpucopy::plan`, per frame on the worker):
- **pack** when an active broker wants VRAM (`FrameRing::want_vram`, set by the relay), the VRAM kind
  is not withdrawn, and a free VRAM slot can take the frame (provisioned that large, and its dma-buf's
  fences signalled);
- **D2H** when the console asked, when an active broker must be fed through host memory, or when
  nobody asked (the 4 Hz copy that keeps a screendump recent, as before). ★ Never only because the
  broker is active while it takes the GPU copy: then no byte goes to the CPU.
- ⊘ **Two demand signals** (the design review's correction): broker activity keeps the 33 ms refresh
  (a front-buffer-rendering guest stays at 30 Hz on the broker) but asks for a host copy only in the
  case above.
- A frame no one can be shown is not made; the flips behind it still complete (`done = n`).

**When rung 0 goes** (`Relay::choose`): an EXPLICIT yes to `QUERY_FORMAT(XR24, the modifier)`, asked
right after HELLO; `CAP_MODIFIERS` and `CAP_RELEASE`; the compositor not on another GPU (`EV_DEVICE`);
not backing off; the slot's VRAM backing fresh, the block-linear extent inside the object, the stride
inside the broker's bounds (`4w ≤ stride ≤ 8w + 4096`, `stride·h ≤ extent`). Otherwise the host rungs
of §8.2, unchanged, each requiring its own backing fresh and not withdrawn.

**Learning that the import failed.**
- Wayland reports a failed probe as `EV_FORMAT x=0`: a later "no" wins, rung 0 stops, and the frames
  attached under it are reclaimed.
- X11 reports nothing today. The **acknowledgement detector**: 3 native commits and ≥ 1 s with no
  RELEASE naming a native frame back the rung off for 5 s, doubling to 60 s; a native RELEASE
  acknowledges it and clears the back-off. It is a timed back-off, never a permanent "no": on X11 with
  the NVIDIA DDX the host rungs may be a black window. The cap of 2 held frames and the 1 s reclaim
  pace an unimported stream, so the trip comes after ~1-2 s (`the_detector_backs_off_retries_and_
  clears_on_a_release`).
- Once nvkvm-pv's X11 backend sends `EV_FORMAT x=0` on a refused DRI3 import (the coordinator's
  default for the owner, same protocol revision as the cursor message), the existing "later no" path
  handles it — no relay change.
- ⊘ **CORRECTED the same day (`22a3e10a`, the coordinator relaying nvkvm-pv's header — the broker
  owns the protocol): the next bullet's encoding and its "proposal" were wrong.** `EV_DEVICE` is
  nvkvm-pv's: type 17 behind `CAP_DEVICE` (1 << 11); `x` = `DEVICE_F_KNOWN` (1) | `DEVICE_F_RENDER`
  (2), 0 = the broker does not know; `y` = 0; `w0`:`w1` = major:minor; sent once after the
  handshake's priming FRAME, and again whenever the display server reports another device. The relay
  now reads it so: another device's RENDER node ⇒ no rung 0 on that connection; this GPU's primary or
  render node ⇒ allowed; `x` = 0 ⇒ the yes and the detector decide; KNOWN without RENDER (an
  unresolved node, "compare with care") decides only when it IS this GPU's node; a broker advertising
  `CAP_DEVICE` gets no rung-0 frame before its `EV_DEVICE`; a later, different device moves the
  decision (`a_compositor_on_another_gpu_gets_no_gpu_copy`, three bite-mutations each fail it). The
  values (and `CMD_CURSOR` = 7, `CAP_CURSOR`, the cursor record, its ops and bounds) are in `wire.rs`,
  AHEAD of the vendored `368d2db` header: `proto_mirror.rs` asserts each is absent from it (the day
  it lands, the test forces it into the mirrored map) and, with `KF_BROKER_PROTO_NEXT` naming the
  newer header, equal to it both ways, the cursor record's layout compiled from it (run locally
  against the nvkvm-pv worktree: `PROTO-NEXT: RAN`; a mutated `CAP_DEVICE` fails it). The hover-mode
  host cursor that would send `CMD_CURSOR` is NOT built: frames keep composing the cursor.
  (⊘ SUPERSEDED 2026-10-03 (night): it is built, §8.12.)
- ⊘ **SUPERSEDED 2026-10-03 (`22a3e10a`) by the correction directly above — kept as first written,
  do not build from it.** Its encoding (`x:y` = the device) and "a proposal" are both wrong: the
  broker owns the protocol, and nvkvm-pv's header (`broker-cursor-gpucopy`, `9cb736f`) defines
  `x` = `DEVICE_F_KNOWN | DEVICE_F_RENDER`, `y` = 0 and `w0`:`w1` = major:minor. What follows is the
  text as first written: **`EV_DEVICE`** (type 17, `x:y` = the compositor's DRM device; AHEAD of the
  vendored header, a proposal to nvkvm-pv's broker): another device ⇒ no rung 0 on that connection;
  this GPU's primary or render node ⇒ allowed; absent or "cannot tell" ⇒ the yes and the detector
  decide.

**Slots and reuse.** Five ring slots as before; each may carry a VRAM backing besides its host one.
`display-broker-vram=auto` (default): realize only probes (render node, `GET_DEV_INFO`, the ABI gate)
and logs whether the rung is possible; the five 10 MiB class-0 slots are provisioned on a
provisioning thread at the first explicit yes (a cross-vendor compositor never says yes and costs no
VRAM). `on`: provisioned and self-tested at realize, a refusal fails realize. `off`: never. A frame
larger than its slot grows every slot once to `SLOT_MAX` (36 MiB) — meanwhile such frames take the
host rungs. Worst case 5 × (10 + 36) = 230 MiB per head, never freed while the device lives; RM is
the only arbiter (`cardbudget` is BAR1-only and slots use no BAR1), the slots come AFTER the store,
and a refused store names display VRAM. ⊘ **RELEASE is not GPU-idle** (X11 presents with no idle
fence; the XRender path RELEASEs right after queuing its composite): the pack takes the eligible free
slot the broker released LONGEST ago, and excludes a slot whose dma-buf still carries an unsignalled
fence (`EXPORT_SYNC_FILE` — lock-free, unlike `poll()` on the dma-buf, which takes `dma_resv_lock`).
Whether NVIDIA compositors attach read fences at all is unverified (E1).

**Security** (§8.5 continues to hold; what is new):
- Only `kf_host::DisplaySlot`s are ever exported: no public constructor, no handle accessor, no API
  that maps one into a VA space; `export_display_slot` refuses a handle the session did not record.
  The slots live in a separate RM client from the store, guest RAM and twins (`OWNER_RULINGS.md` §N).
- The guest cannot write a slot: slots are never mapped into a VA space the guest reaches. ⚠ This
  rests on the EXISTING store-bounded translation — the invariant every other host VRAM object rests
  on (a physical-mode CE hole that reached host VRAM was fixed in `00f62991`); slots add no new path.
- What the compositor gets: one dma-buf per slot, and through `GEM_EXPORT_NVKMS_MEMORY`
  (`DRM_RENDER_ALLOW`) an RM handle to that slot only. It may write it; kf3 never reads a slot after
  the self-test, so nothing flows back.
- The RM export fd is closed after both imports; the dma-buf is `DMA_BUF_MAGIC`-checked; an identity
  collision is retried with a fresh GEM import (≤ 3), the colliding handle closed unsent.
- A hostile guest controls content, geometry (within `MAX_PIXELS`, 8192 a side) and the flip rate —
  never the modifier, stride, kind, slot size or what is exported. Every pack launch re-derives its
  geometry and bounds (`BlPack::check`): the extent inside the slot, the reads inside the staging
  frame.
- New capability: one render-node fd (GEM wrapping only, no KMS). With `auto` on a host where it
  cannot be opened, the rung is not offered and nothing else changes.

**Version tolerance.** Outside 575.51.02 … 615.71.09 the probe refuses by name (*"GPU-copy rung: the
nvidia-drm ABI … at host driver X"*); a host driver tag not in `tags.txt` is NOT MEASURED and refused.
Re-run `tools/drivermatrix/drmnv.py` when a tag is added.

**What has not run** (box only; every item is a prediction until then):
⊘ **Partly superseded 2026-10-03 (night) — run on box 54032077 at kf3 `18562ba4`/`c38032f3`, results
in §8.12:** CUDA importing S0, S1 and S2 (all three provisioned, self-tested and imported by the X
server, every GPU-copy frame RELEASEd); a compositor on the same GPU importing a kayfabe-owned
OFFSCREEN object (the NVIDIA X server, S1 and S2 included); the pack self-test on a GPU (GA106:
passed). The fence check seeing a real fence and the screendump freshness item below have NOT run.
- CUDA importing any of S0/S1/S2 (only S0, the store's set, has been imported — w755x, RTX 3090,
  580.159.04); the default is S1 (`KF3_VRAM_ATTRS=s0|s1|s2` selects for E1).
- a compositor on the same GPU importing and sampling a kayfabe-owned OFFSCREEN object (nvkvm-pv
  showed only a GBM scanout bo);
- the pack self-test on a GPU; the in-GOB order beyond GA106; the fence check seeing a real fence;
- the screendump freshness on rung 0 with the console idle (no D2H then: a screendump sees the last
  host frame until the next console request; the deferred-update remedy — QEMU 9.2's
  `gfx_update_async` / 11.1's bool return — is NOT built; the 10.2 API is unverified).

**Box experiments** (exact recipe in §8.9 once a box is approved): **E0** host preflight (driver tag in
the interval, `modeset=1`, the render node's mode/group/ACL as QEMU's uid, `GET_DEV_INFO`, the broker's
DRI3 modifier log); **E1** the gate — a slot probe per attribute set S0/S1/S2 against the real broker
(X11, default present mode) on the same GPU: CUDA import, `GEM_IMPORT`, the DDX import, an FNV-matched
image at 1920×1080 and 1366×768, 3 laps through all 5 slots, the export fd closed, whether a fence is
ever unsignalled (a `sw_sync` known-positive); **E1b** the same on Wayland, with the probe pause
measured; **E2** negative controls — a declared block height that differs from the pack's must change
the FNV; another GPU (Wayland `x=0`, X11 detector trip); no `CAP_MODIFIERS` never Native; the detector's
known-positive (a forced bad stride trips, backs off, retries, clears); **E3** guest end to end
(`scanout_d2h` = 0 while the console is unwatched, 30 Hz on a non-compositing desktop, cursor, resize,
broker `kill -9` and `SIGSTOP`); **E4** performance (compose+pack vs compose+D2H, CPU vs F_SHM);
**E5** lifetime and security (5 × class 0 then one growth, the export fd absent from
`/proc/<qemu>/fd`, `GEM_EXPORT_NVKMS_MEMORY` of the received dma-buf names a slot-sized object, a write
through it changes no guest byte); **E6** E1 on Turing, Ada and GB20x.

**Deviations from the reviewed design.**
1. **Fallback B (`GEM_ALLOC_NVKMS_MEMORY`) is not built**: nvidia-drm silently retries a refused
   NO_SCANOUT allocation in sysmem (`ogkm-580: nvidia-drm-gem-nvkms-memory.c:533-538`), and the task
   ruled RM-first only. If E1 refuses path A for every set, the rung stays off and B is a new decision.
2. **The ABI gate is a compile probe of the headers, not DWARF**, because the nvidia-drm/NVKMS private
   headers are outside `dm.py`'s spec set; the gate reads `traces/driver_matrix/drmnv.tsv`, exact tag.
3. **The pack PTX is generated by clang** (no CUDA SDK) and the same source is run on the host —
   the compose kernel stays hand-written.
4. **`withdraw_all` is kept** as "both kinds" for any caller that means both; the worker withdraws one
   kind.

### 8.12 The guest cursor as the host pointer — hover mode (`OWNER_RULINGS.md` §O)

> ⊘⊘⊘ **CORRECTED 2026-10-04 (box 54032077, runs `brkF1`/`brkF2`, kf3 code `34696441`, broker
> `badf2d7`) — the colour inference in the box section below ("a straight-alpha blend programmed over
> premultiplied pixels") is REFUTED by the register.** The head's cursor composition word is
> `0x072ff` = `PREMULT_ALPHA` (K1 255, cursor factor `K1`, viewport factor `NEG_K1_TIMES_SRC`,
> `MODE_BLEND`), under which kayfabe passes the surface colour through unchanged; every pixel the host
> shows differently from the guest X server's own cursor is exactly the guest's colour times its alpha,
> and QEMU's VNC console — kayfabe's same image, never through the broker — matches the host's pixel
> for pixel. So the cursor SURFACE holds X's premultiplied pixels premultiplied once more by the guest's
> NVIDIA DDX, under a premultiplied blend: the guest's own head scans out the same darker edge, and the
> host shows what the head would. The mapping stays. Evidence: `traces/v3_display/broker_20261004/`
> (`cursor_alpha.txt`, `README.md`). The same runs graded deviations 1 and 4's replacements (§8.13,
> §8.14) and the hot spot under the broker's `ceil` scaling (exact at x1.28 and x0.75).

> ⊘⊘ **CORRECTED AGAIN 2026-10-03 (run `brkA4`, kf3 at `18562ba4`) — the derivation below was one
> pixel off on both axes, every time.** On the box it derived `4,2` for the arrow the guest's X server
> holds at `3,1`, and `12,12` for its `11,11` crosshair (the images themselves matched: 254 and 281
> visible pixels on both sides). The injected position reaches the guest through two truncating
> scalings, not one: QEMU's onto the tablet's axis (`v = abs * 0x7fff / range`, QEMU 10.2.4
> `ui/input.c:470-481`) and the guest's back onto the head (libinput's `v * size / 0x8000`): brkA4's
> 48 of 1024 arrived as 47, its 8 of 695 as 7. `hot_from_pointer` now models both; its test carries
> brkA4's case (2026-10-03), and the single-scaling formula fails it with exactly the box's `(4, 2)`.

> ⊘ **CORRECTED 2026-10-03 (box 54032077, run `brkA`, kf3 `6da16d2f` + broker `9cb736f`) — the hot
> spot was wrong as built.** The first SET on hardware said `256x256 hot 0,0`: **NVKMS hard-codes the
> hardware hot spot to 0** (`ogkm-580: src/nvidia-modeset/src/nvkms-evo3.c:6565-6569`, "Hard code the
> cursor hotspot") and moves the image's top-left instead; nvidia-drm has no hot-spot handling at all.
> So the hot spot the guest meant is in no register, and a host cursor at hot 0,0 sits offset from
> where the guest's clicks land by the X cursor's own hot spot (a few pixels for an arrow, about half
> the image for a crosshair or an I-beam). **Now** it is derived: in hover the VMM injected the
> guest's pointer itself, so hot spot = pointer (the injected position, scaled from the broker's
> range onto the head) − the image's top-left (`kf_disp::scanout::hot_from_pointer`). A new image
> takes the derived hot spot at once; it is corrected only from a pointer that stayed put for
> `HOT_SETTLE_MS` = 40 ms (the cursor point and the pointer then belong to the same moment), and
> only by more than one pixel of rounding (`kf_broker::cursor::HotTracker`), so a moving pointer
> never re-sends the image. Tests: `scanout.rs` `the_hot_spot_is_the_pointer_minus_the_images_top_left`,
> `cursor.rs` `the_hot_spot_follows_a_settled_pointer_and_ignores_a_moving_one`, `host_cursor.rs`
> `an_injected_absolute_position_reaches_the_cursor_share`; five bite-mutations (an unsettled
> pointer acted on, no hysteresis, a new image waiting for the settle, the broker's range not
> scaled, the relay not recording the position) each fail one of them. ⚠ The derivation has not
> run on a box yet: the next run grades it against the guest X server's own cursor (XFixes).

**STATUS: BUILT AND RUN ON A BOX — 2026-10-03 (branch `v3-broker`; box 54032077, RTX 3060,
580.159.04, KDE on Xorg). The final run, `brkA5` at kf3 `c38032f3` with nvkvm-pv's broker at
`9cb736f`, graded hover, hide, grab and the hot spot against the guest X server's own cursor; the
runs before it found and fixed two hot-spot defects (the corrections above). What has not run is
listed at the end of this section.** (As first written, 2026-10-03: "BUILT IN CODE, GPU-FREE-TESTED
… nothing of it has run on a GPU.")

**What it does.** The owner's rule (§O): in hover the broker shows the guest's cursor IMAGE as the host
pointer, the guest's position is ignored (the absolute device already makes the host pointer the
guest's), nothing is composed and a move makes no frame; under grab the guest owns the position and the
cursor is composed into the frame as before (3d, §8.6); an XOR cursor is composed in both modes and the
host's is hidden. nvkvm-pv's broker carries the wire (`broker-cursor-gpucopy`, `9cb736f`):
`CMD_CURSOR` (7) behind `CAP_CURSOR` (bit 10) — SET (a memfd the broker `pread`s, premultiplied
`ARGB8888`, at most 256x256), HIDE, SHOW; the broker hides its image under grab by itself and learns
nothing from kf3 about the grab.

**Built, by crate:**

| piece | where | test |
|---|---|---|
| the host's view of a head's cursor: the WHOLE image's store span (never clipped to a frame) and the blend as premultiplied ARGB per pixel — coverage `1 - fd(a)`, colour `c * fs(a)`; every blend NVKMS programs (`nvkms-evo3.c:6646-6700`: opaque, premultiplied, straight, both surface-alpha forms) maps exactly; a wholly transparent image is "no cursor"; refused by name: XOR, non-`A8R8G8B8`, sysmem/block-linear, a hot spot outside the image, an unknown factor, bytes past the context DMA, an additive blend (colour above coverage) | `kf-disp/src/scanout.rs` `plan_host_cursor`, `HostCursorSrc::image` | four tests in `scanout.rs` (expected pixels worked by hand) |
| `CursorShare` (the relay's mode for the worker; the worker's newest cursor for the relay, latest wins, a generation per post, a mutex both sides only `try_lock`), `CursorImage` (bounded at construction; an FNV digest over size, hot spot and pixels), `BrokerCursor::next_op` (SET only for an image or hot spot the broker does not hold, HIDE for none, SHOW when the held image comes back) | `kf-broker/src/cursor.rs` | four unit tests |
| the relay: the grab from `EV_GRAB` and from `F_GRABBED` on EVERY packet (HELLO included); the mode (`Hover` for an ACTIVE `CAP_CURSOR` broker not grabbed, `Grabbed`, `Off` otherwise — and on every disconnect); at most ONE `CMD_CURSOR` per entry, never under grab, never without the bit; the SET's memfd (`kayfabe-cursor`, sealed `SHRINK\|GROW\|SEAL`) made, filled, sent and closed in the call; `EAGAIN` owes the command (the watch asks for writability); status `cursor=… cursor_sets/hides/shows/refused` | `kf-broker/src/conn.rs` (`cursor_sync`, `send_cursor`) | eight tests in `kf-broker/tests/host_cursor.rs` |
| the worker: with a cursor-capable broker (mode not `Off`) it reads the head's cursor once per frame — a GPU copy (`DisplayGpu::read_store`, the path that already reads instance memory and pushbuffers) into a buffer kf owns; the CPU reads that copy, never guest video memory (`THE_CONSTRAINTS.md` §38) — and posts it only when its key changed; in hover it leaves the cursor out of the frame unless the host cannot show it; a mode switch recomposes; in hover a move recomposes only a composed cursor. Counters `host_cursor_reads`, `host_cursor_refused`; a refusal is logged by name, the first four and every 256th | `kf-qemu/src/display.rs` (`host_cursor_want`, `cursor_composed`, `move_recomposes`), `broker.rs` (the seat owns the share) | `display.rs` `hover_leaves_the_cursor_out_of_the_frame_and_its_moves_make_no_frame` (CI-compiled: kf-qemu is not built on the dev host) |

**Bounds.** Size: at most 256x256 (the cursor channel's own sizes are 32..256; `CursorImage` and
`CursorCmd::set` refuse anything else), one memfd of at most 256 KiB per SET, closed before the call
returns. Rate: the worker posts at most one cursor per frame and only on a change; the relay takes the
newest and sends at most one command per entry, a SET only for a new digest — so however many cursors
are posted between two entries the broker gets ONE SET, of the newest
(`posts_between_two_entries_coalesce_to_one_set_of_the_newest`); the broker paces uploads again at
125 Hz (`nvkvm_broker.c:1547-1553`).

> ⊘ **SUPERSEDED IN CODE 2026-10-04 (branch `v3-broker`; run on box 54032077 the same day, runs
> `brkF1`/`brkF2` — §8.13, §8.14 and the correction at the top of this section): deviations 1 and 4
> below.** (1) In hover QEMU's console now gets the guest cursor through `dpy_cursor_define` +
> `dpy_mouse_set` (§8.13), so a VNC client shows it as a real pointer; a `screendump` still has none
> in hover (a defined cursor is not part of the surface). (4) XOR is composed by the kernel's XOR
> blend, visibly, and the host's cursor is hidden as before (§8.14). And the box run's colour
> finding below (the guest's colour times alpha on partially transparent pixels) is now answerable:
> the composition word is logged once per change beside an alpha census of the pixels (§8.14).

**Deviations and limits.**
1. **The console loses the cursor in hover.** Frames are shared by the broker and QEMU's console, so a
   `screendump`/VNC shows no cursor while a `CAP_CURSOR` broker is hovered (it does under grab and
   without a broker). This follows §O ("kf-disp does not compose the cursor into frames in this mode").
2. **A mode switch reaches the frame at the worker's next pass** — at most its 50 ms deadline, 33 ms
   while the broker is active — so a grab shows no cursor, or an ungrab two, for up to one frame.
3. **The image is read every frame in `Hover` and `Grabbed`** (a guest may rewrite a cursor surface in
   place, and only a read sees it): at most 256 KiB per frame, 16 KiB for a 64x64 cursor.
4. **XOR is composed by the existing kernel as "not composable"** — refused by name in the frame as in
   3d (the compose kernel's XOR blend of §O is not built), and the host's cursor is hidden.
5. **The worker → relay wake is the frame publish** that follows every post; a post whose frame copy
   failed is taken at the relay's next entry.

**What has not run (2026-10-03):** `display-max-fps` (`OWNER_RULINGS.md` §M) is not built on any
branch, so it could not be graded (⊘ built since, on `v3-maxfps`, 2026-10-04 — §8.16; still not run
on a box); a Wayland broker (E1b); the console's screendump on rung 0 with
the console idle (stale by design, §8.11); the X11 DDX's handling of an XOR cursor (no guest here
programs one); E5's write-through test and a slot's RM export fd identified among QEMU's
descriptors; a second GPU (E2's other-GPU control, E6).

**Local runs (dev host, 2026-10-03; no GPU), each with `cargo test` under the shared flock:**
- `cargo test -p kf-broker`: lib 27 (four new in `cursor.rs`), `tests/host_cursor.rs` 8,
  `relay_machine.rs` 35, `proto_mirror.rs` 5 — all passed.
- `cargo test -p kf-disp --lib`: 64 passed (four new host-cursor tests).
- Bite-mutations, each applied to a copy and restored, each failing at least one test (with
  `--no-fail-fast` where a unit test would otherwise stop the run): the relay ignoring the grab
  (`grab_composes_and_sends_nothing_and_its_end_sends_what_changed`,
  `a_broker_that_says_hello_grabbed_starts_composed`); `CursorMode::composes` composing in hover
  (`hover_leaves_the_cursor_out_of_the_frame_and_grab_composes_it`); no digest compare (six tests,
  incl. `hover_sends_the_image_once_in_a_memfd_and_again_only_when_it_or_its_hot_spot_changes`);
  `F_GRABBED` ignored on ordinary packets; the `CAP_CURSOR` gate dropped; the mailbox keeping the
  oldest post; the memfd left open after the send; a command sent under grab; in `scanout.rs` no
  premultiply, inverted coverage, a transparent image shown, XOR accepted, an additive blend accepted,
  the source pitch ignored.
- kf3.c is unchanged by this step (the cursor is Rust's end to end); KF3 ABI stays 12.

**Box (vdisp = vast 54032077, RTX 3060, host driver 580.159.04, KDE on Xorg with the NVIDIA DDX).**
Evidence: `traces/v3_display/broker_20261003/<run>/`; harness `scripts/bench/display/broker_lane.sh`
(`prep`, `run`) and `broker_hook.sh`.

- **Run `brkA` — kf3 `6da16d2f` (the binary's path stamp, `run_brkA_rev.txt`), harness `aa141a99`,
  broker `9cb736f` (nvkvm-pv `broker-cursor-gpucopy`), 2026-10-03.** `display-broker-vram=auto`,
  the broker on X11 with its default present mode. The broker was started by hand ~20 s into the
  boot (the lane's own start failed — the session user cannot traverse `/root`; fixed in
  `d2864f2d`), hence `failed_attempts=6` before it connected.
  - **E0.** Driver 580.159.04 (inside 575.51.02 … 615.71.09); nvidia-drm loaded `modeset=N` — the
    rung's precondition was NOT met as the box came; `prep` reloaded it `modeset=1` with no X server
    up. Render node `renderD128` (226:128), `root:render 0660` plus an ACL for the desktop's user.
    `GET_DEV_INFO` gave the modifier `0x0300000000606014` and gpu_id 0x7 (`GPU-copy rung possible`).
  - **EV_DEVICE names the real render node:** the broker logged `the X server renders on DRM device
    226:128 (render node)`, and the relay `the compositor renders on this GPU`.
  - **Rung 0 on the same GPU, and the fallback before it:** the first frames went LINEAR (the VRAM
    slots are provisioned only at the first explicit yes); the NVIDIA X server then refused LINEAR
    (`the display CANNOT show XR24 modifier 0x0`, its frame reclaimed by name) and answered YES for
    block-linear; from then on every frame was a GPU copy — the device's exit status:
    `broker[sent=3446 gpucopy=3445 releases=3445 …]`, `scanout_pack=3464 scanout_d2h=360
    display_vram_mib=50`: every GPU-copy frame came back RELEASEd, i.e. the X server imported each.
  - **The guest desktop end to end through the broker:** `host_desktop.png` is the HOST's root window
    with the broker window fullscreen, showing the guest's Cinnamon session (its own
    "fallback mode" dialog — a guest-side Cinnamon fallback this bench's M3 lanes have seen before,
    not a display fault) drawn correctly from the block-linear copy. The window resize also re-moded
    the guest through the 3c hotplug (`resize 1024x768 -> monitor 1024x768 … hotplug posted`).
  - **The hover cursor replaces the host pointer:** mode `hover` from the connection on; one SET
    (`guest cursor image 256x256 hot 0,0` — the hot spot defect above); the host's cursor over the
    window was the guest's arrow (`cur_host_hover.png`, 254 visible pixels).
  - **Hidden when the guest hides it:** the guest called `XFixesHideCursor` for 10 s: the host's
    cursor over the window was blank (`visible_px=0`), then the same image again (same digest)
    after; counters `cursor_hides=2 cursor_shows=1`, `host_cursor_reads=1672 host_cursor_refused=0`.
  - **CTRL+ALT+G:** `grab ON` → `guest cursor: composed into the frame (grabbed; …)`, the relative
    device selected, the host's cursor blank (`cur_host_grab.png`); a second CTRL+ALT+G → `grab off`,
    hover again, the guest's image back over the window. ⚠ Whether the frame carried the composed
    cursor under grab was NOT graded in this run: the hook compared the console's screendumps, which
    on rung 0 stay at a stale 640x480 frame (§8.11's "screendump freshness" limit — no host copy is
    made for an idle console while the broker takes the GPU copy), and it overwrote the root-window
    shots with the cursor images (both fixed in the hook for the next run).
  - **E5 (part), raw:** QEMU held 10 dma-buf descriptors (five VRAM slots and five udmabufs would be
    ten; not attributed one by one), 5 `/dev/nvidiactl` and 311 `/dev/nvidia<N>` descriptors, and
    8728 MiB of VRAM (the 8192 MiB store and 50 MiB of display slots are inside it). ⚠ This listing
    cannot tell a slot's RM export fd from QEMU's other `nvidiactl` descriptors, so "the export fd is
    closed after the imports" is NOT graded by it.
  - **display-max-fps:** NOT built anywhere (`OWNER_RULINGS.md` §M is a ruling, no branch carries the
    property), so it could not be graded. (⊘ Built since on `v3-maxfps`, 2026-10-04, §8.16; not run
    on a box.)
- **Runs `brkA4` … `brkE1s2` — kf3 `18562ba4`, harness `18562ba4`, broker `9cb736f`, 2026-10-03, one
  chain (`chain_A4_B_C_D_E1.log`: every run rc 0), and `brkA5` — kf3 `c38032f3` (the hot-spot
  fixes), 2026-10-03.** Evidence per run under `traces/v3_display/broker_20261003/<run>/`.
  - **The hover cursor, graded against the guest's own (`brkA5`).** With the guest's root-window
    cursor set to `left_ptr`, the host's cursor over the picture had hot spot `3,1` — the guest X
    server's own is `3,1` — the same visible pixels relative to the hot spot (`bbox_rel_hot
    -1,-1,14,22` on both sides, 254 pixels); with a `crosshair`, `11,11` against the guest's
    `11,11`, 281 pixels on both sides. The relay sent one SET per shape (`hot 0,0` before the
    pointer ever entered the window, then `3,1`, then `11,11`). Colours: identical on every opaque
    pixel; on partially transparent ones the host's is the guest's times alpha (62 of the arrow's
    254, 13 of the crosshair's 281; e.g. 137 at alpha 166 became 89). ⊘ REFUTED 2026-10-04 by the
    register (`0x072ff`, a premultiplied blend; the correction at the top of this section) — the
    darker edge is in the surface's pixels. As written then: ⚠ Inferred, not read from a
    register (the composition word is not logged): the cursor surface holds the X server's
    premultiplied pixels and the head is programmed with the non-premultiplied blend, so the
    head itself scans out the darker edge, and the host shows what the head would.
  - **Hidden when the guest hides it** (`brkA4`, `brkA5`): blank (`visible_px=0`) while the guest
    holds `XFixesHideCursor` for 10 s, the same image (same digest) after.
  - **CTRL+ALT+G** (`brkA4`, `brkA5`, `brkC`): `grab ON` → `composed into the frame`, the host's
    cursor blank; the broker's picture then carries the guest's crosshair at the guest pointer
    (76 changed pixels in a 96-pixel box, `crop_hover_grab_moved_x3.png` in `brkA4`), a relative
    move under grab moves it (76 again at the new position: 79,29 → 119,56), and after `grab off`
    the frame is cursor-free again (0 changed pixels against hover). On `brkB`/`brkC`/`brkD`, where
    the console's frames are fresh (host-memory rungs), the console shows the same.
  - **Rung 0 vs the fallbacks:**
    | run | configuration | what carried the frames | broker counters |
    |---|---|---|---|
    | `brkA4`, `brkA5` | `display-broker-vram=auto`, X11 default | block-linear GPU copy after the X server refused LINEAR | `sent=3912 gpucopy=3910 releases=3899` (`brkA5`) |
    | `brkB` | `vram=off`, X11 default | LINEAR refused, then F_SHM which an X11 broker in its default mode does not present: a blank picture | `sent=205 releases=0 reclaims=204` |
    | `brkC` | `vram=off`, broker `--present-mode=shm` | F_SHM, presented | `sent=3005 releases=3005` |
    | `brkD` | `vram=auto`, broker `--present-mode=linear` (E2) | the broker said NO to block-linear: no GPU-copy frame and NO display VRAM (`display_vram_mib=0`), LINEAR refused: blank | `sent=205 gpucopy=0 releases=0` |
    So on X11 with the NVIDIA DDX, rung 0 is the only rung that shows a picture without changing
    the broker's present mode — §8.11's prediction, now run.
  - **E1, the three slot attribute sets** (`brkE1s0/s1/s2`, `display-broker-vram=on`): for S0
    (store-like), S1 (NVKMS offscreen) and S2 (NVKMS scanout) alike, realize provisioned 50 MiB in 5
    slots and the pack self-test passed on the GPU (`pack kernel self-test PASSED (into display VRAM
    slot 0)`), the X server imported them and every GPU-copy frame came back released (S0
    `gpucopy=2285 releases=2284`, S1 1474/1474, S2 1458/1458). Not run from E1: the FNV-matched
    image at two sizes, the export fd's closure, and a fence that is ever unsignalled.
  - **E3** (`brkA5`): the broker SIGSTOPped for ~7 s did not hold the guest (it answered, `glxgears`
    ran at 84.8 FPS); after `kill -9` and a restart the relay reconnected and replayed
    (`re-sent geometry 1024x695 and the last frame to the new broker`, `reconnected … #1`).
  - **EV_DEVICE** named `226:128` (renderD128) in every run with a broker on this X server.
- **Runs `brkA2` (kf3 `1ddccbdf`) and `brkA3` (kf3 `7753459b`) graded NOTHING about the cursor, and
  the cause is the bench, not kayfabe.** Both connected, took rung 0 and imported every frame
  (`brkA3`: `sent=4088 gpucopy=4086 releases=4073`), but CTRL+ALT+F/G never reached the broker and
  the guest pointer never moved (no ABS, so both SETs said `hot 0,0`). The KDE session had locked
  itself while idle between `brkA` and `brkA2` (`kscreenlocker_greet`, `LockedHint=yes`): the locker
  holds the keyboard and pointer. Unlocked, the broker's own test client received every key and the
  grab toggled. A second harness defect surfaced on the way: the `runuser` wrapper ignores SIGTERM,
  so `brkA3`'s broker restarted by its E3 step outlived the lane. Both fixed in `18562ba4` (`prep`
  turns auto-lock off; `run` unlocks and stops every broker on its socket before and after).
  ⊘ What `brkA2`/`brkA3` DID show: on a crosshair (`xsetroot -cursor_name crosshair`) the guest X
  server's own cursor is 24x24 with hot spot 11,11 — the case an underived hot spot 0,0 gets wrong
  by half the image; and E3's SIGSTOP of the broker for ~7 s did not hold the guest (it answered,
  `glxgears` ran at 82 FPS meanwhile).

### 8.13 The guest cursor on QEMU's console in hover (the coordinator's decision, 2026-10-04)

> ⊘ **CORRECTED 2026-10-04, later (the two reviews of this section's code; fixed on `v3-broker`,
> GPU-free-tested, NOT run on a box — the STATUS below predates it and its VNC lines must be
> re-run):**
> - **The pixels are PREMULTIPLIED now, not straight.** QEMU sends `QEMUCursor.data` to an
>   alpha-cursor VNC client verbatim (`ui/vnc.c:1001-1010`) as the Cursor With Alpha
>   pseudo-encoding (-314), whose pixels the protocol defines as premultiplied (`rfbproto.rst`:
>   "Alpha is pre-multiplied for each colour channel"; TigerVNC's
>   `CMsgReader::readSetCursorWithAlpha` divides each channel by alpha on receipt — both read
>   2026-10-04). Straight words came out too bright there and wrapped in an 8-bit channel. One
>   `QEMUCursor` cannot suit both conventions: SDL (`ui/sdl2.c:763`) and GTK (`ui/gtk.c:480`) read
>   the words as straight, so they now show a partly transparent edge slightly darker — exactly as
>   with QEMU's own virtio-gpu, which hands every frontend the guest's premultiplied cursor
>   (`hw/display/virtio-gpu.c:74`). The decision serves VNC, so VNC is the one made right.
>   ⚠ The box's "VNC digest = host pointer digest" was graded by `vnc_cursor.py` premultiplying what
>   it received — a round trip under kayfabe's own convention, not what a viewer draws. The grader
>   now takes the wire as premultiplied (and counts `wire_above_alpha`, a spec violation).
> - **The define follows the frame the console SHOWS** (`kf_broker::ShownFrame`; the worker marks
>   each frame it publishes, the console notes the one it takes). The relay flips the mode the
>   moment it reads `EV_GRAB`, while the console keeps the last frame until its next refresh (VNC's
>   backs off to 3 s): the image used to be defined at once beside the cursor still composed into
>   that frame — two cursors after every ungrab and at every broker connect — and hidden at once
>   at a grab — none until the next refresh. Now: no cursor of ours while the shown frame carries
>   one; in hover the image once the console shows a cursor-free frame; while the worker is about
>   to compose it (grab, an XOR cursor) the image the console holds stays until a frame that
>   carries it is shown. `kf3_gfx_update` takes its frame FIRST, then asks about the cursor.
> - **At most one DEFINE per 16 ms** (`DEFINE_MIN_MS`; the newest state at the first poll after).
>   A broker sets the grab on every packet, so one flipping it per packet made QEMU's main loop
>   allocate, copy and send a 256x256 cursor to every VNC client per packet, unpaced.
> - **The pointer (`dpy_mouse_set`) is moved only in hover, for an image the console holds, and
>   only under an ABSOLUTE pointer** (`qemu_input_is_absolute`); never turned "off" — the hidden
>   cursor hides it. ⊘ The sentence below that GTK "ignores it under an absolute pointer" was true
>   and beside the point: a broker grab is exactly what makes input RELATIVE, and GTK's
>   `gd_mouse_set` then WARPS THE HOST POINTER (`ui/gtk.c:447-467`) — with `-display gtk` and a
>   tablet not bound to this console, every grab warped it; so could a guest that leaves the
>   tablet idle. SDL needs one `on` move to show the guest sprite, which hover gives it.
> - **QEMU reports what it applied** (`kf3_display_cursor_done`, ABI 12's surface): a define it
>   could not make (`cursor_builtin_hidden`/`cursor_alloc` returning NULL — now checked — or a bound)
>   or a move it skipped is handed out again, paced; Rust no longer believes the console holds an
>   image it was never given. `kf3_display_cursor` and `kf3_display_frame` refuse a misaligned
>   pointer at the boundary (§R).
> - **The cursor point's "none" collided with a real point**: `u64::MAX` is `(-1, -1)` packed (a
>   crosshair with hot spot 11,11 at guest pointer 10,10). It is now 31-bit fields and a valid bit
>   (`kf_broker::CursorPoint`).
>
> Tests (each turns red under its bite-mutation, applied and restored 2026-10-04): `console.rs` —
> the image waits for a cursor-free frame (Grabbed→hover and Off→hover), grab/XOR keep the image
> until a composed frame is shown, the guest hiding and a broker gone hide at once with no pointer
> call, pacing with the newest state winning, an update not applied is handed out again, the pixels
> are the premultiplied words, the point round-trips `(-1, -1)`; `host_cursor.rs` — a broker
> flipping `F_GRABBED` 1000 times in 1 s gives at most `1000 / 16 + 2` defines (known-positive:
> flips 16 ms apart each define); kf-qemu `display.rs` — the frame's cursor bit reaches what the
> console shows. `vnc_cursor.py --selftest` takes premultiplied words as received and flags a
> straight word.

**STATUS: RUN ON A BOX — 2026-10-04 (box 54032077, runs `brkF1`/`brkF2` with `BRK_VNC=1`, kf3 code
`34696441`, broker `badf2d7`; `traces/v3_display/broker_20261004/`).** In hover an alpha-cursor VNC
client received the crosshair as `256x256 hot=11,11`, 281 visible pixels, with the host pointer's
digest exactly (`fnv_rel_hot=0xf6108d49685ef58f`), and the xterm glyph (`hot=4,8`, 86 pixels) with the
guest X server's own digest; under grab QEMU's hidden cursor (`32x32`, nothing visible). With the guest
scaled into the broker window the console's cursor stays in guest pixels (the console is not scaled).
Not run: GTK, SDL, `dpy_mouse_set`'s position (VNC ignores it). (As first written: "BUILT IN CODE,
GPU-FREE-TESTED — 2026-10-04 (branch `v3-broker`); not run on a box.")

**Decision (the coordinating session's, for the owner, 2026-10-04; the owner may revisit):** while a
`CAP_CURSOR` broker hovers, QEMU's own console (VNC, GTK, SDL) receives the guest cursor through
QEMU's cursor API — `dpy_cursor_define` + `dpy_mouse_set` — so a VNC client shows it as a real
pointer; under grab the cursor stays composed into the frame. It closes §8.12's deviation 1: the
frames are shared by the broker and the console, and in hover they carry no cursor (§O).

**The console's own mode without a broker: composed, unchanged** (decided here, same day). QEMU
gives a device no way to know whether a viewer can draw a defined cursor: its VNC server sends one
only to a client that negotiated the rich- or alpha-cursor encoding and silently not otherwise
(QEMU 10.2.4 `ui/vnc.c:992-1027`), and a `screendump` never contains one. Moving the cursor out of
the frame for the console alone would make it vanish for exactly the viewers that cannot say so,
while composing shows it on every frontend; with no broker there is no second viewer and so no
second cursor to avoid. (The alternative — the console's own "hover" whenever its input is
absolute — would suit cursor-capable VNC clients better; it is an owner question, not built.)

**What the console is told** (`kf_broker::console::ConsoleCursor`, VMM-agnostic, main loop only):

| relay mode (§8.12) | guest cursor | console |
|---|---|---|
| hover | an image | that image, defined once per image or hot spot — once the console SHOWS a frame without the cursor composed (⊘ 2026-10-04, later); moved once per position (the image's top-left on the head, which the worker stores every pass, plus the hot spot), under an absolute pointer only |
| hover | hidden | the hidden cursor (no pointer call) |
| hover | one the frame composes (XOR) | the image it holds until the console shows a frame with the cursor composed, then the hidden cursor |
| grab | (composed into the frame) | the same: the image it holds until the shown frame carries the cursor, then the hidden one — never two cursors, never none |
| no `CAP_CURSOR` broker / broker gone | (composed into the frame) | nothing — or, once the console was ever given a cursor, the hidden one at once (the worker no longer reads the cursor, so what the console holds is stale) |

**Ownership and bounds (QEMU 10.2.4).** `kf3.c`'s `kf3_console_cursor` runs in the console's
`gfx_update` — ⊘ AFTER it takes its frame since the correction above (as first written: "at the top
of", which defined a cursor against the frame about to be replaced); every refresh, since a cursor
change in hover makes no frame — and after every broker pump (each
cursor post is followed by a frame publish, which lands there — VNC's refresh backs off to
`VNC_REFRESH_INTERVAL_MAX` = `GUI_REFRESH_INTERVAL_IDLE`, 3 s, on a still picture, `ui/vnc.c:61`,
`include/ui/console.h:48`). `cursor_alloc` and
`cursor_builtin_hidden` hand the caller one reference (`ui/cursor.c:93-108`); `dpy_cursor_define`
takes its own (`ui/console.c:961-980`), and kf3.c drops its own right after. The pixels are kayfabe's
own copy of the image (the `CursorShare` post the broker is sent, made by a GPU copy into memory kf
owns) — never guest memory — copied into the `QEMUCursor`'s data: exactly `width * height` words
(`kf3_display_cursor_pixels` refuses a null or misaligned pointer, more than 256x256 words, or a
count that is not the defined image's), each a host-endian `0xAARRGGBB` — ⊘ PREMULTIPLIED since the
correction above (as first written: "with STRAIGHT alpha (QEMU's SDL frontend reads the words as
straight ARGB, `ui/sdl2.c:763-764`; the broker's image is premultiplied, so each channel is divided
by its alpha)").

**KF3 ABI stays 12**: the broker's surface is unmerged, so `Kf3Cursor`, `kf3_display_cursor` and
`kf3_display_cursor_pixels` join it; an archive without them fails to LINK with a kf3.c that calls
them, never at run time. `tests/wire_mirror.rs` carries the new layout and constants; the unsafe
ratchet of kf-qemu moves 55 → 59, itemised in `ci.yml`.

**Tests (GPU-free, `kf-broker` `console.rs`, each with a known-positive):** an image defined once
and moved per position, nothing for the same posts under grab or without a broker; hidden, XOR,
grab and a broker gone each define the hidden cursor once and turn the pointer off, and the image
comes back in hover; the copy is straight ARGB of exactly `width * height` words or nothing. (As
first written; ⊘ since the correction above the pointer is never turned off, grab and XOR hide only
once a composed frame is shown, and the copy is the premultiplied words — the tests changed with
it.)
Bite-mutations, each applied and restored on 2026-10-04: the hidden cursor defined before any
hover, an image shown under grab, premultiplied pixels passed through — each turns a test red.
kf3.c: `-fsyntax-only -Werror` with QEMU 10.2.4's own warning flags (gcc 15), clean.

**Not run (as first written; run since — the STATUS above):** any of it on a box. The grading is ready, opt-in: `BRK_VNC=1 broker_lane.sh run <tag>`
gives QEMU `-vnc 127.0.0.1:7`, and the hook's hover step then reports (`BRK_VNC_HOVER`) the cursor an
alpha-cursor VNC client receives — `scripts/bench/display/vnc_cursor.py`, an RFB client in the stdlib
whose `--selftest` replays QEMU 10.2.4's own bytes (`ui/vnc.c:992-1027`) — and compares it with the
guest X server's own through XFixes (`xcursor.py compare`; the straight↔premultiplied round trip may
cost a channel step of rounding); under grab (`BRK_VNC_GRAB`) it must be the hidden cursor. Also
not run: GTK and SDL frontends, and `dpy_mouse_set`'s position (only SPICE and D-Bus listeners use
it; VNC ignores it, GTK and SDL ignore it under an absolute pointer — ⊘ and WARP the host pointer
under a relative one, which a grab makes it: the correction above). The composition word line is
collected by the hook too (`BRK_CURSOR_COMPOSITION`, §8.14).

### 8.14 XOR cursors, and the cursor's alpha (`OWNER_RULINGS.md` §O; 2026-10-04)

**STATUS: RUN ON A BOX — 2026-10-04 (box 54032077, runs `brkF1`/`brkF2`/`brkF3`, kf3 code
`34696441`; `traces/v3_display/broker_20261004/`), the XOR blend only as the GPU self-test.** The
compose kernel's bring-up self-test, with its XOR pixel, `PASSED` on the RTX 3060 in all three runs.
No guest cursor reached the head as XOR: the guest X server's xterm core glyph with no theme arrived as
a two-colour ARGB image under the unchanged word `0x072ff` (`MODE_BLEND`) and was shown as the host
pointer, identical to the guest's (86 pixels) — the source reading below, now on hardware. So "an XOR
cursor composed in hover with the host's hidden" is still not run: it needs a guest that writes the
core channel itself. The composition word was read: see "The cursor's alpha" below. (As first
written: "BUILT IN CODE, GPU-FREE-TESTED — 2026-10-04 (branch `v3-broker`); not run on a GPU.")

**What NVKMS can program that is XOR- or invert-like: nothing** (`ogkm-580`, read 2026-10-04). Its
cursor composition table writes `MODE_BLEND` for each of the five blending modes it supports
(`src/nvidia-modeset/src/nvkms-evo3.c:6646-6702`; the same at `nvkms-evo4.c:965-1019`; the mode set
is `NV_EVO3_SUPPORTED_CURSOR_COMP_BLEND_MODES`, `nvkms-evo3.h:44-49`: opaque, premultiplied and
straight alpha, each with or without a surface alpha in K1), EVO2's only `_ALPHA_BLEND` and
`_PREMULT_ALPHA_BLEND` (`nvkms-evo2.c:3194-3206`); its only cursor format is `A8R8G8B8`
(`nvkms-evo3.c:6517-6524`); and no factor pair can invert (`out = src*fs + dst*fd`, both factors in
0..=1). The class has `MODE_XOR` (`clc37d.h:859-861`) and a second cursor format, `A1R5G5B5`
(`clc37d.h:836`), so an XOR cursor reaches a head only from a guest that writes the core channel
itself — a hostile one, or another OS's driver.

**The XOR blend, as built — a DEFINITION, stated as one:** `out = below XOR (source & 0x00ffffff)`;
alpha and the composition factors play no part (the class header names the mode and says nothing of
what it computes). The literal reading was taken: an all-zero image composes to nothing, a white
pixel inverts. ⚠ The alternative is the masked-colour reading (alpha as the AND mask, which lets one
surface carry opaque pixels too — what a Windows monochrome pointer needs); only hardware, with a
producer that programs the mode, can settle it. `A1R5G5B5` stays refused by name (the kernel moves
32-bit pixels). The host cursor still refuses XOR (no ARGB "over" expresses it), so in hover an XOR
cursor is composed AND the host's is hidden — now visibly, where before it was refused in the frame
too.

**Built:** `kf_disp::scanout` — `COMPOSE_*` flag constants; `plan_cursor` plans `MODE_XOR` as an XOR
layer (factors not consulted; an unknown MODE refused by name); `compose_reference`, the CPU
reference of the kernel's whole per-pixel arithmetic (address, swap, opaque, XOR, blend with C's
truncating division). `cuda/display/kf_scanout.ptx` (hand-written, no generated source and so no FNV
pin): flags bit 3 = XOR, one branch, no parameter change (kf-cuda's parameter-list test holds).
`kf_cuda::display::ComposeLayer::check` refuses a flag bit the kernel does not know
(`COMPOSE_FLAGS`), and kf-qemu pins the two flag sets equal at compile time. The display worker's
bring-up GPU self-test gains an XOR pixel: alpha 0 and factors that would make a blend write black,
so a kernel without the path, or one that lets alpha gate it, fails before a guest cursor is
composed.

**"The same arithmetic on the host" for hand-written PTX** (`kf-disp/tests/compose_kernel.rs`): the
committed PTX TEXT is parsed and executed, instruction by instruction, for every (CTA, thread) of the
launch kf-cuda makes, and the frame it leaves is compared byte for byte with `compose_reference` —
pitch and block-linear (block heights 1 and 16 GOBs) surfaces, every blend NVKMS programs with and
without source alpha, red/blue swap, XOR over a non-black frame, a 300-pixel row (the threads'
stride loop), clipping at the frame. Every load and store must fall in the source extent or the
frame, so the kernel's own bounds claim is checked; an instruction the interpreter does not model
panics, so a kernel edit cannot pass unmodelled. Bite-mutations on the PTX, each applied and
restored on 2026-10-04 — the XOR branch skipped, the alpha byte XORed too, the blend's rounding
dropped — each turn two tests red. ⚠ It is the kernel's logic, not the PTX JIT or the GPU: those
are graded by the bring-up self-test on a box.

**The cursor's alpha (§8.12's open item).** On box 54032077 the host's semi-transparent cursor
pixels were the guest's colour times alpha — inferred, not read, as a straight-alpha blend
programmed over premultiplied pixels. Source reading cannot settle it: kf-disp's mapping is the
head's own arithmetic for each of the five modes (unit-tested since 3d); nvidia-drm maps DRM's
default "Pre-multiplied" blend mode (the plane state's reset value in DRM core) to `PREMULT_ALPHA`, or
to `PREMULT_SURFACE_ALPHA` with K1 = the plane alpha, 255 by default
(`ogkm-580: kernel-open/nvidia-drm/nvidia-drm-crtc.c:306-353`); and the guest X driver that would
choose a straight blend (the NVIDIA DDX, which drives NVKMS directly) is closed. So the worker now logs,
once per change of the word, on the read the host cursor already makes:

```text
kf3: display: guest cursor composition 0x075ff = NON_PREMULT_ALPHA (straight alpha) (K1 255, cursor
factor 5, viewport factor 7, mode 0); its 64x64 pixels: N partially transparent, 0 with a colour
channel above alpha (consistent with premultiplied pixels)
```

(`kf_disp::scanout::cursor_composition` — the word as programmed, `clc37d.h:850-861` — and
`HostCursorSrc::alpha_census`; a pixel with a channel above its alpha is impossible when
premultiplied.) ⊘ Bounded since the review of 2026-10-04 (later): "once per change" of a word the
GUEST programs is a line per frame for a guest alternating two words — gigabytes a day of QEMU's
stderr — so the line now carries its change number and is logged for the first 8 changes and every
256th after (`kf_disp::scanout::CompositionLog`, test
`the_composition_line_is_bounded_however_the_guest_alternates`). If the line says a straight blend over premultiplied-consistent pixels, the head
itself scans out the darker edge and the host shows what the head would: the mapping stays, and
the darker edge is the guest driver's.

**On the box (2026-10-04, `brkF1`/`brkF2`):** `guest cursor composition 0x072ff = PREMULT_ALPHA (K1
255, cursor factor 2, viewport factor 7, mode 0); its 256x256 pixels: 163 partially transparent, 0 with
a colour channel above alpha`. A premultiplied blend, not a straight one: the inference this paragraph
opens with is refuted. Every pixel the host shows differently from the guest X server's own cursor is
exactly the guest's colour times its alpha, and the VNC console's copy of kayfabe's image is identical
to the host's — so the surface holds X's premultiplied pixels premultiplied again by the guest DDX, and
the head itself would scan out the darker edge. The mapping stays
(`traces/v3_display/broker_20261004/cursor_alpha.txt`).

### 8.15 nvkvm-pv's broker at `badf2d7` — what the relay now handles (2026-10-04)

> ⊘ **CORRECTED 2026-10-04, later (the two reviews of the relay changes; fixed on `v3-broker`,
> GPU-free-tested, NOT run on a box — a GPU-copy run must be re-run for `BRK_COUNTER_GRADE`):**
> - **A dma-buf commit the broker refuses BY NAME no longer counts toward the silent-drop
>   detector** (`Ack::uncount`, when `EV_FORMAT x=0` reclaims a committed frame of its pair). The
>   first frames of a connection race the relay's own question and are dropped at the format gate;
>   counted, three of them over a second (with another dma-buf pair committed after them, as the
>   test does) tripped a back-off whose line blames `/proc/self/fdinfo`. Test
>   `a_dma_buf_frame_refused_by_name_never_trips_the_detector` (known-positive: the same frames
>   dropped silently trip it).
> - **Row 1's eviction order was wrong**: the table dropped its oldest non-"no" FIRST, so sixteen
>   volunteered "no"s evicted the relay's own recorded YES for the GPU-copy pair — the rung, the
>   worker's pack and the "CAN show" line then flapped frame by frame. Now: volunteered rows (any
>   verdict) first, then the relay's own non-"no"s, its own "no"s last. Test
>   `volunteered_noes_never_push_out_the_relays_own_yes`.
> - **Row 5's refusal refused the FRAME on the rung `choose` picked** — a GPU-copy descriptor that
>   failed the check would have been picked again for every frame: a black broker display with
>   nothing tripping. Now a refused dma-buf rung (GPU copy, or the host dma-bufs) backs off for the
>   connection like an unacknowledged one, and the same frame takes the next rung; only a refused
>   `F_SHM` memfd, or a frame no other rung holds (a VRAM-only GPU copy), refuses the frame — by
>   name. The check runs BEFORE a seq is spent or a rung announced
>   (a refused frame used to log "frames go as a LINEAR dma-buf"), and once per backing — a
>   proven descriptor is not re-read from `/proc` per frame (`carrier_checks`). Tests:
>   `a_descriptor_that_is_neither_memfd_nor_dma_buf_never_reaches_the_broker` (now also the GPU
>   copy's pipe), `a_proven_descriptor_is_checked_once_per_backing`.
> - **The HELLO line said "no /dev/udmabuf here"** at every first connection (no slot holds a
>   descriptor before the guest's first frame; box run `brkF1`). It now speaks only when installed
>   slots all lack a dma-buf. Test `the_hello_line_claims_shared_memory_only_for_slots_without_a_dma_buf`.
> - **The status line now carries `dmabuf_trips`, `carrier_refused`, `carrier_unchecked` and
>   `formats_unasked`**, and `broker_lane.sh` grades a GPU-copy run on them (`BRK_COUNTER_GRADE`:
>   PASS needs `carrier_refused=0 dmabuf_trips=0`; a status line without them is UNMEASURED, never
>   zero). Rows 4 and 5's "RUN ON A BOX" had no counter evidence before this.
>
> Bite-mutations, each applied and restored 2026-10-04 (later): the uncount removed, the old
> eviction order, the check cache bypassed, no fall-back to the next rung, a counter dropped from
> the status line (each of the four), the old HELLO rule, a seq spent and a rung announced before
> the check — each turns its test red.

**STATUS: RUN ON A BOX — 2026-10-04 (box 54032077, runs `brkF1`/`brkF2`/`brkF3`, kf3 code
`34696441`, broker `badf2d7` built on the box; `traces/v3_display/broker_20261004/`), except a DRI3
refusal, which the NVIDIA X server never made.** The first frame(s) of each connection went LINEAR
(dropped at the broker's format gate, then reclaimed on its `x=0`) or `F_SHM`; after that every
frame was a block-linear GPU copy and came back released (`brkF2`: `sent=7263 gpucopy=7262
releases=7262 reclaims=1`). (⊘ Corrected 2026-10-04, later: this said "Rung 0 carried every frame",
which the counts do not show; `brkF1`'s second connection logged two LINEAR gate drops and an
`F_SHM` line against an X11 dma-buf-tier broker that refuses `F_SHM` — the evidence `README.md`
has it right.) After E3's `kill -9` the relay's new connection asked
LINEAR and block-linear again (no "no" carried across connections). A DRI3 client
(`scripts/bench/display/dri3_refusal.py`) had the NVIDIA DDX 580.159.04 import ten malformed
descriptors of a real block-linear bo — wrong pitch, offset, kind, block height, a udmabuf under the
NVIDIA modifier, block-linear extents past the buffer's end — and every one was imported and presented
with no X error, so no unsolicited `x=0` and no twins were seen on the GPU. The only refusal this X
server makes is the format gate (LINEAR, not advertised), and the relay asks about LINEAR before that
drop's `x=0` arrives, so its reaction to a volunteered "no" for an unasked pair was not run there. The
twins and their per-connection lifetime were shown on `badf2d7`'s test backend by the same client
(`local_dri3_testbackend.txt`). (As first written: "BUILT IN CODE, GPU-FREE-TESTED — 2026-10-04 (branch
`v3-broker`); run locally against the real `badf2d7` broker (dev host, root, `/dev/udmabuf`); not run
on a box.")

nvkvm-pv's review fixes (`broker-cursor-gpucopy` at `badf2d7`) change what the broker DOES on an
unchanged wire. The protocol header is byte-identical at `8665a2d`, `9cb736f` and `badf2d7`, so the
vendored copy (still `368d2db`, with the newer values carried "ahead" in `wire.rs`) needs no bump;
`proto_mirror.rs`'s check against the newer header (`KF_BROKER_PROTO_NEXT`) now runs against the
committed `badf2d7` header — its first run found the census reading a `_Static_assert` as an enum
entry, fixed. Each behaviour, with its relay change and the test that turns red without it
(`kf-broker/tests/relay_machine.rs` unless named):

| # | the broker (`badf2d7`) | the relay | test |
|---|---|---|---|
| 1 | sends unsolicited `EV_FORMAT x=0` whenever an ATTACH is dropped at its format gate — once per pair per connection, possibly a pair the relay never asked about (`nvkvm_broker.c:1735-1749`) | RECORDS it (it used to log "stale" and drop it); an unasked `x=1` is still never an upgrade. The verdict table grows 4 → 16 rows and evicts, in order, a volunteered row (any verdict), one of the relay's own questions or yeses, and only then one of its own "no"s (⊘ as first written: "a question, a volunteered 'no', and only then a 'no' for a pair the relay sends" — which let volunteered "no"s push out the relay's yes, the correction above) | `an_unasked_no_is_recorded_and_its_pair_is_not_sent`, `volunteered_noes_never_push_out_a_no_for_a_pair_the_relay_sends` |
| 2 | on X11 sends two `x=0` for one refusal — XR24 and AR24, the same modifier (`:781-821`) | both kept; the frames held under the refused pair come back once | `an_x11_refusal_names_both_alpha_twins_and_both_are_kept` |
| 3 | forgets its refusals at detach, on both backends | carries none across a reconnect: verdicts and both acknowledgement detectors are connection state (`Conn`) | `no_refusal_or_back_off_outlives_its_connection` (an unasked no, the block-linear no, a detector back-off) |
| 4 | accepts a dma-buf only when `/proc/self/fdinfo` proves it (`exp_name:`), and drops one it cannot prove with NO word on the wire (`:1675-1689`, `:2219-2231`) | the host dma-buf rungs (LINEAR, implicit) get their own acknowledgement detector (the GPU-copy detector, refactored into `Ack`): unacknowledged dma-buf commits back off to `F_SHM` for 5 s, doubling to 60 s, until a RELEASE acknowledges the class; the log names the cause | `unacknowledged_dma_buf_frames_back_off_to_shared_memory_and_retry`; `broker_loopback.rs` `a_broker_that_cannot_prove_a_dma_buf_gets_shared_memory` — the REAL broker in a mount namespace with a tmpfs over `/proc` logs `cannot read /proc/self/fdinfo`, the detector trips, `F_SHM` frames are released |
| 5 | closes any fd it cannot prove is shmem or a dma-buf on a helper thread (`nb_fd_drop`, `:1487-1520`) | every descriptor the relay sends is classified first with the broker's own two tests in its order (`kf_linux_raw::fd_carrier`: `F_GET_SEALS`, `fstatfs` only after it succeeded, then `/proc/self/fdinfo`'s `exp_name:`; never an `fstatfs` on an unproven descriptor); an `F_SHM` or cursor memfd must carry `F_SEAL_SHRINK`. A refused frame is refused by name, counted (`carrier_refused`), the connection stays up. ⊘ A descriptor the VMM cannot CLASSIFY (its own `/proc` unreadable, e.g. `-run-with chroot=` without `/proc`) goes anyway, counted (`carrier_unchecked`) and logged at a bounded rate: refusing every dma-buf for the VMM's missing `/proc` would black out a display the broker can show | `conn.rs` `the_descriptor_rule_refuses_only_what_it_can_name`; `a_descriptor_that_is_neither_memfd_nor_dma_buf_never_reaches_the_broker`; `host_fd_unsafe.rs` `the_brokers_descriptor_proof_tells_shmem_dma_buf_and_neither` (memfd, unsealed memfd, regular file, `/dev/null`, pipe, and a udmabuf) |
| 6 | paces cursor uploads itself (at most one per 8 ms, latest wins, a published snapshot) and scales the hot spot as `ceil(hot * out / in)` | no change: the relay sends at most one command per entry, never paces on its own, and sends the hot spot in guest pixels | `host_cursor.rs` `the_relay_leaves_the_cursor_pacing_to_the_broker` |

Bite-mutations, each applied and restored on 2026-10-04: an unasked "no" ignored (4 tests red), the
dma-buf back-off ignored (2), the descriptor check always passing (1), the eviction ignoring which
pairs the relay asked about (1). Not bitten: a "no" carried across connections (it would need the
verdicts moved out of `Conn`). The unsafe ratchet of kf-linux-raw moves 93 → 94 (`fd_carrier`'s
one `fcntl(F_GET_SEALS)`). Local runs, all against `badf2d7`:
`traces/v3_display/broker_20261004/local_runs.txt` (`broker_loopback.rs` 8/8 with `--ignored`).

**Not run:** X11's DRI3 refusal itself — the NVIDIA DDX refused none of the ten descriptors the box
stage tried (2026-10-04, above); a Wayland broker. (As first written, the same day: "any of it on the
box; X11's DRI3 refusal itself (it needs the NVIDIA DDX; `brkA`'s LINEAR refusal predates `badf2d7`); a
Wayland broker.")

### 8.16 `display-max-fps` — the configurable frame-rate bound (`OWNER_RULINGS.md` §M; 2026-10-04)

**STATUS: BUILT IN CODE on `v3-maxfps` (cut from `v3-broker` `82f98f42`), GPU-free tested locally and
in CI; NOT RUN ON A BOX.** The design was reviewed adversarially before it was built (kept outside
the repo, as §M says); the owner's decisions of 2026-10-04 are binding over it: D1 tearing flips are
gated, D2 (Claude's proposal, adopted unless the owner objects) replaces the fixed refresh clock, D3
refuses values above 75, D4 is pending the box run below, D5 keeps the unset EDID byte-identical.

**The property.** `-device kf3-gpu,display=on,display-max-fps=N`, whole Hz. 0 (the default) is
unset. `kf_disp::pace::check` refuses by name a value without `display=on`, below 24 (the slowest
refresh an authored CVT mode carries) or above 75 (D3: the EDID range ends there; single-link DVI
fits 1080p only up to 71 Hz). It runs first in `Device::realize` (`Config::check`), before anything
is opened. KF3 ABI 16: `kf3_realize` gains `display_max_fps` after `display_broker`.

**Two numbers** (`kf_disp::pace`): the **cap** is the property, or 75 unset, and never changes for the
device's life (it does not follow the host's monitor, so a valid 61–75 Hz mode is never ticked below
its raster); the EDID's **preferred rate** is `preferred_hz(cfg, host)` — 60 with neither, the host
window's rate under the cap when the broker reports one.

| cfg | host | preferred |
|---|---|---|
| 0 | 0 | 60 |
| 0 | S | S (clamped 24..75) |
| B | 0 | B |
| B | S | min(B, S) |

**The mechanism: a capped tick.** Each head ticks at `max(raster period, floor(1e9 / cap) ns)` —
the worker's epoll deadline, never a sleep, never on a vCPU, under no lock (`THE_CONSTRAINTS.md`).
`kf_disp::pace::Pacer` owns every period: it keeps a head in phase when its CLAMPED period did not
change, re-arms it from now when it did, stops an idle head, and reschedules a late tick from its
scheduled time (from now when more than a period late — no catch-up burst). The log line is
`head N ACTIVE raster WxH period P ns (raster R ns, cap C Hz[, CLAMPED])`, in nanoseconds (a 30 Hz
1080p CVT mode is `33401904 ns`; the old `period / 1000 us` truncated it). Everything the guest paces
by vblank follows the tick: non-tearing flips (parked for it), NVKMS's vblank callbacks, the
`RG_DPCA` frame counter and the head-timing interrupt.

**D1, tearing flips.** `Engine::tear_gate` (on by default): a group without the core holding a
tearing (immediate) window on an active head PARKS for the head's next tick when that head already
presented since its last one; the first after a tick still latches at once. A flip in kayfabe copies
a finished buffer, so it never tears; the gate is about rate only. Groups holding the core latch at
once (a modeset) — the tick does not bound them, so they are counted (`core_imm`) and included in
`over`. ★ F7: a group parked for a head that goes idle is unparked at once (an acquire-only wait if
its acquire does not hold) — its tick will never come.

**D2, copies without a flip.** They exist for front-buffer rendering — the boot console, X11 without
a compositor, front-buffer applications — and for cursor moves: kayfabe has no physical scanout, so
without a copy the host never sees those writes. The fixed 30 Hz / 4 Hz refresh clock is gone:
1. a CHECK runs only at the console head's (capped) tick — for a picture with no armed head (the boot
   layer, a preserved scanout) at the preferred rate under the cap — real scanout's cadence;
2. the check composes the frame into the staging frame and runs `kf_sum` (`cuda/display/kf_sum.ptx`,
   hand-written, a module of its own so a JIT refusal costs only the detection) over it: per row, the
   sum of `mix(p ^ x·0x9e3779b9)` — a bijection of each pixel per column, so one changed pixel always
   changes its row's sum; the host folds the rows into an FNV-1a digest. The frame is SENT (pack and/or
   D2H from the same staging frame, then published) only when it differs from the last published one
   — digest, size, cursor flag, or a backing the copy would fill now that the last frame lacked (a
   console that starts watching after VRAM-only frames). A new watcher (the console's or broker's
   first request after 2 s, a broker session that became active) and a cursor-mode switch force the
   next send;
3. nothing is checked while nobody watches (no console request and no active broker within 2 s).
   QEMU's console update is now asynchronous (`gfx_update_async`): each `gfx_update` — a `screendump`
   among them — asks for a frame no older than itself (`kf3_display_refresh`); the worker answers at
   the console head's next tick with a check that STARTED after the request (at once when nothing
   can be copied), signals a descriptor, and kf3.c shows the newest frame and ends the wait
   (`graphic_hw_update_done`); a 1 s backstop ends it anyway.
Flip copies are never gated and always sent (the flip is tick-paced already); they record their
digest, so the next check compares against them. A cursor move makes no copy of its own any more
(`move_recomposes` is gone): in grab the next tick's check sees the cursor moved; in hover the frame
composes no cursor, so nothing changes and nothing is sent — §O's "coalesced to the display rate".

**The EDID (D5).** Unset, `Monitor::configured(0)` is `default_1080p()` and its EDID is the
`82f98f42` one byte for byte (a golden array; nine 3c windows at 60 Hz and below are pinned by their
FNV too). Set: a 1920x1080 monitor at the cap (CEA at 60, CVT-RB otherwise), the range limit's
maximum (byte 78) = the cap, and below 60 NO 60 Hz mode is listed — no established or standard
timing, no second CEA DTD — because NVKMS keeps an EDID-listed mode even against the EDID's own range
(`ogkm-580: nvkms-modepool.c:1472-1490`). Above 60 the RATE yields, not the size: 1920x1080 asked at
75 Hz is 71 Hz at 164 750 kHz (⊘ before, a 72–75 Hz broker window became a 1800x1012 monitor). The
boot display's option ROM is built from the same monitor. The relay now de-duplicates `SURFACE` on
(w, h, refresh), so a host monitor that changes only its rate re-authors the guest's monitor.

**Status.** The worker meters each window of at least 1 s and prints, at most every 2 s and only on a
change, `kf3: display fps[cap=30(cfg) h0[armed=29.938 tick=29.94 clamped=0 late_max_us=812 kms=29.9
tear=0.0 held=0 core_imm=0 vblirq=29.9] copies=29.9 checks=30.0 same=12 ondemand=0 over=0]`; the
same fragment follows `disp[...]` in the status line (`try_lock`, never a wait). `armed` is the head's
tick rate, `tick` the achieved one, `kms` presents (latches, every path), `tear` the tearing ones,
`vblirq` ticks while the guest had the head's vblank interrupt enabled (a vblank consumer exists — a
precondition, not a grade), `copies` published frames, `checks`/`same`/`ondemand` D2's counters, and
`over` the windows in which some head's presents exceeded `floor(elapsed / period) + 2` (the late-tick
rule's worst case). ⚠ X11 vsync makes no flips and no display methods, so the status line cannot see
an X11 breach — X11 is graded in the guest.

**What §M named and this does NOT build.** `NV9072_CTRL_CMD_NOTIFY_ON_VBLANK` stays refused by name:
host CPU-RM writes the notifier only through a kernel mapping kayfabe never sets, so forwarding it
would turn a refusal into a dropped release; no client in the dispsw runs of 2026-10-03 called it
(`traces/v3_display/dispsw_20261003/README.md`, on `v3-dispsw-exp`). Doorbell pacing of
display-SW channels is not built (it would be per submission, leaky — the host twin reads the
guest's own `GP_PUT` — and would throttle the X server's and the compositor's channels too). Both
wait on D4.

**Tests (GPU-free), each with the mutation it was run against on 2026-10-04 (applied in place, the
named test went red, restored; every run, count and command in
`traces/v3_display/maxfps_20261004/local_runs.txt`):**

| check | test | mutation (red) |
|---|---|---|
| the property's bounds, by name | `pace::config_check_refuses_by_name`; `kf-qemu tests/max_fps_property.rs` drives `Device::realize` | `>=` at 75; `realize` without `Config::check` |
| tick = slower of raster and cap, floor | `pace::paced_period_is_the_slower_of_raster_and_bound` | `min` for `max` |
| the cap never follows the host | `pace::preferred_and_cap_tables` | `max` for `min` |
| phase kept under the clamp | `pace::on_heads_keeps_phase_under_the_clamp` | comparing the raster period |
| 30 ticks/s at cap 30; no burst after a stall | `pace::pacer_synthetic_time` | rescheduling every late tick from now |
| `over` = floor(L/period) + 2 | `pace::meter_over` | `>=` |
| D2 rules (tick, watched, coverage, force) | `pace::non_flip_copies_follow_d2` | a check while unwatched; off the tick; the host-coverage term dropped |
| a screendump is served only by a later copy | `pace::a_screendump_is_served_by_a_copy_that_started_after_it` | serving at the request |
| the checksum's arithmetic | `pace::the_checksum_sees_a_pixel_a_move_and_a_row_swap` | no column term; a digest over unordered rows |
| `kf_sum` PTX = the reference, in bounds | `kf-disp tests/compose_kernel.rs` `the_sum_kernel_is_the_reference_checksum`, `the_sum_kernel_reads_only_its_frame` | the PTX's column multiplier set to 0 |
| the launch matches the PTX's parameters | `kf-cuda display::the_sum_kernel_declares_the_parameters_the_launch_passes` | — |
| unset EDID byte-identical | `edid::the_unset_edid_is_byte_identical` | (golden array from `82f98f42`) |
| below 60: no 60 Hz mode, byte 78 = cap | `edid::a_cap_below_60_lists_no_60hz_mode`, `edid::every_authored_edid_honours_its_cap` | range max fixed at 75; extras kept |
| above 60 the rate yields | `edid::above_60hz_the_rate_yields_not_the_size` | the old shrink loop |
| the model serves the configured monitor | `kf-rm display::the_model_serves_the_configured_monitor` | `model_for` ignoring the property |
| D1 gate | `engine::a_second_tearing_flip_waits_for_the_vblank` | gate off by default; `presented` ignored; not cleared at the tick |
| presents per latch, core counted | `engine::presents_count_one_per_latch_including_core_groups` | per window; `core_imm` not counted |
| F7 unpark | `engine::a_group_parked_on_a_head_that_goes_idle_latches` | no unpark |
| boot check clock | `kf-qemu display::the_boot_check_clock_ticks_only_while_it_runs` | a clock that runs unwatched |
| refresh + new watchers | `kf-qemu display::the_console_refresh_and_new_watchers` | an epoch bump on every request |
| the worker ticks only through the pacer | `kf-qemu display::the_worker_arms_vblanks_only_through_the_pacer` (a source scan with a known-positive) | a planted `next_vblank` |
| SURFACE on (w, h, rate) | `relay_machine.rs` `input_is_bounded_before_the_vmm_sees_it` | de-dup on size only |
| the seam | `wire_mirror.rs` (ABI 16, three new entries) | — |

⊘ **Not bitten, stated so nobody reads more into the table:** the worker's own wiring of D2 — which
copies start in which pass, `ScanState::completed`'s check-then-send decision, the on-demand serving
and the C device's async console — needs a display GPU context and a guest; the source scan sees
only that the calls exist. kf3.c compiles with `-fsyntax-only -Werror` against QEMU 10.2.4's headers
with the `system_ss` flags (the configured build there has no pixman: the two `PIXMAN_x2*10` formats
were predefined for the check; `82f98f42` gives the same two errors without them). The
`no /dev/udmabuf here` HELLO line the box run found was already fixed at `2b7751bc`
(`the_hello_line_claims_shared_memory_only_for_slots_without_a_dma_buf`; reverting the condition
turns it red).

**Bench pieces built for the runs below.** The realize log names the monitor the guest is told
about — `kf3: display: monitor 1920x1080 at 60000 mHz, range max 75 Hz, EDID fnv1a64=c9dcbb394c28b1c7
(display-max-fps 0)` unset — and every 3c re-author logs the new EDID's FNV too. `hook.sh` takes
`FPS_BOUND` (the run's cap, default 60): the deadlines of the fixed-frame vkcube runs scale by
`60 / FPS_BOUND` (a correctly paced vkcube under cap 30 must not read as `RC=124`), and each prints
`WALL_MS`. `kfdisp_probe list` prints each connector's `max_vrefresh` (below a 60 Hz cap no 60 Hz
mode may be offered); `kfdisp_probe show <card> <hold> <flips> async [gap_ms]` flips with
`DRM_MODE_PAGE_FLIP_ASYNC` (and prints `cap_async_page_flip`), and `hook.sh` runs it for R4 with
`DISPLAY_ASYNC=1` — 240 async flips back to back, then 60 with 50 ms between them. Both are compiled
here only (`gcc -Wall -Werror`, the DRM and the `KFDISP_NO_DRM` builds); neither has run in a guest.

**Candidate 2 verification update, 2026-10-04 (`9d82f259`, GA106, 580.159.04):**
R0 confirms the unchanged unset EDID hash/cap. R1 gives 29.938 Hz EDID, pixel-exact
KMS at 29.94 Hz, Wayland FIFO/IMMEDIATE/MAILBOX completion and zero cap overruns.
R1b's forced 60 Hz raster is clamped to 33333333 ns with GLX about 30 FPS.
D2 gives zero checks/copies while unwatched, fresh on-demand red/blue screenshots,
and idle VNC + broker at 59.9 unchanged checks/s with zero copies. HMP's previously
hung nested wait is fixed by completing refresh on QEMU's main AioContext; HMP/QMP
both return fresh full frames. Broker GPU-copy and host/VNC cursor parity pass.

**D4 is supported:** actual Vulkan FIFO returns measure **29.976857 FPS**, all 480
calls observed with zero errors; IMMEDIATE is 1557.8 FPS. The original 15.5–17.5 s
whole-process expectation below is superseded: variable loader/startup time made
it unsuitable for measuring presentation rate. Windowed/fullscreen/Cinnamon GLX
also hold about 30 FPS. X11 MAILBOX is unsupported on this stack; Wayland MAILBOX
works. Three Xorg-start flip-event warnings match candidate 1's existing handoff
limitation; the KMS flip workload has none. §M's correction is now folded in.

R4 passes the pacing checks: 240/240 async flips at 60.01 Hz, `tear=59.9`,
held=240 and zero overruns/timeouts/DRM warnings. The 50 ms-after-completion
control gives 60/60 at 18.36 Hz; held rises once to 241 at its initial flip,
then stays flat. The original literal "unchanged" prediction below is corrected
for that initial boundary; the steady slow path is not held. R6 passes at 30/50/60 Hz with constant 1920x1080 through
broker `9f2fd00` (refresh-only/reconnect fix), the real relay and actual guest.
Three duplicate hints are suppressed. A userspace connector reprobe (`modetest -c`)
refreshes the cached EDID; sysfs alone returned old bytes. The recorded udev monitor
has no DRM event, so this does not claim automatic desktop switching or a physical
host monitor transition; test-backend hints were used. Stopped-broker rendering now passes too: the broker stayed SIGSTOPped for 27.214 s,
while guest SSH answered and GLX reported 58.118–59.950 FPS. The empty first
six-second sample is superseded by this longer, PID-checked observation. Actual NVIDIA
DDX import refusal was not provoked; six unsafe descriptors are now rejected
before X and real GPU-copy passes. Evidence, failed probes and full caveats:
`traces/v3_candidates/cand2_20261004/README.md`.

**Historical unrun plan (superseded above where verified; remaining rows stay open):**
- R0 default: run 10's recipe unchanged; the realize line's `EDID fnv1a64=c9dcbb394c28b1c7`;
  `fps[cap=75(unset) … clamped=0 … over=0]`.
- R1 cap 30, KMS: `preferred=1920x1080@30`, the log's `period 33401904 ns`, `kfdisp_probe` flips at
  29–30.05 Hz, Wayland vkcube `--c 400` taking ≥ 13 s, `over=0` (the hook's timeouts at `hook.sh`
  scale by the cap first).
- R1b the clamp's falsifier: bare X with `AllowNonEdidModes, NoVertRefreshCheck` and a CEA 1080p60
  mode under cap 30 — `clamped=1`, glxgears ≤ 30.5.
- R4 async flips at cap 60 (`DISPLAY_ASYNC=1`): `ASYNC … flip_hz` ≤ 61 with `tear≈60` and
  `held>0`; `ASYNC_GAP50` ≈ 20 Hz with `held` unchanged.
- D2 on hardware: an idle KDE desktop with a VNC client shows `same` growing and `copies` near 0;
  nothing checked with the console and broker both idle; `screendump` pixel-exact (`hook.sh`'s
  pattern A) with nobody watching.
- **D4 (§M's X11 premise):** `display-max-fps=30` with `x11-dispsw=on` — windowed vsync glxgears
  28.5–30.5 and vkcube FIFO 480 frames in 15.5–17.5 s would show the cap bounds X11 through the
  guest's vblank consumer; ≥ 55 FPS refutes it. **This branch does not carry `x11-dispsw`
  (`v3-dispsw-exp`, ABI 13)**, so the run needs a merge (a new ABI number) first; §M's text changes
  only after it.
- R6 a refresh-only host change: needs the broker to re-send `SURFACE` when only the rate changes —
  nvkvm-pv's `nb_sink_surface` (`broker-cursor-gpucopy` at `badf2d7`) compares the size alone and
  drops it, so the relay's new de-dup is not exercised by that broker yet.

### 8.17 The interactive broker window on the trusted host (2026-10-08)

> ⊘ **SUPERSEDED IN PART 2026-10-08 (later), §8.18:** the "Defect found" paragraph's root cause is
> found and fixed in kf3 (not a broker race: a copy delayed 3.5 s by host RM was given up for good),
> and the launcher's 3 s broker delay is removed; "Cinnamon runs in fallback mode" was kayfabe's
> refusal of `NV906F_CTRL_GET_CLASS_ENGINEID` at 595.91.07, fixed; "the GPU-copy rung is not
> offered at host 595.91.07" was an unmeasured tag in `drmnv.tsv`, measured and offered. The text
> below is the record as first written.

**STATUS: RESEARCH — RUN ON THE TRUSTED HOST, 2026-10-08** (runs p1-p4, ab1-ab4, wl1; kf3 binary
`0e64a960`; broker nvkvm-pv `badf2d7`; RTX 4070, host 595.91.07, GNOME Wayland;
`traces/v3_display/broker_interactive_20261008/`). Nothing in kf3's code changed on this branch.

**The launcher.** `scripts/bench/display/interactive.sh` (as root, from ssh, while the desktop session
is logged in) finds seat0's active session, starts the broker as that user inside it (`--backend
wayland` on a Wayland session, else `x11`), and boots kf3 with OVMF, `gop=on`, `x11-dispsw=on`,
`display-broker=/run/user/<uid>/nvkvm/display.sock`, `display-broker-uid=<uid>`, a virtio keyboard, a
virtio tablet bound to `kf0` and a virtio mouse. `prep` builds the broker into `/opt/nvkvm-broker` from a
`git archive` of the pinned revision (the clone is not touched) and makes the guest disk (an overlay of
`guest.qcow2` with Cinnamon, lightdm autologin, an `xorg.conf` naming the NVIDIA DDX, a 10 s grub menu,
`nvidia-drm.modeset=1 fbdev=1`). `stop` powers the guest down. CTRL+ALT+G grabs/releases, CTRL+ALT+F
toggles fullscreen (the broker's hotkeys).

**Falsifiers, stated before run p4** (`input_proof.sh`'s header): ABS — the guest pointer more than 2 px
from the injected position scaled by the range; REL — the evdev REL sum on `QEMU Virtio Mouse` differs
from the injected sum, or the X pointer does not move by it under the flat profile; KEY — an injected
edge missing or extra on `QEMU Virtio Keyboard`; grub — no editor text after `e`.

**Measured, run p4 (2026-10-08, checkout `b435bbd9`, kf3 `0e64a960`)** — input through the REAL broker
(`badf2d7`, its display-less `test` backend scripted on stdin, so the broker's own focus gate, hotkey
chord and grab rules are on the path), the guest recording with evtest and xdotool:
- **grub**: a DOWN key from the broker stopped grub's countdown at 6 s and `e` opened the entry editor
  (serial text and `p4_grub_editor.png`). OVMF has no virtio-input driver; the keys reached grub through
  QEMU's routing to the PS/2 keyboard (the virtio keyboard is inactive until Linux binds it).
- **keys**: shift+a, ctrl+l, a — the ten edges arrive in order on `QEMU Virtio Keyboard`. The CTRL+ALT
  of the grab chord reach the guest and are released by the broker (`nb_release_all`); G never does.
- **absolute**: four positions and one after the grab: evtest's ABS_X is `x * 32767 / 1920` exactly
  (1706 for 100), the X pointer lands at `x - 1` (99 for 100; QEMU's `qemu_input_scale_axis` divides by
  the range, not range − 1). Graded FAIL by the script, which assumed the `--size 1280x720` window; the
  test backend's window had taken the guest's 1920x1080 from the relay's WINDOW, so the range was 1920 —
  against that range every position is within 1 px. The script is corrected; a window smaller than the
  guest frame was NOT run (the test backend cannot hold one; the restart meant to try was broken).
- **relative**: CTRL+ALT+G → `pointing device -> #5 QEMU Virtio Mouse (relative)`; 16 REL packets
  summing (70, 30) → evdev REL_X 70, REL_Y 30 on the mouse; the X pointer moved (639,299) → (709,329);
  an ABS while grabbed was dropped (pointer unchanged); release → `#4 QEMU Virtio Tablet (absolute)`.
- **buttons, wheel**: BTN_LEFT and a BTN_RIGHT under grab arrive on the TABLET device (the handler bound
  to `kf0` takes buttons in both modes); wheel detents too.
- **unload** (`modprobe -r nvidia_drm nvidia_modeset nvidia`, rc 0, after stopping lightdm): the window
  goes black (`p4_after_unload_15s.png`: every pixel 0) and stays so — no console comes back, as §4.11.13
  records for bare metal on Linux 7.x; not chased.
- **two clean reboots in one QEMU, broker connected**: QEMU alive, grub and the login prompt again, 0 Xid,
  no `RmInitAdapter failed`, no WPR text in the guest dmesg, `nvidia-smi -L` answers, and the X desktop
  is back (`p4_reboot2_after.png`).
- **NV0073 controls** (`KF3_RPC_TRACE=1`, three boots): the guest sent 22 different NV0073 controls to
  kf3's GSP (`p4_nv0073_rpc_census.txt`); `SYSTEM_GET_CONNECTOR_TABLE` (0x73011d), `SYSTEM_GET_HOTPLUG_CONFIG`
  (0x730109) and `DP_GET_CAPS` (0x731369) are NOT among them — this Linux 580.159.04 guest did not call
  them over the RPC. kf3 does not claim them (`kf-rm` `display.rs`
  `unsupported_windows_controls_never_get_a_success_reply`), so a caller gets the normal refusal
  (inferred from that test; not run here).

**Measured, the real Wayland broker (run wl1, 2026-10-08 11:42, the owner watching):** the window maps
on GNOME (mutter offers no server-side decorations; the broker draws its own 28 px title bar; output
scale 1.5). The GPU-copy rung is not offered at host 595.91.07 (`the nvidia-drm ABI is not measured`).
The relay's LINEAR udmabuf was advertised by mutter and then refused at import, so the broker presents
through **wl_shm** (`presenting through wl_shm …`) — the shm rung is the only one that ran on this stack.
Status at the snapshot: `sent=167 releases=165 reclaims=2 dmabuf_trips=0`. The owner reports the guest's
NVIDIA desktop and its cursor in the window. Wayland-backend input was the owner's own hand test; it was
not automated (injecting into the owner's live session was not done).

**Defect found, measured (ab1-ab4, p1, p3; kf3 `0e64a960`):** with `gop=on`, kf3's FIRST scanout copy,
when it is requested within about 2 s of QEMU starting, never completes (`scanout REFUSED copy 1 did not
complete in 2s`); the worker then sets `failed` and the display is dead for the VM's life (the guest's
nvidia-modeset then times out on its core channel, `Error while waiting for GPU progress … c77d`). Who
requests it does not matter: a broker already listening when QEMU starts (ab D, E — also what a QEMU
restart against a still-running broker does) or a console readback at +1 s (p3). Not seen: no broker
property (ab B), the property with no broker (ab G), no `gop` (ab A, C, F), a broker connecting at +3 s,
+8 s, +20 s (ab I, J, H), p2/p4 (broker at +8 s). **Root cause not established.** Workaround in the
launcher: the broker starts once the worker is up plus 3 s. A QEMU restart that reattaches to a broker
left running was not measured separately; by ab D it connects at once and hits this stall.

**Measured: Cinnamon runs in fallback mode on this host** (p4, wl1). Xorg: `(EE) NVIDIA(0): Failed to
allocate display software resources`; `cinnamon` segfaults in `libnvidia-glcore.so.580.159.04`. kf3:
`display-SW twin REFUSED (0x56): GF100_DISP_SW … its software classID was not readable (Other(19314))` —
the `x11-dispsw` twin's host read is refused at host 595.91.07, where the box run of 2026-10-03 (host
580.159.04) had it working. Without the `xorg.conf`, X chose modesetting and failed outright (`modeset(0):
Failed to create pixmap`, run p2). Next: measure that host control's layout at 595.91.07.

**nvkvm-pv's relay against kf3's, input only** (`src/qemu/nvkvm_display_relay.c` at `9bc7d7f`, the
input code is the same at `badf2d7`): KEY — nvkvm-pv calls `qemu_input_event_send_key_linux` (QEMU 11.1),
kf3 `qemu_input_event_send_key_qcode` (10.2), the same map lookup as the filter: equivalent. BTN, ABS,
WHEEL, GRAB (`qmp_query_mice` + `qemu_mouse_set`, Virtio preferred): the same calls. REL — kf3 sums
consecutive REL packets of one read batch (at most 64) into one event; same sum, fewer sync points
(measured equal sums above). ABS — kf3 additionally clamps to `[0, w-1]` and caps the range at 2^20 in
Rust. Verdict: no input difference explains a relative-mode failure, and none was measured. nvkvm-pv's own
relay was not run as a control (it needs its QEMU build).

**Hypervisor-agnostic split — where policy still sits in `kf3.c`** (the owner's rule of 2026-10-08:
input policy in `kf-broker` behind a VMM-neutral trait, only the API shim in the VMM): (1) the choice of
pointing device on GRAB/ungrab (`kf3_broker_set_relative`: absolute vs relative, prefer a name containing
"Virtio", else the first); (2) the forwarded button set (`kf3_broker_btn`: LEFT/RIGHT/MIDDLE/SIDE/EXTRA);
(3) the absent-tablet warning (`kf3_broker_check_pointer`); (4) the wheel as a press-release pair and
CLOSE force→shutdown / else→powerdown. Already in Rust and VMM-neutral: the evdev range, ABS clamp and
range cap, REL summing, wheel direction, SURFACE clamp and de-duplication. Proposed, NOT built (it changes
the KF3 ABI and needs a kf3 rebuild and a box run): a `kf_broker::InputSink` trait — `key(code, down)`,
`button(Button, down)`, `abs(x, y, w, h)`, `rel(dx, dy)`, `sync()`, `pointers() -> Vec<Pointer {index,
absolute, name}>`, `select_pointer(index)`, `powerdown(force)` — with the device choice, button set and
warning as Rust functions over it (tests with a fake sink: the GRAB choice, the summing, the clamp), and
kf3.c reduced to the eight calls.

**Owner decisions:** (a) the first-copy stall — fix in kf3 (a box hunt) before the launcher's delay is
removed; (b) the `x11-dispsw` host read at 595.91.07; (c) whether the `InputSink` refactor goes ahead.

### 8.18 The early-frame freeze, `x11-dispsw` at 595.91.07, and the buffer path per environment (2026-10-08, later)

**STATUS: RESEARCH — RUN ON THE TRUSTED HOST, 2026-10-08** (kf3 `2a20e699`, built by `build_kf3.sh`;
before: `0e64a960`; broker nvkvm-pv `badf2d7`; RTX 4070, host 595.91.07; runs e1-*, wl2, wl3, p5, xvfb;
`traces/v3_display/broker_interactive_20261008/`).

**1. The early-frame freeze — root cause and fix.** Falsifiers stated before each run
(`scripts/bench/display/early_frame.sh`, kf3 `0e64a960`, `KF3_DISPLAY_TRACE=1`):
- e1-a (broker listening before QEMU): H "the GPU never completes copy 1" predicts no
  `TRACE scanout copy 1 done` before the give-up — none came; the give-up followed the VA manager's
  `mem t=3.710s [1/3] prewarm … ram_obj 3528088 us` (host RM registering the 8 GiB guest-RAM memfd).
- e1-b (no `display-broker` at all, console readback at +1 s): H "the broker's memfd/registered frames
  cause it" predicts LIVE — STALL. Falsified: any early copy stalls.
- e1-c (broker listening, QEMU `-S`, `cont` at +6 s): STALL — the guest's firmware is not involved.
- e1-d ×2 (as e1-a with `-m 2048`): H "the copy waits behind the prewarm and is given up at 2 s"
  predicts LIVE, since the registration takes 0.88 s there — LIVE 2/2 (`ram_obj 880778 us`).

**The broken invariant.** The worker treated "no completion signal within `STUCK_COPY` (2 s)" as a
LOST completion: it dropped the in-flight copy, set `failed` for the VM's life and never served the
copy's barrier, so no frame was published again and the core-channel completions queued behind that
barrier never ran (the guest's nvidia-modeset then times out, `Error while waiting for GPU progress`).
But the signal was only LATE: the display stream's work was queued behind host RM's 3.5 s
registration of the guest's RAM (which side waits: the display worker on its stream's
`cuLaunchHostFunc` eventfd; whose event is late: that host function's, held up by the host RM, not
lost). A timeout is evidence of neither completion nor loss. **Fix** (`kf-qemu` `display.rs`
`give_up_if_stuck`/`stuck_verdict`, `kf-cuda` `DisplayGpu::signal_state`): every completion signal
now records an event; a copy past 2 s is given up only when `cuEventQuery` reports a stream FAILURE
(or there is no GPU context to ask); while it reports "not ready"/"done", the worker waits for the
real signal, forges nothing, and logs one line (`copy 1 has not completed in 2s and its stream reports
no failure … waiting`). GPU-free test: `a_late_copy_behind_a_busy_host_rm_is_waited_for_not_given_up`
(the interleaving's timings; known-positive: the old rule loses it at 2.1 s). The launcher's 3 s broker
delay is removed: the broker starts first, and a running broker is reused.

**Measured after the fix (`2a20e699`, 2026-10-08):** broker listening at QEMU start (0 s) LIVE 3/3;
console readback at +1 s without a broker LIVE 3/3; QEMU restarted three times against one live broker
(its 4 attaches) LIVE 3/3; broker listening + `-S` + `cont` at +6 s LIVE 1/1; each with the waiting line
and then `TRACE scanout copy 1 done` after the prewarm (`ram_obj 2960324 us`). Before (`0e64a960`):
STALL in all 7 early cases (ab D, E; p1; p3; e1 a, b, c). The interactive launcher, broker first, ran
to the desktop three times (wl2, p5, demo-final). Not measured: a stream that really fails (the `Lost`
branch is GPU-free-tested only).

**2. `x11-dispsw` at host 595.91.07 — kayfabe's defect, fixed.** The refused read: kf-host's twin
readback `NV906F_CTRL_GET_CLASS_ENGINEID` (`0x906f0101`, `ctrl906f.h:94-103`), status `Other(19314)`
= `HOST_ABI_REFUSED` (`0x4B72`) — never sent: the control had no `kf_abi::hostabi::HOST_CONTROLS` row,
and an unlisted control is refused outside `[580.65.06, 581)`. Derived, not captured: the driver
matrix now measures `NV906F_CTRL_GET_CLASS_ENGINEID_PARAMS` (one 16-byte layout at all 30 tags
535.309.01 … 615.71.09) and the id (`host_chan_cmds`, `tools/drivermatrix/host.spec`; regen
2026-10-08, +6 rows in `ranges.tsv`), and the row carries it at 580 and 595 (test
`the_display_sw_readback_is_carried_at_595_and_580`). **Measured (wl2, `2a20e699`):** 8 of 8 guest
GF100_DISP_SW twinned, `software classID n = the guest's [kept]`, 0 refused; Xorg has no `Failed to
allocate display software resources`; `cinnamon --replace` runs, no segfault, `glxinfo`: NVIDIA GeForce
RTX 4070, direct rendering; the frame shows the normal Cinnamon desktop (`wl2_desktop.png`), no
fallback dialog. Same in p5 and demo-final. Normal, for comparison: the 2026-10-04 box (RTX 3060, host
580.159.04) had the twins and an X desktop (`traces/v3_display/broker_20261004/`). Left: one
`Flip event timeout on head 0` at guest 7.5 s in each boot (also before the fix; not chased).

**3. The buffer path per environment.** kayfabe's order: rung 0 (a GPU copy into kayfabe-owned VRAM,
block-linear, offered only after an explicit yes for the pair and `CAP_MODIFIERS`+`CAP_RELEASE`, and
not for a compositor on another GPU) → rung 1 LINEAR udmabuf (unknown verdict counts as yes) / rung 1b
implicit modifier → F_SHM, with every "no" final for the connection and unacknowledged dma-buf commits
backing off to shm. Why rung 0 was not offered at 595.91.07: `traces/driver_matrix/drmnv.tsv` had no
595.91.07 rows (the tag was added to `tags.txt` after the 2026-10-03 probe); `drmnv.py --tags
595.91.07` (2026-10-08) measured every item equal to 595.84's, so `kf_abi::drmnv` offers it now.

| environment | native NVIDIA | LINEAR dma-buf | shm | how |
|---|---|---|---|---|
| this host: GNOME Wayland on the NVIDIA GPU | YES — **measured** wl2: `gpucopy=169` of `sent=170`, `the display imported a GPU-copy frame`; one 5 s back-off trip at start (3 early commits unreleased), then native | advertised, refused at import — **measured** (`the display CANNOT show XR24 … 0x0`) | **measured** wl3 (`display-broker-vram=off`): `presenting through wl_shm`, `sent=398 releases=396`, desktop up | GPU-free: `env_native_display_ends_native_and_falls_back_to_shm_never_linear_again` |
| laptop: compositor on an Intel iGPU, NVIDIA dGPU without display | NO | YES | YES | **MODELLED BY TEST ONLY** (no such hardware): `env_other_gpu_compositor_ends_linear_and_falls_back_to_shm` |
| Xvfb / headless llvmpipe | NO | NO | YES | **measured** for Xvfb (xvfb1): the X11 broker `has no DRI3 … descending to the shm tier`, the relay `frames go as shared memory`, `180 attach, 180 commit, 0 rejected`, `releases=180`; llvmpipe modelled by `env_software_display_ends_on_shm` |

A refusal of an advertised path leads to the next path in every test and run above; none stalled.

**nvkvm-pv's selection (reference, `badf2d7`: `src/qemu/nvkvm_isolate_handlers.c:2110-2140`,
`nvkvm_present_egl.c:1630-1700`, `nvkvm_display_relay.c:640-660,1380-1420`) against kayfabe's**:
(1) nvkvm-pv's native path exports the GUEST's own buffer; kayfabe GPU-copies into its own VRAM
object (`OWNER_RULINGS.md` §L) — one GPU copy more, nothing of the guest's exported. (2) nvkvm-pv goes
native OPTIMISTICALLY while the verdict is unknown; kayfabe needs an explicit yes (the first frames
go LINEAR/shm). (3) nvkvm-pv's LINEAR is a readback by the guest's GPU after a native "no"; kayfabe's
host-RAM udmabuf is filled by the same D2H copy as the console. (4) shm after a LINEAR "no": the same
pages as a memfd in both. (5) Verdicts: both per connection, unknown → answer, yes → no allowed, a no
stays no (`relay_format_verdict_next`; kayfabe also records `badf2d7`'s unsolicited `x=0`). (6) kayfabe
adds acknowledgement detectors (no RELEASE → back off to shm 5 s, doubling) and the EV_DEVICE
other-GPU check; nvkvm-pv relies on the broker's `x=0` (read from the source, not run). (7) kayfabe
has rung 1b (implicit modifier) for a broker without `CAP_MODIFIERS`. None of these makes a refused
path stall.
