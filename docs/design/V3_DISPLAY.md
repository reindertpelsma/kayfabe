# V3 display — a virtual NVIDIA display the stock driver drives, scanned out by kayfabe

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
> - **The frame to capture** is, per head, the window surface latched at vblank plus the cursor
>   (built, §4.4–§4.6). Write the flip-completion notifier only after the frame was taken, and with
>   the broker never overwrite a copy before `RELEASE`.
> - **Order:**
>   1. the GOP option ROM;
>   2. a stock guest display with no tweaks;
>   3. the broker;
>   4. the unload tests.
>
>   ⊘ *Corrected 2026-10-03 (later): the recommendation is now **A, guarded**, with B as the fallback
>   (`docs/OWNER_QUESTIONS_2026-10-03.md` item 2), and A is built as a default-off experiment so the owner
>   can rule on box data — the `x11-dispsw` note directly below. The sentence it corrects:*
>   X11 still waits on the `GF100_DISP_SW` choice (A/B; recommendation B).

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
> - **Answers:** (a) never exercised — host RM was asked for **no** release:
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
>   `no_map_asks_host_rm_for_a_kernel_cpu_mapping` pins that). No client measured asked for a release,
>   so nothing was dropped. A candidate that kernel-mapped every guest-RAM row of a display-SW space
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
>      the guest's behalf.
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
>      with the property on.
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
