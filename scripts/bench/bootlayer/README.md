# Boot-layer host capture plan

STATUS: DESIGN-ONLY, 2026-10-18. CODE READ: these scripts use the existing QMP
transport and device log vocabulary. MEASURED: L5 performed syntax checks only;
no script, VM, GPU operation, guest command or capture in this directory was run.
A successful collector exit means observations were saved, never a hardware PASS.

## Host-lane preparation

1. Use a disposable Linux guest and a disposable Windows guest. Keep the owner's
   live VM and launcher alone; arrange access through that host lane's authorized
   stop procedure. `run.sh` takes `flock -o /tmp/kayfabe-fastguest.lock`, waits at
   most 60 seconds and exits 75 if busy. Do not invoke it inside another holder of
   that lock. It never searches for or kills an existing process.
2. Build the exact lane revision using the normal host procedure. Record GPU die,
   host driver/flavour, guest driver, source SHA, binary SHA, UUID and broker
   revision. The runner records the supplied SHA and binary hash; the host lane
   must establish that the binary was actually built from that SHA.
3. Copy the known-good launcher's QEMU argv, including its UUID, firmware, guest
   disk, RAM, broker and device properties. Give kf3 `id=kf0,display=on,gop=on`.
   Use a foreground QEMU binary, no `-no-reboot` and no launcher shell. Reboot must
   use the SAME QEMU PID/store; a process restart does not test the scrub.
4. Put every output under the host lane's own `/data/<lane>/` directory. Example
   (replace the angle-bracket operands; do not paste the placeholders literally):

   ```sh
   scripts/bench/bootlayer/run.sh <lane> /data/<lane>/bootlayer-linux <40-hex-sha> 180 \
     --store-reader /data/<lane>/existing-store-reader -- \
     <qemu-binary> <the-assigned-guest-qemu-arguments>
   ```

   The wrapper adds an initial `-S`, two QMP sockets, and `-action reboot=reset`.
   The collector starts before QEMU. It samples the realized, paused device,
   sends `cont`, and requests PNG screendumps every 100 ms from the first guest
   instruction. Actual start/end times are in `frames.jsonl`; a slow copy causes
   a recorded gap, never a catch-up burst. The default budget is 2 GiB and the
   collector stops if less than 4 GiB is free. A hard failure is not a missing
   frame silently treated as black. Duration includes QEMU realization; choose
   an adequate bounded duration (maximum 900 seconds).
5. The second QMP socket is `<output>/control.qmp`. Existing helper:

   ```sh
   python3 scripts/bench/windows/host_tools/runners/boundary-tools/qmp.py \
     <output>/control.qmp cmd query-status
   ```

   CODE READ: this branch's broker has no screendump control CLI. For its actual
   window, use the host lane's existing compositor/window capture tool and record
   that command and matching timestamps; if its installed broker revision has a
   capture API, identify that revision and API explicitly. QMP observes QEMU's
   console; it does not establish what the broker imported or presented. Do not
   start a second broker on the owner's socket. Keep its log/revision with the run.

## Five store observations

CODE READ: `WalkKernel::read_store` and `DisplayGpu::read_store` are bounded
internal GPU readback APIs. L5 found no existing external QMP/store-dump command
in this branch. **This part is pending the host lane identifying the existing
diagnostic adapter.** Do not substitute QMP `pmemsave`, BAR1 `xp`, `/proc/PID/mem`
or a framebuffer screenshot: virtual BAR1 can show a different store offset or
scratch. No new product diagnostic, guest trap or debugger call is introduced.

`phase_dump.py` accepts that existing adapter as an executable, without a shell:

```text
existing-store-reader OFFSET LENGTH RAW_OUTPUT
stdout: {"space":"store","offset":0,"length":8323072,"source":"the diagnostic identity"}
```

The adapter must run in the foreground without detached children, read exactly store `[0,0x7f0000)` of this assigned QEMU process
through its existing diagnostic path, return only after completion, and exit
nonzero on refusal. It must not attach to another VM, read through a BAR alias,
or execute a blocking CUDA call on a vCPU/under QEMU's BQL. It gets a 20-second
measurement deadline. Raw bytes and a complete, repeat-collapsed hexdump stay on
the host. JSON records the byte count, nonzero count, hash, source and start/end
times. Missing/short/misidentified reads are `UNAVAILABLE`, never zero-filled.

| Phase | Trigger | Question |
|---|---|---|
| `realized` | QMP ready, initial `-S`, before `cont` | Is the complete realized boot range zero? |
| `fn72` | `GSP rpc GspSetSystemInfo` | What reaches the range at RM life start? |
| `armed` | first `the console shows head ...` | What is in the former boot range when display takes over? |
| `unloaded` | unload range zero completes (`scrubbed first`) | Is the old range clean, and did the physical seed actually return? |
| `reboot_scrubbed` | `NEW OS: ... completion awaited, N ms` | Did a same-process reboot scrub, with the expected wall-time line? |

The running phases are triggered reads, not atomic snapshots at the log event.
Compare `events.jsonl`, the sample's start/end and `qmp-events.jsonl`; reject a
phase that races the next transition. The first observation is paused and
unambiguous. A log bound, missing trigger, refused reseed or missing adapter
leaves the phase missing/unavailable. `capture-summary.json` retains that fact.
For a manual phase use `phase_dump.py OUTPUT PHASE --revision SHA --reader READER`
in a fresh output directory; it exits 78 if the path is unavailable.

## Linux unload, reload and reboot

Before a run, state: hypothesis = unload restores the physical boot view and a
redraw changes the monitor; falsifier = seed refused, wrong BAR1 mapping, no new
console text, a newly allocated RM life retired, or re-init fails. Keep the
existing B5 controls in `../display/unload_hook.sh` as prior art; do not run that
whole script on the shared host without adapting its output paths and guest.

1. Capture a working NVIDIA desktop, then stop the assigned guest's graphical
   session, persistence daemon and GPU clients through that guest's normal
   mechanism. Stage `linux_console.sh` in the guest. Run it as guest root with
   `unload`. Busy modules fail the phase; no forced unload or process sweep.
2. Run `redraw` twice and save two different text images through QMP and the
   broker. Require fn 47 with no PM/GC6, successful frees, a re-seed instead of an
   overlap refusal, BOOT chosen only after a completed zero, and visibly new text.
   A black screen or old unchanged console image fails. Capture dmesg and module
   state; distinguish a preserved NVKMS console from the physical boot view.
3. Run `reload`; require a new fn 72, normal init, successful `nvidia-smi`, an
   armed NVIDIA desktop and a second unload/redraw. Repeat at least three times.
   Include the reboot-before-first-RM case separately: the seed has never retired.
4. For the boot-range read-back probe, keep the guest driver unloaded and the
   physical seed confirmed. Stage `guest_pattern.py` in the guest. Supply the
   BAR1-mode register, mask and PHYSICAL value derived for this die from
   `kf-chip::bar1mode`/the generated hwref table:

   ```sh
   sudo python3 guest_pattern.py fill <BDF> --mode-register <offset> \
     --mode-mask <mask> --physical-value <value>
   ```

   It writes one padding page `[G-4096,G)` above the visible 1920x1080x4 pixels.
   Confirm the exact pattern through the store diagnostic before reboot; a BAR1
   echo alone can be scratch. If PCI BAR decoding is disabled after unbind, record
   the refusal; the script deliberately does not program or enable the device.
5. Request the assigned guest's normal reboot or QMP `system_reset`. Confirm the
   QEMU PID is unchanged, exactly one relevant NEW OS scrub completion appears
   (record any extra reset), and retain its `N ms` wall time in `new-os.txt`.
   In the new OS BEFORE NVIDIA loads, run the same command with `read-zero` and
   corroborate with the store readback. Stop on a nonzero byte, missing scrub or
   unexpected second scrub; do not change a timeout or delay to get a pass.
6. Scope: that pattern verifies only the reused boot region, which finish also
   zeroes. It does NOT prove whole-store coverage. For that oracle a host lane
   must additionally fill guest-controlled pages outside `[0,G)`, including the
   carve-out, through its existing guest GPU/diagnostic tool; prove their actual
   store offsets before reboot and read the SAME offsets after it, before reuse.
   Preserve expected root-page rewrites as exceptions. No such whole-store helper
   is claimed to exist here. A resumed bus reset and S3 must show NO NEW OS scrub.

## Windows Basic Display and O1/O3 discriminators

Hypothesis = this RM's old BAR1 mappings outlive unload; falsifier = a new fn 72,
BAR1 root or virtual-mode mapping precedes the attempted retirement. Use the
ordinary Windows driver-control path on the disposable guest to select the
in-box Microsoft Basic Display Adapter for the assigned device (Device Manager,
Update driver → Browse → Let me pick). Do not delete the driver package. Record
its exact instance id and every requested reboot. Disable/re-enable alone does
not establish that Basic Display bound.

Stage `windows_basic.ps1`; invoke its read-only identity check through the existing
guest-agent script-FILE channel with that instance id. It requires status OK,
Microsoft provider and service `BasicDisplay`. In a PowerShell window on the
logged-in guest desktop, run the same script with `-ShowClock` to show its moving
clock; a service/session-0 agent is not an interactive UI launcher. Capture two
distinct QMP and broker frames, before and after the
switch, and the fn-47/frees/physical-view log slice. Run a second redraw after the
adapter is stable; a one-time black/old image is not console return. Switch back
to NVIDIA using the installed signed driver; require a new fn 72 and working
apps. Repeat at least three cycles, plus a normal reboot in the SAME process.

For each refused re-seed retain the last BAR1 coverage/root, the oldest mapping
installation, PHYSICAL write, fn 47's full fields, frees completion, every fn 72
and later BAR1 change. Capture whether the monitor is BOOT, blank, held or armed.
Explicitly exercise/record PHYSICAL writes during normal init, ordinary use,
unload, failed init, TDR and S3 resume. Windows' closed RM is not proved by the
Linux HAL source or by one end-of-life log. A fast unload/reload arm must check
whether fn 72 can arrive between the VA thread's generation check and zero; the
current observation-only guard does not prove exclusion of that interleaving.

## Report and retention

Report n and spread of NEW OS wall time; screenshots' actual gap distribution;
phase completeness; successful console changes and re-init cycles; every refused
seed; and before/after guest/host fault counts. Make no throughput/FPS claim from
this diagnostic capture. Copy only selected small text/PNG evidence into
`traces/bootlayer_followup_20261018/`; keep raw files and full logs on the host,
with their paths in the host-lane report. The wrapper reaps only its own QEMU and
collector and releases the flock on exit. Verify that cleanup in `run.json` and
with the host lane's ordinary own-PID check.
