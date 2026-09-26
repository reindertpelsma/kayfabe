#!/usr/bin/env bash
# ★★★★★ ONE ITERATION OF THE FAST LANE — boot, run, verdict, under a HARD budget.
#
# usage: run_fast_guest.sh [tag] [budget-seconds]
#   KF_ARMS="--timer --engines"   which raw-client arms to run (default: a small set)
#
# ## The rule this file exists to enforce: A TIMEOUT IS A CRASH.
#
# `[measured w760]` the fat-guest suite gave a timing-out arm 300 s, SIGKILLed it, and the arm
# AFTER it then failed on a device the kill had left unusable. Hours went into separating "this
# arm is broken" from "its predecessor poisoned it" — and the answer was collateral three times
# out of three. Under one whole-run budget that category cannot exist: nothing runs long enough
# to poison a successor, and a slow path is simply RED.
#
# ⇒ Perf is not a separate lane. A regression that doubles a round trip fails this run.
#
# ⊘ The budget covers EVERYTHING — QEMU start, kernel, insmod, the client, poweroff. A boot that
# hangs and a client that hangs are the same verdict here, deliberately: both mean "we cannot
# iterate", and distinguishing them is the serial log's job, not the harness's.
set -uo pipefail

TAG=${1:-fast}
BUDGET=${2:-20}
BENCH=${BENCH_DIR:-/workspace/bench}
FG=$BENCH/fastguest
if [ "${KF_DEVICE:-kf3}" = kf3 ]; then
    # ★ 2026-09-25: the binary built from THIS checkout's revision (`build_kf3.sh` installs one per
    # revision) — never the shared build dir's, which another build can replace mid-measurement.
    KF3_REPO="$(cd "$(dirname "$0")/../.." && pwd)"
    KF3_REV=$(git -C "$KF3_REPO" rev-parse --short=8 HEAD 2>/dev/null || echo unknown)
    [ -z "$(git -C "$KF3_REPO" status --porcelain --untracked-files=no 2>/dev/null)" ] || KF3_REV="$KF3_REV-dirty"
    Q=${QEMU_BIN:-$BENCH/kf3-bins/$KF3_REV/qemu-system-x86_64}
    [ -n "${QEMU_BIN:-}" ] || [ -x "$Q" ] || {
        echo "run_fast_guest: no kf3 binary for this checkout's revision ($KF3_REV) at $Q — run scripts/bench/build_kf3.sh from this checkout (or set QEMU_BIN)"
        exit 2
    }
    echo "== kf3 binary: $Q (rev $KF3_REV)"
else
    Q=${QEMU_BIN:-$BENCH/qemu-build/qemu-system-x86_64}
fi
SER=$BENCH/fast_${TAG}_serial.log

for f in "$FG/vmlinuz" "$FG/initrd.cpio.gz" "$Q"; do
    [ -f "$f" ] || { echo "run_fast_guest: missing $f — run build_fast_guest.sh"; exit 2; }
done
# ⚠ Serial, not ssh. See build_fast_guest.sh for the two boots this cost before it was serial.
rm -f "$SER"

# ★★★★★ **SERIALIZE, OR TWO RUNS DESTROY EACH OTHER — measured 2026-09-20 (w812).**
#
# The `pkill` below is deliberate (a clean slate per boot), and that makes ANY two concurrent
# invocations mutually destructive: each kills the other's QEMU, and the loser leaves a
# **zero-byte serial log** behind. ⊘ `[measured w812]` two arms re-run in parallel with a
# timeout sweep produced exactly that — two 0-byte logs, freshly timestamped, which read as
# *"the arm produced no output"* rather than *"the arm was shot"*. That is the repo's own
# `a KILLED background job is indistinguishable from a RUNNING one` trap, arriving through the
# harness instead of through ssh.
#
# ⚠ CLAUDE.md already says GPU tests run **strictly serially**. A rule that lives only in a
# document is a rule that gets broken by whoever did not read it; this makes it structural.
# ⊘ Blocking (not `-n`): a second run should WAIT, never silently skip — a skipped arm that
# reports nothing is the same false-negative the lock exists to prevent.
exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"
if command -v flock >/dev/null 2>&1; then
    echo "== serializing on ${KF_LOCK:-/tmp/kayfabe-fastguest.lock} (GPU runs are strictly serial)"
    flock 9 || { echo "run_fast_guest: could not take the run lock" >&2; exit 2; }
else
    echo "run_fast_guest: ⚠ no flock(1) — CONCURRENT RUNS WILL KILL EACH OTHER; run serially" >&2
fi

# ⊘ `pkill` on its own line and with the bracket trick: a pattern that appears later on the same
# command line matches the shell running it, and then everything after silently never runs
# (`nvkvm-pv`, 2026-08-17).
# ★ 2026-09-26 (V3_MULTI_GPU_AUDIT §2 harness): kill ONLY this lane's own QEMUs — the ones started
# with `-name kf-fastguest` below — never every `qemu-system-x86` on the box (that shot other VMs,
# and ruled out several VMs per host by construction). The lock above already serializes this lane.
pkill -9 -f '[k]f-fastguest' 2>/dev/null
sleep 1

# ⊘⊘⊘ **ONE WHITESPACE-FREE TOKEN, OR THE ARMS SILENTLY DO NOT ARRIVE.** The kernel command
# line is split on whitespace and no quoting survives it, so a space-separated `KF_ARMS`
# reaches `/init` truncated at the first space and the guest runs the DEFAULT arms while the
# log says it was asked for others — a green run that measured the wrong thing. Commas here,
# `tr ',' ' '` in `/init`.
# ★★★★★ **w788 — THE GUEST GETS THE PRIMITIVE THE EMULATOR ACTUALLY SERVES.**
#
# ⊘⊘⊘ `[measured w786]` EIGHT arms of the 30-arm suite reported their POSITIVE CONTROL as
# failed — `alias-two-vas`, `alias-unmap-observe`, `late-map-race`, `missing-page-fault`,
# `cross-client-leak`, `rpc-mixed-allocs`, `defer-liveness`, `blockage-coverage` — and every
# one of them carried this line, printed by the client BEFORE any rung ran:
#
#   W381_PROBE=sem-release — the w379 host-FIFO `SEM_RELEASE`. ⊘ The Mode-2 CPU copy-engine
#   emulator decodes `PushMethod::SemRelease` and DELIBERATELY DOES NOT ACT ON IT, so inside
#   a guest EVERY rung below is expected to report its CONTROL as FAILED and NOTRUN. That is
#   the emulator's declared scope, NOT a red.
#
# ⇒ **Those arms were UNMEASURED, not failing.** They were run with a primitive the emulator
# declares out of scope, and they said so, in the log, on every run. The client's default is
# `sem-release` for a BARE-METAL reason it states plainly — *"so every committed w379 arm
# stays byte-comparable to its own predecessors"* — and this harness only ever runs in the
# guest, where that reason does not apply and the opposite one does.
#
# ⚠ Read the cost: a suite scoreboard of 14/30 counted eight arms as failures that had never
# asked their question. `a_census_zero_needs_a_known_positive`, arriving as a whole battery.
#
# ⊘ `KF_PROBE=sem-release` still selects the old primitive — the point is that the choice is
# now MADE and PRINTED by the harness rather than inherited from a bare-metal default.
KF_PROBE=${KF_PROBE:-launch-dma}
case "$KF_PROBE" in
    launch-dma)  PROBE_TOK="--probe-launch-dma" ;;
    sem-release) PROBE_TOK="" ;;
    *) echo "run_fast_guest: KF_PROBE must be launch-dma or sem-release, got [$KF_PROBE]" >&2; exit 2 ;;
esac

ARMS_TOK=$(echo "${KF_ARMS:-} ${PROBE_TOK}" | tr -s ' ' ',' | sed 's/^,//; s/,$//')
[ -n "$ARMS_TOK" ] || ARMS_TOK="--timer,--engines,--doorbell-census"
case "$ARMS_TOK" in *[[:space:]]*) echo "run_fast_guest: KF_ARMS still holds whitespace after folding: [$ARMS_TOK]" >&2; exit 2 ;; esac

# ★ The client's OWN deadline fires INSIDE the guest, below the harness budget, so it can
# dump the ioctl ring over the serial console before QEMU is killed from outside. A deadline at
# or above the outer budget is a deadline that never speaks.
DEADLINE_MS=$(( (BUDGET - 4) * 1000 ))
[ "$DEADLINE_MS" -gt 1000 ] || DEADLINE_MS=1000

echo "== arms: $ARMS_TOK   budget: ${BUDGET}s   self-deadline: ${DEADLINE_MS}ms   trace: ${KF_IOCTL_TRACE:-ring}"
# ⊘ SUPERSEDED 2026-09-25 (coordinator ruling): the timing lane's default is now `ring`, and
# `verbose` is opt-in (`KF_IOCTL_TRACE=verbose`). `[measured vh, 536fcf85..acf0f907]` every verbose
# trace line goes to the hvc0 console, whose guest driver spins until QEMU has consumed it — a full
# nested-KVM round trip per ioctl: `--ce-client-guest-ram` reached 2 514-4 923 invalidates per 60 s
# verbose against 5 052-8 810 ring on the same binary. The owner's 2026-09-18 requirement still
# holds in `ring`: the client's self-deadline dumps the WHOLE ring at timeout
# (`ioctltrace::arm_self_deadline`), so a hang still leaves its record. ⊘ No verdict reads the
# trace: FAST_VERDICT comes from the DONE marker, the client's rc and the device's ledger lines.
# > Owner, 2026-09-18 (the text this replaces): *"Run in verbose mode so it prints the ioctls. At
# > timeout trace dump the whole thing."*
# ⚠ **THE BINARY'S AGE, PRINTED, BECAUSE A STALE QEMU IS INVISIBLE.** The device is a Rust
# archive LINKED INTO qemu-system-x86_64, so a source change that was never relinked runs the
# OLD device while the tree says otherwise. `[measured w763]` a default flip read as "the flip
# did not take" for one whole cycle; the binary predated it by four minutes.
# ⊘ Printed rather than checked: a check would need a provenance stamp inside the binary, and
# `strings | grep -q` as a gate is a trap this campaign has already paid for.
# ⊘⊘⊘ **w826 — THE PREVIOUS VM'S GPU MEMORY OUTLIVES ITS QEMU FOR A MOMENT.** `[measured w826
# q10/q11]` twice, the next run's store reservation probe answered NOTHING_RESERVABLE in 0.6 ms
# and realize refused (empty serial log) — while nvidia-smi showed 0 MiB used seconds later. The
# run lock releases when QEMU exits; the isolate child holding the ~11 GiB reservation exits
# after it. ⇒ Wait for the GPU to drain before launching, and say so if it never does.
if command -v nvidia-smi >/dev/null 2>&1; then
    KF_GPU_IDLE_MIB=${KF_GPU_IDLE_MIB:-512}
    for _i in $(seq 1 60); do
        # ★ The MAX over every GPU (a multi-GPU run uses several), not the first line's.
        _used=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null | tr -dc '0-9\n' | sort -n | tail -1)
        [ -n "$_used" ] && [ "$_used" -le "$KF_GPU_IDLE_MIB" ] && break
        sleep 0.5
    done
    [ -n "${_used:-}" ] && [ "$_used" -gt "$KF_GPU_IDLE_MIB" ] && \
        echo "run_fast_guest: ⚠ the GPU still holds ${_used} MiB after 30 s — a previous VM did not release it; the store reservation will likely fail" >&2
fi
echo "== qemu:  $Q  (built $(date -r "$Q" "+%Y-%m-%d %H:%M:%S" 2>/dev/null || echo unknown))"
# ⊘⊘⊘ **w793 — PRINTING THE AGE WAS NOT ENOUGH, AND IT COST A RUN THE SAME DAY IT WAS WRITTEN.**
#
# The line above already said *"a stale QEMU is invisible… a default flip read as `the flip did
# not take` for one whole cycle; the binary predated it by four minutes"*. It PRINTS and does
# not GATE — `a_check_that_reports_is_not_a_check_that_gates`, in the file that documents the
# hazard.
#
# `[measured w793]` a relink failed with `error[E0061]` on a call site that only compiles under
# `--features cuda-scratchpad`; the harness ran anyway, the guest booted the PREVIOUS binary,
# and its ledger was read as a regression of the change under test. The change was not even in
# the binary.
#
# ⇒ Refuse when the archive's own sources are newer than the linked binary. ⊘ Source mtime, not
# a git stamp: an uncommitted edit is exactly the case that bites, and a commit-hash stamp
# would call that tree clean.
# ★ v3: `kf3` is the only device (crates/kf-qemu + qemu/hw/misc/kf3), built by
# scripts/bench/build_kf3.sh into its own QEMU build dir. ⊘ The old `nvkvm` device and its
# archive checks (the source-mtime refusal above, the host-isolate feature check) moved to
# archive/ with crates/kayfabe-qemu-raw at the v3 cutover; KF_DEVICE=nvkvm now refuses by name.
KF_DEVICE=${KF_DEVICE:-kf3}
if [ "$KF_DEVICE" != kf3 ]; then
    echo "run_fast_guest: ⊘ KF_DEVICE=$KF_DEVICE refused — the old nvkvm device (crates/kayfabe-qemu-raw) was archived at v3 — see archive/README.md; use KF_DEVICE=kf3 (the default)" >&2
    exit 2
fi

# ⊘⊘⊘ **THE DEVICE LINE AND THE RAM BACKING ARE NOT THE FAST LANE’S TO INVENT.** As first
# written this file said `-device kayfabe-gpu` and `-machine q35,accel=kvm -m 4096`, and BOTH
# were wrong in a way that would have been read as a kayfabe defect:
#
#   1. The QOM type is **`nvkvm-gpu`**, not `kayfabe-gpu` — QEMU would have exited before the
#      kernel ran, and the verdict printed would have been `CRASH`, indistinguishable here from
#      a guest that hung.
#   2. `-m` ALONE gives an anonymous `MAP_PRIVATE` block **no other process can see**. The
#      scratchpad isolate lives in another process and adopts the hypervisor’s
#      `memory-backend-memfd` to reach guest RAM at all; without `share=on` the guest-RAM
#      crossing never arms and every store-backed path refuses — a REAL failure caused
#      entirely by the harness, on the lane built to stop exactly that.
#
# ⇒ Both derive from the fat guest’s own boot script (`scripts/bench/boot_nvkvm.sh`) rather
# than being restated here, so the two lanes cannot silently diverge on the thing under test.
NVKVM_RAM_MB=${NVKVM_RAM_MB:-2048}
RAMARGS=(-object "memory-backend-memfd,id=ram0,size=${NVKVM_RAM_MB}M,share=on"
         -machine "q35,accel=kvm,memory-backend=ram0" -m "$NVKVM_RAM_MB")

# ⚠ One variable, both halves: the device registers this BAR1 and the chip row tells the guest
# the same number. `nvkvm_apply_identity` refuses at realize if they differ.
# ★ **128, not 256** -- and the device told us so itself, in one line, in 4 seconds:
#   "a guest BAR1 of 256 MiB does not fit: 256 MiB + 16 MiB headroom > 256 MiB host aperture.
#    The largest that fits here is 128 MiB, and a 128-MiB-BAR1 GA106 is a real hardware
#    configuration -- a different truthful board, not a lie."
# ⊘ `boot_nvkvm.sh` defaults to 256 because the fat lane passes this variable explicitly on
# every invocation; a lane that passes NOTHING must default to something that boots. 128 is also
# the plan-of-record value for the single store, so the two agree.
BAR1_BYTES=$(( ${KAYFABE_GUEST_BAR1_MB:-128} * 1024 * 1024 ))
export KAYFABE_GUEST_BAR1_MB=${KAYFABE_GUEST_BAR1_MB:-128}

# ★ w770 — pass an operator-supplied VA through to the device's walk probe.
[ -n "${KAYFABE_PROBE_VA:-}" ] && export KAYFABE_PROBE_VA

# ★★ MULTI-GPU (V3_MULTI_GPU_AUDIT §2 harness, 2026-09-26): `KF3_GPUS=0,1` emits ONE kf3 device
# per listed HOST MINOR (`gpu-minor=<m>`, `id=kf<i>`), in order; the guest sees them as its
# /dev/nvidia0..N-1 and `/init` runs the client on each (`KF_MGPU_MODE`, below). Unset = the
# single-device lane exactly as before (one device, the property's default minor 0).
# `KF3_DEV_EXTRA` applies to every device. A host minor listed twice is the same card twice —
# `cardbudget` then refuses by name unless the summed BAR1 demand fits.
MGPU_TOK=""
case "$KF_DEVICE" in
    kf3)
        if [ -z "${KF3_GPUS:-}" ]; then
            DEVARGS=(-device "kf3-gpu,fb-mb=${KF3_FB_MB:-8192},bar1-size=$BAR1_BYTES,bar2-size=33554432,id=kf0${KF3_DEV_EXTRA:+,$KF3_DEV_EXTRA}")
        else
            case "$KF3_GPUS" in *[!0-9,]*|,*|*,|*,,*) echo "run_fast_guest: KF3_GPUS must be comma-separated host minors, got [$KF3_GPUS]" >&2; exit 2 ;; esac
            DEVARGS=(); _i=0
            for _m in $(echo "$KF3_GPUS" | tr ',' ' '); do
                DEVARGS+=(-device "kf3-gpu,gpu-minor=$_m,fb-mb=${KF3_FB_MB:-8192},bar1-size=$BAR1_BYTES,bar2-size=33554432,id=kf$_i${KF3_DEV_EXTRA:+,$KF3_DEV_EXTRA}")
                _i=$((_i+1))
            done
            # ★ The guest's side of the contract, in whitespace-free tokens (see KF_ARMS above).
            # KF_MGPU_MODE: serial | concurrent | serial,concurrent (default: both, serial first).
            # KF_HOLD0=0 (default) leaves /dev/nvidia0 CLOSED while GPU 1.. run alone — the
            # instance-renumbering case (the guest RM gives the first-attached GPU instance 0,
            # whatever its minor). KF_HOLD0=1 holds it open throughout.
            _mode=$(echo "${KF_MGPU_MODE:-serial,concurrent}" | tr -d ' ')
            case ",$_mode," in *,serial,*|*,concurrent,*) ;; *) echo "run_fast_guest: KF_MGPU_MODE must name serial and/or concurrent, got [$_mode]" >&2; exit 2 ;; esac
            MGPU_TOK="KF_NGPU=$_i KF_MGPU_MODE=$_mode KF_HOLD0=${KF_HOLD0:-0} "
        fi ;;
    *) echo "run_fast_guest: KF_DEVICE must be kf3, got [$KF_DEVICE]" >&2; exit 2 ;;
esac
echo "== device(s): ${DEVARGS[*]}"
[ -n "$MGPU_TOK" ] && echo "== multi-GPU guest contract: $MGPU_TOK"

# ⊘⊘⊘ **P5c — THE 16550 IS A 115 200-BAUD LINK, AND THE VERBOSE TRACE WAS RUNNING INTO IT.**
# QEMU's UART paces transmission at the programmed baud rate, and 115 200 is the 16550's ceiling
# (baud base / divisor 1): ~11.5 KB/s. `[measured d1, 1a93c1df]` `--concurrency` printed 6 122
# IOCTL lines = 422 KB of serial ≈ 37 s of the 60 s budget before any work of the device's;
# `[measured c1/c2]` `--ce-client-guest-ram` spent ~20 ms per declared row printing (≈ 260 s for
# its 13 000) and `--concurrent-fuzz` PASSED in 40 s with the trace ring-buffered and TIMED OUT
# with it printed. The bare-metal lane prints nothing, so the guest was being graded against a
# UART, not against kayfabe (`a_harness_that_accuses_is_worse_than_one_that_is_silent`).
# ★ The trace stays VERBOSE (owner, 2026-09-18) — it moves to a virtio console (`hvc0`, built into
# the host kernel the fast guest boots), which has no baud pacing. The kernel's own messages go to
# BOTH consoles: hvc0 replays the whole ring when it registers, so `$SER` is still the complete
# record, and the UART log (`_ttyS0.log`) keeps what an early death prints before virtio is up.
# `KF_CONSOLE=serial` restores the old single-UART lane. `KF_APPEND="nokaslr "` (trailing space) adds
# kernel arguments (a profiling run).
case "${KF_CONSOLE:-hvc}" in
    hvc)    CONARGS=(-serial "file:${SER%_serial.log}_ttyS0.log"
                     -device virtio-serial-pci,id=kfvs0 -chardev "file,id=kfcon0,path=$SER"
                     -device virtconsole,chardev=kfcon0,bus=kfvs0.0)
            CONSOLE="console=ttyS0 console=hvc0" ;;
    serial) CONARGS=(-serial "file:$SER"); CONSOLE="console=ttyS0" ;;
    *) echo "run_fast_guest: KF_CONSOLE must be hvc or serial, got [${KF_CONSOLE}]" >&2; exit 2 ;;
esac
echo "== console: ${KF_CONSOLE:-hvc} ($CONSOLE)"

start=$(date +%s)
timeout --kill-after=3 "$BUDGET" "$Q" -name kf-fastguest \
    "${RAMARGS[@]}" -cpu host -smp "${KF_SMP:-3}" \
    -kernel "$FG/vmlinuz" -initrd "$FG/initrd.cpio.gz" \
    -append "$CONSOLE panic=1 loglevel=6 ${KF_APPEND:-}${MGPU_TOK}KF_ARMS=$ARMS_TOK KF_IOCTL_TRACE=${KF_IOCTL_TRACE:-ring} KF_BUDGET_S=$BUDGET" \
    "${DEVARGS[@]}" \
    -msg timestamp=on \
    "${CONARGS[@]}" -display none \
    > "$BENCH/fast_${TAG}_qemu.log" 2>&1
rc=$?
elapsed=$(( $(date +%s) - start ))

echo "=== FAST GUEST $TAG — ${elapsed}s of a ${BUDGET}s budget, qemu rc=$rc ==="
grep -a 'FASTGUEST:' "$SER" 2>/dev/null | sed 's/^/    /'

# ★ 124 is `timeout`'s own code. ⊘ Reported as a CRASH and not as "slow": that is the whole rule.
if [ "$rc" = 124 ] || [ "$rc" = 137 ]; then
    echo "FAST_VERDICT=CRASH (budget ${BUDGET}s exceeded — a timeout IS a crash)"
    echo "⊘ the serial log's last lines are where it stopped:"
    tail -5 "$SER" 2>/dev/null | tr -d '\r' | sed 's/^/    /'
    exit 1
fi
# ⊘⊘ **A ZERO-BYTE SERIAL LOG IS ITS OWN STATE, NOT A VERDICT.** `[measured w812]` a run shot
# by a concurrent invocation leaves an empty, freshly-timestamped log, and every signal says
# the evidence is there. Distinguish it BY NAME from a guest that booted and died.
if [ ! -s "$SER" ]; then
    echo "FAST_VERDICT=CRASH (the serial log is EMPTY — QEMU wrote nothing at all)"
    echo "⊘ This is NOT 'the guest died before reporting': nothing was ever written. The"
    echo "   usual cause is a CONCURRENT run whose pkill shot this one — the lock above"
    echo "   prevents that, so if you see this with the lock held, QEMU failed to start."
    exit 1
fi
if ! grep -aq 'FASTGUEST: DONE' "$SER" 2>/dev/null; then
    echo "FAST_VERDICT=CRASH (no DONE marker — the guest died before reporting)"
    exit 1
fi
# ⊘ The client's own rc, not QEMU's: a clean poweroff with a failing client is still a failure.
crc=$(grep -ao 'FASTGUEST: client rc=[0-9]*' "$SER" | tail -1 | grep -o '[0-9]*$')
if [ "${crc:-1}" != 0 ]; then
    echo "FAST_VERDICT=FAIL (raw client rc=${crc:-?})"
    exit 1
fi
# ★★★★★ **A GREEN RUN THAT NEVER ASKED THE GPU IS NOT A PASS.**
#
# > Owner: *"ensure that eventually it must do a real CE copy AND SCRUB on hardware."*
#
# ⊘⊘⊘ `[measured w813]` the ledger CANNOT answer that, and the proof is a control:
# `KAYFABE_CE_EXECUTOR=local` — the CPU as the only executor — passes **all five rows**. Once
# `join_one_fb_leaf` makes the guest's window and the host object ONE memory, a CPU memcpy and
# a copy-engine copy write byte-identical results, so content verification is structurally
# incapable of telling them apart.
#
# ★ The DOORBELL LEDGER can, and says so in its own words: *"`forwarded=0` with `emulated>0`
# means it never went to hardware; `forwarded>0` means it did"*. Measured on the same binary,
# same arm, same guest:
#
#     ce_executor=host   tok=0x00000003 emulated=1 forwarded=5   ⇒ the GPU ran it
#     ce_executor=local  tok=0x00000003 emulated=6 forwarded=0   ⇒ the CPU ran it
#
# ⇒ So the gate is on `forwarded`, and it turns a silently-local run from PASS into FAIL.
# ⚠ Scoped to the guest's OWN channels (`tok=0x000000xx`): the kernel/system tokens
# (`0x0001xxxx`) are `forwarded=0` BY DESIGN today — that is §12.26's system-data-plane rule,
# and whether §46 supersedes it is an open OWNER ruling, not something this gate may pre-empt.
# ⊘ Opt out with `KF_REQUIRE_FORWARD=0` for a deliberate CPU-arm control run — which must stay
# spellable, or the control this gate is built on becomes unrunnable.
QLOG="$BENCH/fast_${TAG}_qemu.log"
if [ "${KF_REQUIRE_FORWARD:-1}" = 1 ] && [ -s "$QLOG" ]; then
    # ⊘⊘ **PER TOKEN, NOT IN TOTAL — the total is too weak and a control proved it.**
    # `[measured w813b]` the CPU arm still shows **3** forwarded doorbells in total (one
    # unrelated token forwards), so a `total == 0` gate passes the very run it exists to
    # catch. The ledger's rule is stated per token and must be applied per token.
    rows=$(grep -ao 'DOORBELL-LEDGER tok=0x000000[0-9a-f][0-9a-f] .*' "$QLOG" 2>/dev/null | sort -u)
    guest_tokens=$(printf '%s\n' "$rows" | grep -c 'tok=' || true)
    fwd=$(printf '%s\n' "$rows" | sed -n 's/.*forwarded=\([0-9]*\).*/\1/p' | awk '{s+=$1} END {print s+0}')
    # A token that was RUNG (`emulated>0`) and never FORWARDED did its work off the GPU.
    # ⊘ ONE STATEMENT OF THE RULE, in `stranded_tokens.awk`, exercised GPU-free by
    # `gate_selftest.sh`. It was inline until w813d, where it rotted in the one direction a
    # gate must never rot — passing everything — and the suite score moved the RIGHT WAY while
    # it did. Run the self-test after any edit to this gate.
    stranded=$(printf '%s\n' "$rows" | awk -f "$(dirname "${BASH_SOURCE[0]}")/stranded_tokens.awk")
    # ★★★★★ **A REFUSAL BY NAME IS NOT A SILENT CPU FALLBACK — subtract the declared ones.**
    #
    # ⊘ The first version of this gate was STRICT but not TRUE, and the suite said so: it
    # failed `--missing-page-fault`, an arm whose whole point is a copy naming a page nothing
    # binds. `OPERAND-GATE refused=N` already counts exactly that — *"doorbells NOT forwarded
    # because the copy named an operand page this channel's VA space binds nowhere"* — so a
    # token stranded for THAT reason is a declared outcome, not a copy that quietly ran on the
    # CPU. `[measured w813c]` the two classes separate cleanly:
    #
    #     --missing-page-fault  refused=1  stranded=1   ⇒ fully explained
    #     --ce-client           refused=0  stranded=1   ⇒ NOT explained — a real finding
    #
    # ⚠ Conservative by construction: `refused` counts DOORBELLS and `stranded` counts TOKENS,
    # and one token can be stranded by several refusals — so `stranded > refused` means at
    # least one token cannot be accounted for, whichever way the refusals are distributed.
    # ⊘ `refuse by name means the NAME IS TRUE`: a gate that called a declared refusal a
    # hardware failure would be making exactly the error it exists to catch.
    og=$(grep -ao 'OPERAND-GATE refused=[0-9]*' "$QLOG" 2>/dev/null | tail -1 | grep -o '[0-9]*$')
    og=${og:-0}
    n_stranded=$(printf '%s\n' "$stranded" | grep -c 'tok=' || true)
    if [ "${guest_tokens:-0}" -gt 0 ] && [ "${n_stranded:-0}" -gt "$og" ]; then
        echo "FAST_VERDICT=FAIL (rows verified, but these guest channels never reached hardware)"
        printf '%s\n' "$stranded" | sed 's/^/    ⊘ /'
        echo "⊘ $n_stranded token(s) stranded, and the operand gate declared only $og refusal(s),"
        echo "   so at least one cannot be accounted for by a refusal this run made BY NAME."
        echo "⊘ Each was RUNG (emulated>0) and never FORWARDED, which is the DOORBELL LEDGER's"
        echo "   own rule for 'it never went to hardware'. The copies ran on the CPU, and the"
        echo "   content ledger CANNOT tell: once the leaf is joined, the guest's window and"
        echo "   the host object are ONE memory, so both executors write identical bytes."
        echo "⊘ If this IS the deliberate CPU control arm, re-run with KF_REQUIRE_FORWARD=0."
        echo "⚠ If a guest channel here is legitimately EMULATED (a function we implement, with"
        echo "   no GPU counterpart), this gate is telling you to say so explicitly rather than"
        echo "   letting an emulated copy pass as a forwarded one."
        exit 1
    fi
    echo "== hardware: $fwd doorbell(s) forwarded across $guest_tokens guest token(s); \
${n_stranded:-0} stranded, $og declared by the operand gate"
fi
echo "FAST_VERDICT=PASS (${elapsed}s)"
