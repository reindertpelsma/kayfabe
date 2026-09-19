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
Q=${QEMU_BIN:-$BENCH/qemu-build/qemu-system-x86_64}
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
pkill -9 -x qemu-system-x86 2>/dev/null
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

echo "== arms: $ARMS_TOK   budget: ${BUDGET}s   self-deadline: ${DEADLINE_MS}ms   trace: ${KF_IOCTL_TRACE:-verbose}"
# > Owner, 2026-09-18: *"Run in verbose mode so it prints the ioctls. At timeout trace dump the
# > whole thing."* ⇒ `verbose` is the default HERE and nowhere else: this lane exists to
# > diagnose hangs, and a line per ioctl over the serial console is the only record a guest that
# > never reaches poweroff leaves behind. `KF_IOCTL_TRACE=ring` when measuring a round trip,
# > where the per-call print is itself the cost.
# ⚠ **THE BINARY'S AGE, PRINTED, BECAUSE A STALE QEMU IS INVISIBLE.** The device is a Rust
# archive LINKED INTO qemu-system-x86_64, so a source change that was never relinked runs the
# OLD device while the tree says otherwise. `[measured w763]` a default flip read as "the flip
# did not take" for one whole cycle; the binary predated it by four minutes.
# ⊘ Printed rather than checked: a check would need a provenance stamp inside the binary, and
# `strings | grep -q` as a gate is a trap this campaign has already paid for.
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
KF_ROOT=${KF_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
if [ -d "$KF_ROOT/crates" ]; then
    newest=$(find "$KF_ROOT/crates" -name '*.rs' -newer "$Q" -print -quit 2>/dev/null)
    if [ -n "$newest" ]; then
        echo "run_fast_guest: ⊘⊘ REFUSED — $Q is OLDER than $newest." >&2
        echo "   The relink did not happen (or failed). Running would measure the previous" >&2
        echo "   binary and attribute the result to the change under test. Rebuild:" >&2
        echo "   KAYFABE_SHIM_FEATURES=cuda-scratchpad bash scripts/build_qom_shim.sh <src> <build>" >&2
        exit 3
    fi
fi
echo "== the archive needs cargo features: cuda-scratchpad (implies host-isolates)."
echo "   Without them KAYFABE_ISOLATES=real refuses at realize BY NAME -- that is the"
echo "   intended failure, not a kayfabe defect. Rebuild: KAYFABE_SHIM_FEATURES=cuda-scratchpad"
echo "   bash scripts/build_qom_shim.sh <qemu-src> <qemu-build>"

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

start=$(date +%s)
timeout --kill-after=3 "$BUDGET" "$Q" \
    "${RAMARGS[@]}" -cpu host -smp "${KF_SMP:-3}" \
    -kernel "$FG/vmlinuz" -initrd "$FG/initrd.cpio.gz" \
    -append "console=ttyS0 panic=1 loglevel=6 KF_ARMS=$ARMS_TOK KF_IOCTL_TRACE=${KF_IOCTL_TRACE:-verbose} KF_BUDGET_S=$BUDGET" \
    -device "nvkvm-gpu,bar1-size=$BAR1_BYTES,bar2-size=33554432,id=kf0${NVKVM_DEV_EXTRA:+,$NVKVM_DEV_EXTRA}" \
    -msg timestamp=on \
    -serial "file:$SER" -display none \
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
    stranded=$(printf '%s\n' "$rows" | awk '
        { e=0; f=0
          for (i=1;i<=NF;i++) { split($i,a,"=")
              if (a[1]=="emulated") e=a[2]; if (a[1]=="forwarded") f=a[2] }
          if (e+0 > 0 && f+0 == 0) print $1 }')
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
