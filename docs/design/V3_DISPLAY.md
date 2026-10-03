# V3 display — a virtual NVIDIA display the stock driver drives, scanned out by kayfabe

> **STATUS: BROKER RELAY AND CURSOR COMPOSITION BUILT, GPU-FREE — 2026-10-03 (branch `v3-broker`;
> nothing run on a box).** Display step 3 ("the broker") sub-steps 3a (frames + input), 3b (reconnect,
> pacing) and 3d (the head's cursor composed as the top layer) are in code: the VMM-agnostic crate
> `kf-broker`, its OS doors in `kf-linux-raw`, the kf3 glue (KF3 ABI 11, properties `display-broker` /
> `display-broker-uid`), and `kf_disp::engine::CursorScan` + `kf_disp::scanout::plan_cursor`. With
> `display-broker` unset nothing changes for the broker; the cursor layer applies to the console too.
> 3c (resize) is NOT built. Design, deviations, local runs and the pending box tests: **§8**.

> **NEXT — owner direction 2026-10-01 (design only, nothing built): (1) a STOCK guest display with no
> guest-side tweaks, then (2) a VMM-agnostic display BROKER instead of QEMU's UI — both before Windows.**
> What the M1–M3 lanes still change inside the guest (`scripts/bench/display/`), and what each needs:
> - `modprobe nvidia-drm modeset=1 fbdev=1` by hand after boot → test a packaged driver loading at boot
>   (Ubuntu's `modprobe.d` already sets `modeset=1`); the bench loads the driver over ssh.
> - X11's `xorg.conf` pins the BusID: the VM runs `-vga none` and kf3 shows no firmware framebuffer, so no
>   device is `boot_vga` and Xorg cannot choose (Wayland enumerates DRM and needs no pin) → **kf3 must be
>   the boot display**: a UEFI GOP (and a legacy VGA path) over a kf-disp linear framebuffer, handed over
>   to the NVIDIA driver when it loads. ★ Windows needs this regardless: its installer, boot and safe mode
>   run on the GOP framebuffer before `nvlddmkm` starts. The largest item (~1–2 weeks, estimate).
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
>   - Optional, only if boot is found to be slow: back the GOP region with host RAM during boot, and
>     move it into VRAM before the NVIDIA driver touches it through the GPU.
> - **Driver unload.**
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
>   1. the GOP option ROM;
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

**STATUS: LIVE — 3a, 3b and 3d BUILT IN CODE, GPU-FREE, 2026-10-03 (branch `v3-broker`).** Local
runs are §8.8; nothing has run on a box (renting needs the owner's approval). 3c is not built (§8.6). This section folds in the reviewed design (the adversarial review of 2026-10-03 applied);
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
| frame backing (memfd + `cuMemHostRegister` + udmabuf), the relay seat, KF3 ABI 11 | `crates/kf-qemu/src/broker.rs`, `display.rs`, `ffi_unsafe.rs`; `crates/kf-cuda` (registration) |
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
backlog, a failed attempt — `kf-linux-raw` `a_full_backlog_is_eagain_and_never_in_progress`), and at
most 64 packets per call with the tail left in the socket
(`at_most_64_packets_per_call_and_the_tail_stays_in_the_socket`).

### 8.2 Frames and the rungs

With `display-broker` **unset**, frames stay `cuMemAllocHost` and there are 3 slots — the console path
is unchanged. With it **set**, each of **5** slots is a sealed memfd (`kayfabe-display-frame`, sealed
`SHRINK|GROW|SEAL`, never `WRITE`), mapped and page-locked with `cuMemHostRegister_v2` so the existing
asynchronous D2H scanout copy lands in it, plus a udmabuf over the same pages when `/dev/udmabuf` opens
(`root:kvm 0660`). The frame is registered **before** its descriptors enter the ring, so the ring never
names a backing the GPU does not write. A refusal (no `cuMemHostRegister`, or a driver that will not
pin these pages) is logged once by name; the console keeps its own frames and the broker is shown
nothing — a CPU copy is never the fallback. The descriptor: `XR24`, `offset 0`, `stride = width × 4`,
`LINEAR` or `MOD_INVALID`.

1. **LINEAR dma-buf** — udmabuf present, `CAP_MODIFIERS` set, verdict for (XR24, LINEAR) not "no" (an
   unknown verdict counts as yes).
1b. **Implicit-modifier dma-buf** — udmabuf present, `CAP_MODIFIERS` clear, and an explicit yes to
   `QUERY_FORMAT(XR24, MOD_INVALID)`. ⚠ That importers treat an implicit-modifier udmabuf as linear is
   a box question.
2. **F_SHM memfd** — always available; an X11 broker takes it only with `--present-mode=shm`, and a
   refusal is not reported back (the relay's log says so once).

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
`free_slot()` returns `Option`; `None` is counted (`scanout_no_slot`), never papered over with slot 0.
Descriptors are never closed while the device lives (retire-never-free, per slot at most two backing
generations in `OnceLock`s), so a recycled descriptor number can never be sent. RELEASE is matched only
against the id of the descriptor actually sent on that frame's rung (memfd and dma-buf inodes come from
different counters); identities are checked distinct across every slot and generation at install.

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
| CLOSE | FORCE → `qemu_system_shutdown_request(SHUTDOWN_CAUSE_HOST_UI)`; otherwise `qemu_system_powerdown_request()`, with nvkvm-pv's repeat-ask message |
| SURFACE | 3a/3b: logged; the broker scales (no `.ui_info` hook before 3c) |
| FOCUS / BYE | logged; POINTER, HELLO, CLIPBOARD ignored (no clipboard: CAPS bit 0 is clear) |
| RELEASE / FRAME / FORMAT | consumed by the relay |

### 8.5 Security

The broker stays a separate process holding the display-server connection and the grab; QEMU holds one
socket and imports nothing. The relay never binds, chmods or unlinks. **Peer check right after
`connect`, before a byte is read** (`SO_PEERCRED` is fixed at connect): accepted uids are 0, QEMU's
effective uid, `display-broker-uid` when set, and the **owner of the socket's directory** (owner question
1, implemented as the recommended default: that uid already decides who can create the path, so it
admits no squatter). A refusal is loud and retried with backoff. Events are read with `recvmsg` and **no
control buffer**: a descriptor the peer attaches is dropped by the kernel and `MSG_CTRUNC` is a protocol
violation. Outbound descriptors are dedicated, sealed frame copies — never guest RAM or the store — and
are never closed while the device lives. No clipboard (owner default).

### 8.6 Cursor composition (3d), and what is not built

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

**Not built:**
- **3c, resize** — not built: no `.ui_info` hook, no authored-on-request EDID, no hotplug. Its design
  (claim `NV0073_CTRL_CMD_INTERNAL_GET_HOTPLUG_UNPLUG_STATE` 0x730401 — not the public 0x73012d, which
  kernel RM serves itself; a narrow `NV01_EVENT_KERNEL_CALLBACK_EX` HOTPLUG registration in the kf-rm
  display link with `osevent` untouched; a `bNotifyList` post with the bare `notifyIndex = 1`, posted by
  the register drainer; the registration retired on FREE and on GSP re-init) stands as reviewed.
- Zero-copy export of the guest's surface with NVIDIA's modifier (later, as before).
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
5. **The effective uid comes from `/proc/self/status`** (safe code) rather than `geteuid` (one more
   unsafe relaxation).
6. **No `kf3_display_ui_info` entry yet** — it belongs to 3c.
7. **The loopback's SIGSTOP case grades "never blocks"** (every relay call returned within 50 ms, frames
   waited or were reclaimed) rather than owed/dropped counts: pacing commits at most ~10 frames/s to a
   stopped broker, which does not fill a socket buffer in 5 s. The owed/dropped paths are covered by the
   scripted-link tests (invariant 7).
8. **The rung-1b loopback case asks the real broker's `QUERY_FORMAT` answers** for (XR24, MOD_INVALID)
   directly: the test backend always sets `CAP_MODIFIERS`, so the relay never takes 1b against it. The
   relay's 1b path is `rung_1b_is_the_implicit_modifier_after_an_explicit_yes` (scripted link).

### 8.8 Local runs (dev host, 2026-10-03; no GPU)

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
- GitHub CI (`stable`, `aarch64`) green at `78051779` (run 37126498993) and `df35472f` (run
  37127120277): it builds the workspace, runs kf-qemu's tests including `wire_mirror` at ABI 11, Clippy
  and the gates — including `UDMABUF-gate reached-count` (the runner's `/dev/udmabuf` is not permitted,
  so both gated tests print `SKIPPED` and are counted). CI does **not** compile kf3.c: every QEMU call in
  it was read against v10.2.4's headers.

### 8.9 Pending box tests (exact commands; renting needs the owner's approval)

Every result cites the kf3 binary's revision (§5.2). On the box, with the branch head checked out:

1. **Build and the unchanged bar.** `scripts/bench/build_kf3.sh` (QEMU 10.2.4, ABI 11 — the device
   refuses an ABI-10 archive), then the merge bar with `display-broker` unset:
   `scripts/bench/v3_gates.sh` and `KF_DEVICE=kf3 scripts/fastguest/fast_suite.sh <tag> 180` (30/30),
   and the display lane M1/M2 (`scripts/bench/display/`) — pixel-exact as before.
2. **The broker.** `git -C <nvkvm-pv> archive 368d2db src/broker src/common | tar -x -C /opt/nvkvm-broker
   && make -C /opt/nvkvm-broker/src/broker nvkvm-display-broker`; then the relay's own loopback on the
   box: `KF_BROKER_BIN=/opt/nvkvm-broker/src/broker/nvkvm-display-broker cargo test -p kf-broker --test
   broker_loopback -- --ignored --test-threads=1` (6/6, `UDMABUF-GATE: RAN`).
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
6. **X11 with scaling active:** the broker window resized away from the guest's mode (XRender path);
   grade: the frame rate is not capped at 10 fps (`broker[sent=…]` over 10 s ≈ the guest's flip rate).
7. **Pointer visible (3d):** in the Wayland and X11 broker runs of (5), move the guest pointer
   (`a <x> <y>` on the test backend's stdin, or the real compositor's pointer) and grade a
   compositor screenshot and the console's `screendump` for the pointer image at the point; the
   same on the console alone with `display-broker` unset (the cursor layer applies there too).
8. **Never holds the guest:** `kill -STOP <broker>` for 10 s while `kfdisp_probe` flips: ~60 Hz, 0 flip
   timeouts; `kill -CONT`; then `kill -9` and a restart: reconnect within the backoff and the last frame
   replayed (broker log `TEST attach`), VM unaffected.
9. **Absent at boot:** boot with no broker; the VM boots, the console and VNC work; start the broker
   later: it attaches.
10. **Resize** — needs 3c; graded when 3c lands (SURFACE 1600x900 → one hotplug → `modetest` lists the new
    preferred mode and 1080p; 2560x1440 fitted under 165 MHz).

### 8.10 Owner questions (each implemented with the stated default; the owner confirms later)

1. Peer policy: {0, QEMU euid, the socket directory's owner} + `display-broker-uid` — implemented.
2. Broker distribution: installed separately, pinned to nvkvm-pv `368d2db` — as above.
3. Clipboard: left out of step 3.
4. Native resizes above ~1920×1200@60 (DVI single-link): not in this step.
5. The 3c hotplug registration amending A.11's `osevent` rule: not built.
6. Reuse without RELEASE: the narrowed rule of §8.3, with the deviations §8.7 (1)-(2).
