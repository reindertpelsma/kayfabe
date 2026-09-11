#!/usr/bin/env bash
# ★★★★★ w418 — R34 IN THE GUEST, against a bare-metal PASS on the same box.
#
# `[measured w417, bare metal]` R34 is green at decoy depth **0** and **2000**: a CE copy
# whose SOURCE is `NV01_MEMORY_SYSTEM` moves 4096 bytes byte-correct and releases its
# semaphore. That is the half `BARE-METAL PASS + GUEST FAIL ⇒ KAYFABE BUG` needs, and it did
# not exist until the rung's own two defects were fixed (a notifier allocator used as a
# general one, and a gpu-node CPU map of a ctl-node object).
#
# ⊘ This hook supplies the OTHER half. It is deliberately NOT the LLM: R34 has no CUDA
# runtime in it, so a failure here is a rung, and the LLM's `CE2 HUBCLIENT_CE0 FAULT_PDE
# ACCESS_TYPE_VIRT_WRITE` would have a reproduction that costs seconds instead of an hour.
#
# ⚠ Two depths, always. Depth 0 asks *"does a guest-RAM CE operand work at all"* and depth
# 2000 asks *"does it survive a queue of rows ahead of it"*. `[measured w417]` those were the
# same answer for the wrong reason once already — both died before any decoy mattered — so
# neither depth alone is the measurement.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
KEY=/workspace/bench/guest_key
SCP_OPTS=(-i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
          -o LogLevel=ERROR -o ConnectTimeout=5)
TMO=${R34_TIMEOUT:-300}
DEPTHS=${R34_DEPTHS:-"0 2000 13000"}

echo "=== ★★★★★ w418 — R34 GUEST-RAM CE, IN THE GUEST ==="

BIN=${R34_BIN:-}
if [ -z "$BIN" ]; then
  for c in "${KAYFABE_REPO:-/root/kayfabe}"/target/release/kayfabe-rm-ladder \
           "${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w290}"/release/kayfabe-rm-ladder; do
    [ -x "$c" ] && BIN="$c" && break
  done
fi
if [ -z "$BIN" ]; then
  echo "R34_GUEST_OUTCOME=(N) ⊘ UNMEASURED_NO_BINARY"; exit 0
fi
# ⚠ CONTENT, never a stamp: the step-attributed error is what makes a refusal readable, and
# a binary that predates it reports `Other(31)` with no call attached.
# ⊘⊘⊘ `grep -a`, NOT `strings | grep -q` — and the reason is a MEASURED INVERSION, not style.
#
# `[measured w418, on the bench]` the first version of this check read
# `if ! strings "$BIN" | grep -q 'refused at'`, and it refused a binary that CONTAINS the
# marker 10 times. Reproduced directly:
#
#     set -uo pipefail
#     strings "$B" | grep -q "refused at"   => rc 141
#     grep -aq "refused at" "$B"            => rc 0, FOUND
#
# `grep -q` exits the instant it matches. `strings` is still writing, takes **SIGPIPE**, and
# dies with 141. Under `set -o pipefail` the PIPELINE reports 141 — so **finding the string
# faster is what makes the check fail.** The more certainly the marker is present, the more
# reliably it reports absent.
#
# ⚠ This is the sibling of *"a pipe eats the exit status"* and it is worse: that one LOSES a
# failure, this one MANUFACTURES one out of a success. Any `<producer> | grep -q` under
# pipefail has it. `grep -a` reads the binary directly — no pipe, no signal, no inversion.
if ! command -v grep >/dev/null 2>&1; then
  echo "R34_GUEST_OUTCOME=(E) ⊘ UNMEASURED — no grep to check the binary's content with"
  exit 0
fi
if ! grep -aq 'refused at' "$BIN"; then
  echo "R34_GUEST_OUTCOME=(N) ⊘ UNMEASURED — this binary predates the step-attributed refusal"
  echo "  ⊘ the marker 'refused at' is ABSENT from $BIN — this is the binary's age, not a tool"
  exit 0
fi
echo "info  R34 bin           = $BIN"

if ! $G true >/dev/null 2>&1; then
  echo "R34_GUEST_OUTCOME=(E) ⊘ UNMEASURED_GUEST_UNREACHABLE"; exit 0
fi
if ! scp "${SCP_OPTS[@]}" "$BIN" ubuntu@192.168.77.2:/tmp/rmladder >/dev/null 2>&1; then
  echo "R34_GUEST_OUTCOME=(N) ⊘ UNMEASURED — could not copy the binary into the guest"; exit 0
fi
$G 'chmod +x /tmp/rmladder' >/dev/null 2>&1

verdicts=""
for d in $DEPTHS; do
  echo "--- depth $d ---"
  out=$($G "timeout $TMO sudo /tmp/rmladder --gpu 0 --guest-ram-decoys $d 2>&1" | grep -E 'R34' || true)
  echo "$out" | sed 's/^/  /' | cut -c1-200
  v=$(echo "$out" | grep -oE 'R34_OUTCOME=\([A-Z]\)' | tail -1)
  [ -z "$v" ] && v="R34_OUTCOME=(E)"
  verdicts="$verdicts depth$d:${v#R34_OUTCOME=}"
done

echo "R34_GUEST_VERDICTS =$verdicts"
# ⊘ One line the ledger can grade. A depth that did not report is `(E)` UNMEASURED, never a
# pass — an absent verdict has been read as a green in this tree before.
if echo "$verdicts" | grep -q '(F)\|(E)'; then
  echo "R34_GUEST_OUTCOME=(F) ⊘ at least one depth did not pass — and R34 passes on BARE METAL"
  echo "  ⇒ BARE-METAL PASS + GUEST FAIL. Per the owner's ruling that indicts kayfabe, not the client."
else
  echo "R34_GUEST_OUTCOME=(P) ★ every depth moved its bytes"
fi
