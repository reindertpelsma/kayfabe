#!/usr/bin/env bash
# ★★★★★ GOAL 8 — **TWO raw clients running the mean test AT THE SAME TIME.**
#
# Owner's goal 8, verbatim: *"TWO raw clients with the mean test in parallel — needs an epoll
# loop in workers so more cuda work runs in parallel than there are workers."*
#
# ⊘⊘ **THIS SCRIPT EXISTS TO TEST THE PREMISE BEFORE BUILDING FOR IT.** The goal states a
# mechanism (`needs an epoll loop`) as well as an outcome (two clients pass). `[measured, the
# goal-8 survey]` the mechanism's premise is weaker than it looks: host verbs are **3.1 %** of a
# CUDA launch, and 1 worker / 4 workers / 4 isolates all measure **1.00x** because RM holds a
# device-global API lock across the GSP RPC. So the reactor buys **liveness isolation**, not
# throughput — and whether two clients pass TODAY is an open empirical question nobody has asked.
#
# ⇒ `reproduce_before_declaring_a_requirement`. If both clients pass here, goal 8's OUTCOME is
# already met and the reactor is an improvement rather than a prerequisite. If one times out or
# is refused, this names which — and that is the measurement the reactor must move.
#
# # ⚠ TWO PROCESSES, NOT MORE THREADS — and the distinction is the whole point
#
# `w392d_mean_hook.sh` already runs `--mean-threads 8` in ONE process. That is eight threads
# inside **one** guest process, which is **one** `ProcId`, which is **one isolate**. Goal 8 is
# about two *clients*: two processes ⇒ two `(ProcId, GpuId)` pairs ⇒ two isolate children ⇒ two
# host RM clients. Raising the thread count would measure nothing new and would read as if it had.
#
# # ⊘ What a PASS here does and does not say
#
# Same scope as the single-client hook: this client checks **statuses, not content**. Two (P)s
# mean the registration path is served for two concurrent clients; they do not mean the mappings
# are correct, and they do not measure throughput.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
KEY=/workspace/bench/guest_key
SCP_OPTS=(-i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
          -o LogLevel=ERROR -o ConnectTimeout=5)
# ⊘ The SAME default as the single-client hook. A parallel run that quietly used a shorter
# timeout would report a contention failure as a client failure.
TMO=${UVM_TIMEOUT:-300}
MEAN_THREADS=${MEAN_THREADS:-8}
MEAN_ROUNDS=${MEAN_ROUNDS:-8}

echo "=== ★★★★★ GOAL 8 — TWO RAW CLIENTS, MEAN TEST, CONCURRENT ==="
BIN=${LADDER_BIN:-/root/kayfabe/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder}
if [ ! -x "$BIN" ]; then
  # ⊘ (N) is UNMEASURED, not a failure — the w443 trap, stated the same way here.
  echo "TWOCLIENT_OUTCOME=(N) ⊘ UNMEASURED_NO_BINARY — rmladder was not built for the guest"
  echo "=== goal-8 two-client hook DONE ==="
  exit 0
fi
if ! scp "${SCP_OPTS[@]}" "$BIN" ubuntu@192.168.77.2:/tmp/rmladder >/dev/null 2>&1; then
  echo "TWOCLIENT_OUTCOME=(N) ⊘ UNMEASURED — could not copy the client into the guest"
  echo "=== goal-8 two-client hook DONE ==="
  exit 0
fi
$G 'chmod +x /tmp/rmladder' >/dev/null 2>&1

echo "    TWOCLIENT_CONFIG=threads:$MEAN_THREADS rounds:$MEAN_ROUNDS falsify:on timeout:${TMO}s"
# ★★★ Both launched from ONE ssh, backgrounded IN THE GUEST, then waited for — so they genuinely
# overlap. ⊘ Two separate ssh invocations would race on setup and could serialise without saying
# so; this way the overlap is a property of the command, not of the network's timing.
RUN='sudo timeout '"$TMO"' /tmp/rmladder --gpu 0 --uvm-mean --mean-threads '"$MEAN_THREADS"' --mean-rounds '"$MEAN_ROUNDS"' --mean-falsify'
OUT=$($G "
  echo TWOCLIENT_STARTED=\$(date -u +%FT%TZ)
  $RUN > /tmp/c1.log 2>&1 & P1=\$!
  $RUN > /tmp/c2.log 2>&1 & P2=\$!
  wait \$P1; R1=\$?
  wait \$P2; R2=\$?
  echo TWOCLIENT_RC1=\$R1
  echo TWOCLIENT_RC2=\$R2
  echo '--- CLIENT 1 ---'; cat /tmp/c1.log
  echo '--- CLIENT 2 ---'; cat /tmp/c2.log
" 2>&1 | tr -d '\r')
echo "$OUT" | sed 's/^/    /'

# ⊘ Count the verdict lines rather than trusting order: the two clients interleave on stdout only
# because we cat them in sequence, but a future change that streams them would break an
# order-dependent parse silently.
P_COUNT=$(echo "$OUT" | grep -ac "W392D_OUTCOME=(P)")
RC1=$(echo "$OUT" | sed -n 's/^TWOCLIENT_RC1=//p' | tail -1)
RC2=$(echo "$OUT" | sed -n 's/^TWOCLIENT_RC2=//p' | tail -1)
FALS=$(echo "$OUT" | grep -ac "MEAN_FALSIFIER=PASS")

echo ""
echo "TWOCLIENT_RC1=${RC1:-ABSENT} TWOCLIENT_RC2=${RC2:-ABSENT}"
echo "TWOCLIENT_PASSES=$P_COUNT of 2   TWOCLIENT_FALSIFIERS=$FALS of 2"
echo "=== ★★★★★ THE VERDICT — pre-registered, stated once ==="
if [ "${RC1:-1}" = "124" ] || [ "${RC2:-1}" = "124" ]; then
  # ⊘ 124 is the guest-side `timeout`, which is a DIFFERENT fact from the client refusing.
  echo "    TWOCLIENT_OUTCOME=(T) ★★★ ONE OR BOTH TIMED OUT at ${TMO}s — this is the"
  echo "        contention result goal 8's epoll loop exists to move. ⊘ NOT a refusal:"
  echo "        the client never reached a verdict. Re-read with UVM_TIMEOUT raised to"
  echo "        separate 'slower than the timeout' from 'wedged'."
elif [ "$P_COUNT" = "2" ] && [ "$FALS" = "2" ]; then
  echo "    TWOCLIENT_OUTCOME=(P) ★★★★★ BOTH CLIENTS PASS CONCURRENTLY — goal 8's OUTCOME is"
  echo "        met at this revision, and the reactor is an improvement rather than a"
  echo "        prerequisite. ⊘ This measures CONCURRENCY, not throughput, and statuses,"
  echo "        not content."
elif [ "$P_COUNT" = "1" ]; then
  echo "    TWOCLIENT_OUTCOME=(H) ★★★★★ EXACTLY ONE PASSED — the sharpest possible result."
  echo "        One client serialises the other out. Read which, and whether the loser was"
  echo "        refused by name or simply never scheduled."
else
  echo "    TWOCLIENT_OUTCOME=(R) ⊘ NEITHER PASSED — and a single-client boot on this same"
  echo "        revision passes, so this is about CONCURRENCY, not the client. Compare"
  echo "        against the single-client hook's ledger on the same boot before concluding."
fi
echo "=== goal-8 two-client hook DONE ==="
