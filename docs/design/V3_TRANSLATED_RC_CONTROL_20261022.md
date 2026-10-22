**STATUS: RESEARCH, 2026-10-23 — executable Linux control; hardware acceptance pending (n=0).**

CODE READ, 2026-10-23: this implements item (2) after the owner's acceptance of
[RESET_CHANNEL refusal](V3_TRANSLATED_RC_RESET_20261022.md). No reset control,
recovery success, notifier clear, or completion for failed work is added. The
hardware question is whether a fault reaches its owning process while only that
channel dies. The Blackwell merged-B1 ON arm remains optional on an authorized
future lease; this control requires only a C56F Linux guest and an ordinary host GPU.

INFERRED hypothesis, stated before hardware execution: one host-authored C56F
reserved opcode on an owned USER twin yields Xid32; B1 publishes the corresponding
guest notifier and RC event without retiring failed guest work, while another
Translated channel completes real GPU work. Falsifiers: no matching host Xid32,
missing/duplicate/misrouted notifier or event, owner success instead of EIO,
advanced victim GP_GET, stalled bystander, guest panic/hang, unmatched host Xid,
or failure to exit inside the fixed capture budget. No hardware run has tested
this hypothesis as of 2026-10-23.

### Public contract and placement

CODE READ source pin: NVIDIA open-gpu-kernel-modules **580.159.04**, commit
`b81d58ee0224d1d290bef1c080592b619e184042`. The following are public source,
not closed-driver analysis:

- [nv_uvm_interface.h](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/580.159.04/kernel-open/common/inc/nv_uvm_interface.h)
  exposes kernel-origin channel/TSG allocation, memory, and CPU-visible channel
  information. The probe derives declarations from this file and includes its
  actual `nv_uvm_types.h` types; it does not guess an ioctl ABI.
- [clc56f.h](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/580.159.04/src/common/sdk/nvidia/inc/class/clc56f.h)
  defines SEC_OP_RESERVED6, bits 31:29. The fixed bad word is `0xc0000000`.
  This is a candidate invalid stream, not an established Xid interrupt suffix.
- [kernel_gsp.c](https://github.com/NVIDIA/open-gpu-kernel-modules/blob/580.159.04/src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c)
  `_kgspRpcRCTriggered` invokes `krcErrorSendEventNotifications_HAL`. A guest-only
  source patch records identity and advances a bench handshake **after** that
  operation returns NV_OK. It never writes the notifier or fabricates an event.

CODE READ, 2026-10-23: a normal Linux userspace channel takes the Passthrough route;
its invalid pushbuffer alone does not exercise B1. The guest userspace owner
therefore loads a small kernel harness through `finit_module`. Its two channels
are allocated through the public kernel interface in separate TSGs. The owning
loader receives EIO only after the harness checks the victim's real error record,
the real guest RM notification handshake, and post-error bystander progress.
This tests a userspace-owned **harness**; it does not establish arbitrary CUDA
process recovery or Windows TDR behavior. The frozen `kayfabe-rm-ladder` is untouched.

CODE READ, 2026-10-23: host support is compiled only with `rc-negative-control`
(`kf-chan`, `kf-qemu`, `kf-harness`) and armed only by a valid `KF3_RC_NEGCTL` run
identifier plus a compile-time `KF_RC_SOURCE_REV`. Ordinary builds and unset
runtime configuration require no new boot flag. The worker consumes its existing
GP_PUT observation; no polling or timer adjustment was added. Exactly one
Translated C56F COPY0..9 USER twin per process may consume the special bench put.
Guest bytes never supply the native stream. The authored reserved word is stored
before a real native fence tail and doorbell. A partial failure is not retried or
reported as RC. The claimed slot stops normal pumping while awaiting B1's durable
host error record; unrelated slots retain their ordinary path.

CODE READ, 2026-10-23: the bounds are one injection per armed process, 1..32 ASCII
run characters, qualified engine/class/USER predicates, and 2 MiB per capture
input. Guest root can select a qualifying victim when the host explicitly arms
this bench binary; it cannot change the word, select another process's channel,
request repeated injections, or gain a privileged host verb. Do not expose an
armed bench QEMU as a normal multi-user service.

### Trusted-host runbook (not executed)

CODE READ preparation, 2026-10-23: use only the lane's own `/data/b1/` directories
and documented revision-specific binary outputs. Do not load the guest probe or
patched guest driver on the host. The host retains its ordinary driver. Build
against the exact guest kernel; the base initrd and patched driver must both be
580.159.04 with identical vermagic. Module signing, if required by that guest,
must use its existing approved build procedure. Stop on a build/version refusal.
No source or output directory below may belong to another lane.

```bash
# Set these to private, already provisioned inputs; BASE_FG contains vmlinuz,
# initrd.cpio.gz, and driver.STAGED. GUEST_HEADERS matches its kernel exactly.
export REPO=/data/b1/kayfabe
export OUT=/data/b1/rc-control-$(date -u +%Y%m%dT%H%M%SZ)
export PUBLIC_OGKM=/data/b1/open-gpu-kernel-modules
export PRIVATE_QEMU=/data/b1/qemu-source
: "${BASE_FG:?}" "${GUEST_HEADERS:?}" "${GUEST_KERNEL_RELEASE:?}"
mkdir -p "$OUT"
cd "$REPO"
test -z "$(git status --porcelain)"
export SOURCE=$(git rev-parse HEAD)
df -h /
export CARGO_TARGET_DIR="$OUT/cargo-target"
bash scripts/fastguest/build_rc_control_qemu.sh \
  "$PRIVATE_QEMU" "$OUT/qemu-build" "$CARGO_TARGET_DIR"
# Read KF3_BUILT from the successful build; no shared binary substitution.
export REAL_QEMU="$OUT/qemu-build/qemu-system-x86_64"
df -h /
flock -w 600 /tmp/claude-0/kf-local-cargo.lock \
  env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" KF_RC_SOURCE_REV="$SOURCE" \
  nice -n 19 cargo build --release -j2 -p kf-harness \
  --features rc-negative-control --bin kf-rc-native
python3 scripts/fastguest/prepare_translated_rc.py \
  --ogkm "$PUBLIC_OGKM" --out "$OUT/guest-build"
# Build only private guest artifacts; do not run modules_install.
nice -n 19 make -C "$OUT/guest-build/ogkm" -j2 modules \
  SYSSRC="$GUEST_HEADERS" SYSOUT="$GUEST_HEADERS" \
  KERNEL_UNAME="$GUEST_KERNEL_RELEASE"
nice -n 19 make -C "$GUEST_HEADERS" M="$OUT/guest-build" -j2 modules \
  KBUILD_EXTRA_SYMBOLS="$OUT/guest-build/ogkm/kernel-open/Module.symvers"
cc -std=c11 -Wall -Wextra -Werror -static \
  scripts/fastguest/kf_rc_owner.c -o "$OUT/kf_rc_owner"
mkdir "$OUT/fastguest"
cp "$BASE_FG/vmlinuz" "$BASE_FG/driver.STAGED" "$OUT/fastguest/"
bash scripts/fastguest/pack_rc_control_guest.sh \
  "$BASE_FG/initrd.cpio.gz" "$OUT/guest-build/kf_rc_probe.ko" \
  "$OUT/kf_rc_owner" "$OUT/guest-build/ogkm/kernel-open/nvidia.ko" \
  "$OUT/fastguest/initrd.cpio.gz"
```

CODE READ identity capture, 2026-10-23: create the following wrapper at
`$OUT/identity-exec.py`, make it executable, and use it for both the native rig and
QEMU. It records the process independently of the in-process FAULT_MARK; exec
preserves its PID. Select the host GPU PCI BDF independently, for example from
`nvidia-smi --query-gpu=pci.bus_id --format=csv,noheader`; the manifest PCI field
uses lower-case `0000:01:00` form without function. Do not infer PID or PCI from
the fault being excused.

```python
#!/usr/bin/env python3
import json, os, sys, time
with open(os.environ['KF_RC_META'], 'x') as f:
    json.dump(dict(pid=os.getpid(), start_ns=time.time_ns(),
                   start_ticks=open('/proc/self/stat').read().split()[21]), f)
os.execv(os.environ['KF_RC_EXEC'], [os.environ['KF_RC_EXEC'], *sys.argv[1:]])
```

CODE READ reservation, 2026-10-23: put the following steps in your own
`$OUT/run-controls.sh` and execute the **entire** script under
`flock -w 600 -o /tmp/kayfabe-fastguest.lock bash "$OUT/run-controls.sh"`.
A reservation timeout is a blocker. Never break the lock, signal another owner's
process, or launch concurrently with an existing guest. The inner fastguest lock
below is private because this outer reservation continuously owns the global GPU
lock. Keep native, OFF, and ON strictly sequential; stop after any failure.

CODE READ capture procedure, 2026-10-23: for each arm save the current kernel
journal cursor immediately before launch, run once, then capture JSON records
using that cursor. Use a fresh directory and run identifier each time. Use
`journalctl -k -b -n 0 --show-cursor` to obtain the cursor and
`journalctl -k -b --after-cursor="$CURSOR" -o json --no-pager` for the capture.
Store all records, including unrelated Xids; filtering them would invalidate the
guard. Reject files over 2 MiB. Record the end time immediately after the process
exits. Each manifest interval must be at most 180 s. The wrapper metadata is the
source of its PID/start time; record `/proc` start ticks for audit.

```bash
# INSIDE the held global reservation. PCI is independently recorded, no function.
: "${PCI:?}"                    # e.g. lower-case domain:bus:device
mkdir "$OUT/native"
export KF_RC_META="$OUT/native/process.json"
export KF_RC_EXEC="$CARGO_TARGET_DIR/release/kf-rc-native"
# Save CURSOR here, before starting the process.
"$OUT/identity-exec.py" native-rc-1 >"$OUT/native/native.log" 2>&1
# Save end_ns and the unfiltered cursor-based journal to native/journal.jsonl.
# Guest capture is empty for native characterization.
: >"$OUT/native/guest.log"
```

CODE READ manifest construction, 2026-10-23: for native mode set `detail=null`.
After saving the journal and end time, write this JSON using wrapper metadata
and the independently recorded PCI. For guest OFF/ON use the same structure,
with that arm's own wrapper metadata, run name and `mode`. Freeze `detail` to the
native result **before** either guest run. Save native verdict and manifest as
immutable inputs; never derive an allowed suffix from the ON fault.

```python
# Example executed with paths/values from the operator's capture script.
import json
meta = json.load(open(process_json))
manifest = dict(run=run, mode=mode, source=source, pid=meta['pid'], pci=pci,
                start_ns=meta['start_ns'], end_ns=end_ns, detail=detail)
with open(manifest_json, 'x') as f:
    json.dump(manifest, f, sort_keys=True)
```

```bash
python3 "$REPO/scripts/fastguest/translated_rc_guard.py" \
  --manifest "$OUT/native/manifest.json" --qemu "$OUT/native/native.log" \
  --guest "$OUT/native/guest.log" --journal "$OUT/native/journal.jsonl" \
  >"$OUT/native/verdict.json"
# Require NATIVE_ONLY (never guest PASS), one Xid32, actual private RC record
# and event, and a native bystander fence after the error. Freeze its detail.
```

CODE READ native refusal policy, 2026-10-23: the native rig prints its marker
before the same fixed invalid word, requires a durable RC code32/statusffff and
OS event, and reports PASS only after bystander progress and successful cleanup.
If that word yields another Xid, no event, an unsupported suffix format, or a
cleanup refusal, stop and preserve the capture. Do not widen the intentional
exception or change a timeout to make it pass. Native success alone proves no
guest delivery.

```bash
# Repeat for mode=off then mode=on, each with a fresh guest boot and fresh DIR.
mode=off                        # the next run uses on
run="linux-rc-$mode-1"
DIR="$OUT/$run"
mkdir "$DIR"
export KF_RC_META="$DIR/process.json" KF_RC_EXEC="$REAL_QEMU"
export KF3_RC_NEGCTL="$run"     # OFF still arms the host; the guest sends no trigger
export BENCH_DIR="$DIR" KF_FASTGUEST_DIR="$OUT/fastguest"
export QEMU_BIN="$OUT/identity-exec.py" KF_LOCK="$OUT/fastguest-inner.lock"
export KF_PROBE=sem-release KF_ARMS="$mode,$run"
export KF_APPEND="loglevel=7 "  # include the probe's pr_info records on serial
# Save CURSOR here. Existing fastguest failure gates remain enabled.
bash "$REPO/scripts/fastguest/run_fast_guest.sh" "$run" 120 >"$DIR/runner.log" 2>&1
# Save end_ns, unfiltered journal.jsonl, and manifest.json with frozen detail.
python3 "$REPO/scripts/fastguest/translated_rc_guard.py" \
  --manifest "$DIR/manifest.json" --qemu "$DIR/fast_${run}_qemu.log" \
  --guest "$DIR/fast_${run}_serial.log" --journal "$DIR/journal.jsonl" \
  >"$DIR/verdict.json"
```

CODE READ acceptance, 2026-10-23: require both fastguest's successful exit/DONE
and the strict control verdict for each fresh boot. OFF requires zero host Xids,
zero guest error evidence, clean notifier, bystander progress, owner success and
guest health. ON requires exactly one marker, submission, host observation and
RC posting for the uniquely born Translated twin; the actual guest notifier
must have nonzero timestamp, code32, guest engine, statusffff and GP_GET=0; the
actual guest RC event must match chid/engine/code/scope and occur once; the owning
loader must receive EIO. Both bystander steps, post-fault guest health and clean
exit are mandatory. The host Xid must match PCI, PID, host chid, code32, frozen
interrupt suffix and marker interval exactly. **Any additional Xid fails.**
FAULT_MARK is an intentional exact-match control, never a general Xid32 allowlist.

CODE READ completion, 2026-10-23: preserve small manifests, verdicts and relevant
text excerpts in `traces/translated_rc_reset_20261018/`; keep complete captures in
the lane directory. Record source/driver/kernel/GPU identities and n/spread. An
unmatched fault or timeout ends the run; diagnose the missing event, never retry
with adjusted timers. Confirm all owned processes exited and release the outer
lock. Clean owned build targets with `cargo clean --target-dir` semantics or
`find "$CARGO_TARGET_DIR" -mindepth 1 -delete`; no shared-directory deletion.

MEASURED GPU-free validation, 2026-10-23: the evidence checkpoint in
[`traces/translated_rc_reset_20261018/`](../../traces/translated_rc_reset_20261018/)
records EXPECTED RED commits, green counts, mutation killers and diagnostic
comparisons. C object compilation and synthetic capture acceptance are not
hardware acceptance. Full guest-driver linking, actual Translated route, native
Xid suffix, guest event receipt, isolation and teardown remain hardware-unverified.
