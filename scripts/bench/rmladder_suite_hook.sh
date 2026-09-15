#!/usr/bin/env bash
# ★★★★★ THE RAW-CLIENT SUITE, RUN INSIDE THE MODE-2 GUEST — the delta that is the work list.
#
# `[measured w715, bare metal, this box]` all **30/30** arms PASS on the host. So this hook's
# output is not "does the raw client work" — it is **which arms kayfabe breaks**, and by the
# tree's own rule (`bare_metal_pass_guest_fail_indicts_kayfabe`) every delta is ours.
#
# ⊘ Pre-registered: the host reference is 30/30. Any guest arm that is not PASS is a defect in
# this port, not in the arm — unless the arm names a precondition the guest legitimately lacks,
# which it must SAY rather than merely fail.
#
# ## ⊘⊘⊘ w735 — THE HOST REFERENCE RAN AS ROOT AND THIS HOOK DID NOT
#
# `[measured w734t]` this file ran `bash /tmp/rmladder_suite.sh …` with **no `sudo`**, i.e. as
# `ubuntu`, while the 30/30 host reference and the graded `--uvm-mean` boot
# (`w392d_mean_hook.sh:93`) both ran under `sudo`. ⇒ the delta it printed was not
# host-versus-guest; it was host-as-root versus guest-as-`ubuntu`, and two of the three failures
# were on arms that spawn a namespaced child and CPU-map BAR0.
#
# ⚠ This is a **harness** correction, not a relaxation: it makes the two sides of the
# differential the same experiment. The ladder's own R16 comment already records why the uid
# matters — as root every mapping takes `RmValidateMmapRequest`'s `osIsAdministrator()` fast
# path and never executes the validation code. ⊘ `RMLADDER_SUDO=0` keeps the unprivileged run
# available, because "does it need root?" is a real question and now has a knob.
set -uo pipefail
TAG=${1:-suite}
SRC="$(cd "$(dirname "$0")" && pwd)"
G="$SRC/gssh_nv"
TMO=${RMLADDER_ARM_TIMEOUT:-90}
SUDO=$([ "${RMLADDER_SUDO:-1}" = "1" ] && echo "sudo -n " || echo "")

echo "=== push the ladder and the suite into the guest ==="
BIN=""
for c in "${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w290}"/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder \
         "${KAYFABE_REPO:-/root/kayfabe}"/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder; do
  [ -x "$c" ] && { BIN="$c"; break; }
done
if [ -z "$BIN" ]; then
  echo "SUITE_GUEST=(N) ⊘ UNMEASURED_NO_BINARY — rmladder was not built for the guest"; exit 0
fi
echo "    binary=$BIN ($(stat -c%s "$BIN") bytes)"

$G 'cat > /tmp/rmladder' < "$BIN" || { echo "SUITE_GUEST=(N) ⊘ push failed"; exit 0; }
$G 'chmod +x /tmp/rmladder'
$G 'cat > /tmp/rmladder_suite.sh' < "$SRC/rmladder_suite.sh" || true
$G 'chmod +x /tmp/rmladder_suite.sh'

# ★★★ THE GUEST'S OWN RING BUFFER, BEFORE AND AFTER. `[measured w424]` the whole
# `ce_utils.c:304` scrubber chain lives ONLY in the guest's `dmesg` at the moment of the failing
# open, and `[measured 2026-08-01]` the serial log contains no `NVRM` at all because the driver
# is `modprobe`d over ssh after boot. A suite that wedges the device and keeps no ring buffer
# has destroyed its own evidence.
BENCH=${BENCH_DIR:-/workspace/bench}
$G "${SUDO}dmesg | grep -a NVRM | tail -40" > "$BENCH/run_${TAG}_suite_dmesg_before.log" 2>&1

echo ""
echo "=== ★★★ THE SUITE IN THE GUEST (host reference: 30/30 PASS) ==="
# ⊘ `RMLADDER_ARMS` must be FORWARDED explicitly: `gssh_nv` is an ssh invocation, so the host's
# environment does not cross into the guest. `[measured w718b]` setting it on the boot command had
# no effect and all 30 arms ran anyway — the override looked armed and was not.
# ⊘ `RMLADDER_RECOVER` crosses for the same reason; `none` is the CONTROL that reproduces the
# cascade, and a run cannot claim containment without being able to switch it off.
$G "${SUDO}env RMLADDER_SUITE_LOGDIR=/tmp/suitelogs RMLADDER_ARMS='${RMLADDER_ARMS:-}' \
    RMLADDER_RECOVER='${RMLADDER_RECOVER:-modprobe}' \
    RMLADDER_OPEN_PROBE='${RMLADDER_OPEN_PROBE:-}' \
    bash /tmp/rmladder_suite.sh /tmp/rmladder 0 $TMO" 2>&1 | sed 's/^/    /' \
  | tee "$BENCH/run_${TAG}_suite.out"

echo ""
echo "=== ★★★★★ THE LEDGER — read UNMEASURED before PASS ==="
# ⊘⊘ A run that stopped is NOT a run with nothing to report. `SUITE_STARTED` without
# `SUITE_RC=` means the suite itself died — the `[measured 2026-08-10]` zero-byte-output trap,
# where "no terminator" was read as "still in flight" three times.
n_start=$(grep -ac 'SUITE_STARTED=' "$BENCH/run_${TAG}_suite.out" 2>/dev/null)
n_end=$(grep -ac 'SUITE_RC='      "$BENCH/run_${TAG}_suite.out" 2>/dev/null)
echo "SUITE_STARTED_LINES=${n_start:-0} SUITE_TERMINATOR_LINES=${n_end:-0}  (started with no terminator ⇒ the SUITE died, not the arms)"
grep -aE 'SUITE_ARMS|SUITE_RECOVERIES|SUITE_UNMEASURED_ARMS|SUITE_NOT_PASSING|SUITE_RC|SUITE_UID_NOT_ROOT|OPEN_ORDINAL_WALL' \
     "$BENCH/run_${TAG}_suite.out" 2>/dev/null | cut -c1-200

echo ""
echo "=== ★ the guest's NVRM ring buffer AFTER the suite — where a wedge names itself ==="
$G "${SUDO}dmesg | grep -a NVRM | tail -60" > "$BENCH/run_${TAG}_suite_dmesg_after.log" 2>&1
diff "$BENCH/run_${TAG}_suite_dmesg_before.log" "$BENCH/run_${TAG}_suite_dmesg_after.log" \
  | grep -a '^>' | head -30 | cut -c1-200
echo "    (full: $BENCH/run_${TAG}_suite_dmesg_after.log)"

echo ""
echo "=== ★ per-arm NVRM captures taken AT the failing open (empty ⇒ no arm was wedged) ==="
$G 'ls -l /tmp/suitelogs/*.nvrm 2>/dev/null | head -20' 2>/dev/null
$G 'for f in /tmp/suitelogs/*.nvrm; do [ -s "$f" ] && { echo "--- $f"; tail -12 "$f"; }; done' 2>/dev/null | head -60
echo "=== suite hook DONE ==="
