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
set -uo pipefail
TAG=${1:-suite}
G="$(cd "$(dirname "$0")" && pwd)/gssh_nv"
TMO=${RMLADDER_ARM_TIMEOUT:-90}

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
$G 'cat > /tmp/rmladder_suite.sh' < "$(cd "$(dirname "$0")" && pwd)/rmladder_suite.sh" || true
$G 'chmod +x /tmp/rmladder_suite.sh'

echo ""
echo "=== ★★★ THE SUITE IN THE GUEST (host reference: 30/30 PASS) ==="
$G "RMLADDER_SUITE_LOGDIR=/tmp/suitelogs bash /tmp/rmladder_suite.sh /tmp/rmladder 0 $TMO" 2>&1 | sed 's/^/    /'

echo ""
echo "=== ★★★★★ THE DELTA — every non-PASS is a kayfabe defect, the host passes all 30 ==="
$G 'grep -E "SUITE_ARMS|SUITE_NOT_PASSING|SUITE_RC" /tmp/suitelogs/../suite.out 2>/dev/null' 2>/dev/null || true
echo "=== suite hook DONE ==="
