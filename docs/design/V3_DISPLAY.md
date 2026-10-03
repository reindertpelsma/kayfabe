# V3 display — a virtual NVIDIA display the stock driver drives, scanned out by kayfabe

> **STATUS 2026-10-03 — display step 1 (the boot display's GOP option ROM): first half BUILT on branch
> `v3-gop-rom` — the firmware, the ROM container and a local stand-in test (§4.11, its own STATUS
> line). The kf3 integration is the second half. The ⊘ notes below dated 2026-10-03 fold that design's
> corrections into the older text.**

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
> the host driver's dma-buf exporter, and inject the broker's input through the VMM's input device (the
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
>   - Keep at least two copy targets, and reuse one only after the broker's `RELEASE`.
>   - A `SURFACE` resize sends a new EDID plus a hotplug event (§4.7).
>   - nvkvm-pv is Apache-2.0 and the owner's, so its code can come under kayfabe's dual license.
> - **The GOP framebuffer must lie inside a kf3 BAR.**
>   - Linux ties the firmware framebuffer to the PCI device whose BAR contains it. That device's
>     driver evicts it (`aperture_remove_conflicting_pci_devices`, `drivers/video/aperture.c`).
>   - ⊘ *CORRECTED 2026-10-03:* the next sub-bullet's last sentence. Xorg is not missing a boot VGA
>     device on today's SeaBIOS bench (the correction at the top). The firmware-default rule matters
>     under OVMF and with a second VGA device; the local stand-in shows it overriding a legacy device
>     (2026-10-03, `da5cc07f`, arm `linux_two_vga`, §4.11.8).
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
>   only when the compositor draws it), scaled windows (shown unscaled, clipped), YUV/16-bit layers.
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
- **Later:** export the latched range of the store as a dma-buf with its NVIDIA modifier and hand it to a GL
  UI (`dpy_gl_scanout_dmabuf`, nvkvm-pv's GL zero-copy path), or to nvkvm-pv's broker protocol.
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

**STATUS: FIRST HALF BUILT, 2026-10-03 (branch `v3-gop-rom`).** Built and tested without a GPU: the
GOP firmware (`firmware/kf-gop`), the ROM container and packer (`crates/kf-oprom`), and the local
stand-in (`scripts/display/gop_standin.sh`, 11/11 arms at `da5cc07f`, §4.11.8). **Not built:**
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
  (*"setting as boot VGA device (overriding previous)"*; 2026-10-03, `da5cc07f`, arm `linux_two_vga`).
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
  resource was exactly 4608 × 648 = 2 985 984 bytes at BAR + 0 (2026-10-03, `da5cc07f`, arm `linux`).
- **Order and passthrough.** `RmSetConsolePreservationParams` runs before `kgspInitRm` (`osinit.c:1993-1995`
  vs `:2024`); `GspSystemInfo.consoleMemSize` (`src/nvidia/src/kernel/vgpu/rpc.c:10585`) reaches us in
  fn 72, which arrives **before** fn 1 and fn 65 (`kernel_gsp.c:4141` vs `:4225`, `:4232`). Under KVM the
  guest is "passthrough" (`gpu.c:4711-4774`), so `RmDeterminePrimaryDevice` returns early
  (`osinit.c:983-991`): console preservation still runs (`:1031-1052`), and console access is **not**
  disabled during GSP/BAR1 setup (`:2003-2007`) — firmware-console writes keep arriving through BAR1.
  The display fuse already reads ENABLE wherever `display=on` works (`kern_disp.c:333`).

| item | value | why |
|---|---|---|
| BAR | PCI BAR1 (`kf3.c:759-760`) | the equality test above; Linux ties the firmware framebuffer to the BAR that contains it (§4.11.8) |
| offset | 0 | `osinit.c:1079`; `kern_bus_gm107.c:1131-1153` |
| size G | `align_up(align_up(4W,256)·H, 64 KiB)` | `osinit.c:1092`; `nvkms-rm.c:4880-4884` |
| backing at boot | store `[0,G)` **zeroed by a GPU write** (the verb that zeroes the roots, `crates/kf-qemu/src/device.rs:337-345`), then one host CPU view of store `[0,G)` placed at BAR1 offset 0 at realize, before any vCPU runs (`WindowOps::arm_store` + `place_view`, `crates/kf-qemu/src/mem.rs:344-357`) | real VRAM, the pages RM will call FB 0; no copy at handover; no exits; nothing from before the VM visible (kayfabe never scrubs the store otherwise, `crates/kf-host/src/lib.rs:1672-1730`) |
| refused | host-RAM backing | sysmem standing in for vidmem (`THE_CONSTRAINTS.md:120-125`, items 18, 22); breaks FB-0 identity; needs an owner ruling (`OWNER_RULINGS.md` A.11), never a fallback |

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
- **No hard-coded chip** (§12, A.7): the PCIR ids and class come from the host identity (`kf3.c:737-745`);
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

**The firmware, as built** (`firmware/kf-gop`, zero external crates, `no_std`, `x86_64-unknown-uefi`;
release `kf-gop.efi` 9 216 bytes at `da5cc07f`):
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
- Every decision is in the safe library (`src/lib.rs`, `forbid(unsafe_code)`, 14 host tests); the 77
  relaxations sit in four `*_unsafe.rs` files, counted by the CI ratchet (`firmware/kf-gop:77`); a
  `#[repr(C)]` under `firmware/` outside `*_unsafe.rs` fails the ABI-quarantine gate.

**The container, as built** (`crates/kf-oprom`: pure, `no_std` + `alloc`, `forbid(unsafe_code)`):

| offset | content |
|---|---|
| 0x00 | `55 AA`, `InitializationSize`, `EfiSignature 0x0EF1`, `EfiSubsystem 0x0B`, `EfiMachineType 0x8664`, `CompressionType 0`, reserved, `EfiImageHeaderOffset` (0x16), `PcirOffset 0x1C` (0x18) |
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
  tell the two layouts apart; the arm records that.
- `parse` accepts PCIR revision 0 and 3 and reports compression: QEMU's own `efi-virtio.rom` (read
  2026-10-03) has two images, the EFI one with PCIR rev 0, length 0x18, compression 1, header offset
  0x38 — a golden test (`crates/kf-oprom/tests/efi_virtio_golden.rs`).
- `pack` refuses by name: a PE that is not PE32+/x86-64/subsystem 11 or lacks the ABI marker
  `KFGOP-ABI=1` (so a VMM never packs a descriptor version its firmware would refuse); a non-display
  class; a bad geometry (`pitch % 256`, `G ≥ pitch·H`, dimensions ≤ 16384); an EDID over 256 bytes; a ROM
  over 0xFFFF blocks.

**The API the kf3 integration uses** (`crates/kf-oprom`, re-exported at the crate root):
`Geometry::for_mode(W, H)` → `BootFramebuffer { bar: 1, offset: 0, geometry }`;
`pack(pe, &Identity { vendor, device, class: ClassCode::from_u24(class) }, &fb, edid) -> Result<Vec<u8>,
PackError>`; `BootFramebuffer::fits(bar_len)`; `pe::check(pe)` for a realize-time refusal by name;
`Descriptor::find(rom)` and `rom::parse` for tests. The geometry and EDID come from kf-disp's `Monitor`
(`crates/kf-disp/src/edid.rs:145-175`, 128-byte EDID).

**How kf3 serves it** (design, for the integration):
1. C finds the PE with `qemu_find_file(QEMU_FILE_TYPE_BIOS, "kf3-gop.efi")`; `-L` or a `gop-image=`
   property overrides it.
2. C passes the bytes to a new `kf3_option_rom()` (**KF3_ABI 10 → 11**, `qemu/hw/misc/kf3/kf3.h:9-14`,
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
- **Install** (`V3_SWEEP_AND_INSTALL.md`): the tarball ships the PE, `kf3-gop.efi`, with its sha256;
  kf3 wraps it per host.

#### 4.11.7 EDK2 behaviour: what the source says, and what the stand-in showed

Read in edk2-stable202408 (QEMU 10.2.4's `roms/edk2`); shown by the stand-in on 2026-10-03 at
`da5cc07f` on Ubuntu `ovmf 2025.11-3ubuntu7` and QEMU's `edk2-x86_64-code.fd`, and at `2c6178fe` in CI
run 37127211692 on Ubuntu `ovmf 2024.02-2ubuntu0.9`.

| behaviour | read in the source | stand-in, 2026-10-03 (`da5cc07f`; CI run 37127211692) |
|---|---|---|
| The bus driver copies the ROM into `RomImage` before running its driver | `PciDeviceSupport.c:239-333` (`ProcessOpRomImage` inside `RegisterPciDevice`) | kf-gop decodes the descriptor from `RomImage` in `Supported`, all three builds |
| Only an EFI boot-service or runtime driver is loaded from a ROM | `PciOptionRomSupport.c:76-80`, `:701` | `build.rs`'s `/subsystem:efi_boot_service_driver` is honoured by lld-link: subsystem 11 (`kf-oprom pe`) |
| `LoadImage` gets `[EfiImageHeaderOffset, InitializationSize·512)` | `PciOptionRomSupport.c:85-90` | the payload is the PE byte for byte (unit test), and it loads |
| **ROM drivers are deferred until after End-of-DXE, and OVMF connects consoles twice** — before dispatching them and again after (*"GPU passthrough only allows Console enablement after ROM image load"*) | `OvmfPkg/Library/PlatformBootManagerLib/BdsPlatform.c:470-503` | QEMU's DEBUG edk2: *"3rd party image[0] is deferred to load before EndOfDxe"*. ⇒ A device OVMF has a built-in driver for is taken by it first: stdvga + a kf-gop ROM gets QemuVideoDxe (30 modes), kf-gop loads and finds the device owned (arm `stdvga_builtin_wins`). NVIDIA has no built-in OVMF driver, so kf3 is in `ati-vga`'s position, where kf-gop binds |
| A single-mode GOP drives the text console | — | *"GraphicsConsole video resolution 1152 x 648"*, *"Graphics Console Started"* (QEMU edk2 log) |
| **Option ROMs are not signature-checked, Secure Boot or not** | `PcdOptionRomImageVerificationPolicy\|0x00` (always trust) in `[PcdsDynamicDefault]`, `OvmfPkg/OvmfPkgX64.dsc:689`; set to 0x04 (deny) only under AMD SEV (`OvmfPkg/PlatformPei/AmdSev.c:468`) | with Secure Boot enforcing — the unsigned boot app is *"Access Denied -- rejected probably by Secure Boot"* in the same boot — the unsigned ROM runs (Microsoft-keyed and snakeoil VARS); the snakeoil-signed ROM and the tail-padded signed ROM run too (arms `sb_*`) |

#### 4.11.8 The local stand-in

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

**Result, 2026-10-03, `da5cc07f`, clean tree: 11/11 PASS** (`traces/v3_display/gop_standin_20261003/`):

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
takeover; replaced by the `/dev/fb0` pattern at `da5cc07f`).

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
- **B0** (OVMF, `gop=off`): the lane with `-drive if=pflash` CODE + a per-run VARS copy; boot, nvidia.ko,
  the M1/M2 probe pixel-exact; where OVMF places kf3's 64-bit BARs; `boot_vga` without a GOP; whether the
  noble image has an ESP. In the local `linux_two_vga` arm Linux read `command=0x0007` (I/O + memory +
  bus master) on a driverless VGA device at init; whether OVMF or Linux set it is not established, so
  B0 checks kf3's decode without a GOP directly.
- **B1** (`gop=on`): the screendump shows OVMF, GRUB and kernel text from the first seconds; simpledrm or
  efifb on BAR1 with BOOTFB at BAR1 + 0; kf-disp boot counters.
- **B2** (RM adoption): fn 72 C == `align64K(H·pitch)` ≤ G; fn 65 has three regions; the first K_BAR1 walk
  is **one run** VA 0 → store 0, length C; the seed retired in that batch; no LEVEL_ERROR *"cannot
  preserve console mapping"*; nvidia-smi works; no black frames across the module load.
- **B3** (`modeset=1 fbdev=1`): eviction, `NOTIFY_CONSOLE_DISABLED`, `boot_done`, no black gap.
- **B4**: Xorg with no `xorg.conf`, plus Wayland; the M3 probes pass.
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
1. **Secure Boot:** documented **off** for the boot display; no signing infrastructure yet. ⊘ The
   question's premise ("an unsigned ROM will not run under Secure Boot-enforcing OVMF") does not hold
   for OVMF: it trusts option ROMs unconditionally except under AMD SEV (§4.11.7). Physical-machine-style
   firmware with a deny policy would need option (b) (a kayfabe key in a VARS template; the PE is
   constant per release, so one signature) or (c) (Microsoft third-party CA).
2. **§13:** `firmware/` is a named unsafe exception, outside the cargo workspace, never linked into the
   VMM (`firmware/README.md`; CI gate B, the ratchet and the ABI-quarantine firmware arm).
3. **Firmware for bench lanes:** OVMF only for the display lane; SeaBIOS lanes stay the measured
   baseline. No legacy VGA BIOS for the release (a code-type-0 VBE image is 1–2+ weeks).
4. **Warm reboot:** kf3 has no reset path (`kf3.c:900-905`): a guest reboot needs a QEMU restart
   (`-no-reboot`). Windows Setup's reboots go through restarts (B10).
5. **No host-RAM backing** for the boot framebuffer (§4.11.2).

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
