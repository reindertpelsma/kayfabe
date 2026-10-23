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
the real guest RM notification handshake, and bystander progress both after the
error and after destruction of the victim channel and its TSG. It selects the
first supported COPY0..9 engine with the public `grce` capability false, matching
the native rig's asynchronous-CE qualification.
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

CODE READ capture helpers, 2026-10-23: define these in `run-controls.sh` before
the arm commands. They write only inside that arm's directory; `rc_capture_end`
receives JSON `null` for native mode or the frozen native detail as a JSON string
for OFF/ON. The end time is recorded immediately after the process exits.

```bash
set -euo pipefail
rc_capture_begin() {
  rc_dir=$1 rc_run=$2 rc_mode=$3
  test ! -e "$rc_dir/process.json"
  rc_cursor=$(journalctl -k -b -n 0 --show-cursor --no-pager | sed -n 's/^-- cursor: //p')
  test -n "$rc_cursor"
  printf '%s\n' "$rc_cursor" >"$rc_dir/journal.cursor"
}
rc_capture_end() {
  rc_end_ns=$(date +%s%N)
  journalctl -k -b --after-cursor="$rc_cursor" -o json --no-pager >"$rc_dir/journal.jsonl"
  python3 - "$rc_dir" "$rc_run" "$rc_mode" "$SOURCE" "$PCI" "$rc_end_ns" "$1" <<'PY_CAPTURE'
import json, pathlib, sys
folder, run, mode, source, pci, end, detail = sys.argv[1:]
folder = pathlib.Path(folder)
meta = json.loads((folder/'process.json').read_text())
manifest = dict(run=run, mode=mode, source=source, pid=meta['pid'], pci=pci,
                start_ns=meta['start_ns'], end_ns=int(end), detail=json.loads(detail))
with (folder/'manifest.json').open('x') as stream:
    json.dump(manifest, stream, sort_keys=True)
PY_CAPTURE
}
```

```bash
# INSIDE the held global reservation. PCI is independently recorded, no function.
: "${PCI:?}"                    # e.g. lower-case domain:bus:device
mkdir "$OUT/native"
export KF_RC_META="$OUT/native/process.json"
export KF_RC_EXEC="$CARGO_TARGET_DIR/release/kf-rc-native"
# Snapshot before launch, then record the interval end and unfiltered journal.
rc_capture_begin "$OUT/native" native-rc-1 native
rc_status=0
timeout --signal=TERM 120 "$OUT/identity-exec.py" native-rc-1 >"$OUT/native/native.log" 2>&1 || rc_status=$?
rc_capture_end null
# Guest capture is empty for native characterization.
: >"$OUT/native/guest.log"
```

CODE READ manifest construction, 2026-10-23: for native mode set `detail=null`.
After saving the journal and end time, write this JSON using wrapper metadata
and the independently recorded PCI. For guest OFF/ON use the same structure,
with that arm's own wrapper metadata, run name and `mode`. Freeze `detail` to the
native result **before** either guest run. Save native verdict and manifest as
immutable inputs; never derive an allowed suffix from the ON fault.

```bash
python3 "$REPO/scripts/fastguest/translated_rc_guard.py" \
  --manifest "$OUT/native/manifest.json" --qemu "$OUT/native/native.log" \
  --guest "$OUT/native/guest.log" --journal "$OUT/native/journal.jsonl" \
  >"$OUT/native/verdict.json"
test "$rc_status" -eq 0
# Require NATIVE_ONLY, then freeze its exact detail before either guest run.
DETAIL_JSON=$(python3 - "$OUT/native/verdict.json" "$SOURCE" <<'PY_FREEZE'
import json, sys
v = json.load(open(sys.argv[1]))
assert v["verdict"] == "NATIVE_ONLY" and v["source"] == sys.argv[2]
print(json.dumps(v["detail"]))
PY_FREEZE
)
readonly DETAIL_JSON
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
# Existing fastguest failure gates remain enabled.
rc_capture_begin "$DIR" "$run" "$mode"
rc_status=0
bash "$REPO/scripts/fastguest/run_fast_guest.sh" "$run" 120 >"$DIR/runner.log" 2>&1 || rc_status=$?
rc_capture_end "$DETAIL_JSON"
python3 "$REPO/scripts/fastguest/translated_rc_guard.py" \
  --manifest "$DIR/manifest.json" --qemu "$DIR/fast_${run}_qemu.log" \
  --guest "$DIR/fast_${run}_serial.log" --journal "$DIR/journal.jsonl" \
  >"$DIR/verdict.json"
test "$rc_status" -eq 0
```

CODE READ acceptance, 2026-10-23: require both fastguest's successful exit/DONE
and the strict control verdict for each fresh boot. OFF requires zero host Xids,
zero guest error evidence, clean notifier, bystander progress, owner success and
guest health. ON requires exactly one marker, submission, host observation and
RC posting for the uniquely born Translated twin; the actual guest notifier
must have nonzero timestamp, code32, guest engine, statusffff and GP_GET=0; the
actual guest RC event must match chid/engine/code/scope and occur once; the owning
loader must receive EIO. All three bystander stages (before the fault, after the error, and after victim
channel/TSG destruction), post-fault guest health and clean exit are mandatory.
The guest birth record must also attest `async_ce=1`, derived from public copy
engine capabilities; an absent/non-asynchronous qualification fails. The host Xid must match PCI, PID, host chid, code32, frozen
interrupt suffix and marker interval exactly. **Any additional Xid fails.**
FAULT_MARK is an intentional exact-match control, never a general Xid32 allowlist.

CODE READ completion, 2026-10-23: preserve small manifests, verdicts and relevant
text excerpts in `traces/translated_rc_reset_20261018/`; keep complete captures in
the lane directory. Record source/driver/kernel/GPU identities and n/spread. An
unmatched fault or timeout ends the run; diagnose the missing event, never retry
with adjusted timers. Confirm all owned processes exited and release the outer
lock. Clean owned build targets with `cargo clean --target-dir` semantics or
`find "$CARGO_TARGET_DIR" -mindepth 1 -delete`; no shared-directory deletion.

MEASURED follow-up, 2026-10-23, source `9ca3aaada7cdfa98b230c6881612d2d6e040812b`:
EXPECTED RED commits `9b0a9140d` and `5f18f946d` precede the stricter post-teardown
bystander and asynchronous-CE requirements. The final capture/patch suite passes
63/63. Nine Python mutants each have at least two killers, including removal of
each new requirement (two killers each). The updated guest probe compiles as a C
object against dev kernel headers 7.0.0-38 using the pinned public ABI. This is
not a linked module or hardware run. The final Rust bar completed on this source
under the dev-box cargo flock after the build farm entered a quiet window:
4020 passed, 0 failed, 22 ignored across 236 targets; Clippy added=0/errors=0
(278→277 diagnostic identities), claims added=0. The final counts, exact commands
and input hashes are in `control-final.json` in the evidence directory.

MEASURED GPU-free validation, 2026-10-23: at build-farm **BF_HEAD
`1c78c481e783b6f2b44a86ed5027a14e5ed7c779`**, the 22 kf-* crates passed
4020 tests, failed 0 and ignored 22 across 236 targets. Optional-feature Clippy
for kf-chan/kf-qemu/kf-harness, all targets, completed with 277 warning identities
versus the earlier 278, added=0/errors=0. All six Rust fault-arm mutants had at
least two killers, and the restored code passed 21/21 tests before/after the run.
The seven guard/observer mutants also had at least two killers; Python tests:
59/59 at that checkpoint. Commands, exact mutation killers and diagnostic comparisons are in
[`traces/translated_rc_reset_20261018/`](../../traces/translated_rc_reset_20261018/).
The build-farm result supersedes the earlier dev-box cargo-lock stop.

CODE READ build-farm reproduction, 2026-10-23: push the lane branch first. On the
controller-provided build farm, run the pushed mutation runner with an unused
lane-owned output directory; record its BF_HEAD and require it to equal the
intended source revision. The cargo command arrays in `control-build-farm.json`
reproduce the full suite and Clippy. The farm supplies nice 15, eight jobs and its
own lane target; it is an authorized exception to dev-box cargo settings. Respect
BF_REFUSED/exit75 while the host quiet flag exists; wait for the next window or
use the documented dev-box flock, never remove that flag. Existing KVM-gated CPU unit tests are included; these commands do not run the
GPU fault-control arm or boot its Linux OS guest. Example (BF_HOST is the controller-provided SSH destination):

```bash
ssh "${BF_HOST:?}" /data/build-farm/bf b1 codex/translated-rc-reset-20261022 \
  python3 scripts/fastguest/check_translated_rc_mutants.py \
  --out /data/b1/rc-control-new-mutant-evidence --jobs 8
```

CODE READ limits, 2026-10-23: C object compilation and synthetic capture acceptance
are not hardware acceptance. Full guest-driver/probe-module linking against the
chosen guest kernel, actual Translated route, native Xid suffix, guest event
receipt, isolation and teardown remain unverified. The native/OFF/ON runbook above
is the remaining acceptance procedure. No reset-recovery claim is made.
