# V3 display — a virtual NVIDIA display the stock driver drives, scanned out by kayfabe

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
> - X11 desktops need `GF100_DISP_SW` (owner choice A/B, `STATUS_AND_HANDOFF.md` §0).
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
>     move it into VRAM before the NVIDIA driver touches it through the GPU.
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
>   X11 still waits on the `GF100_DISP_SW` choice (A/B; recommendation B).

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
>   display object, which the rule forbids.
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
- Completion notifiers and semaphores are written by the worker into the same spans. A write is only ever
  made **after** the thing it reports has happened (§4.5).

### 4.5 Timing: vblank, flip latch and completion — all host events

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
   composition's planned layers on the monitor until a head is armed again. ⊘ *CORRECTED 2026-10-03
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
shared between the worker and the main loop. Nothing new runs on a vCPU.

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

**Reset:** kf3 has no reset path — a guest reboot needs a QEMU restart (owner default). The broker
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

