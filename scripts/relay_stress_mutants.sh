#!/usr/bin/env bash
# 2026-10-20 -- mutation check of the real-thread relay stress harness (crates/kf-chan/tests/relay_stress.rs).
# Each mutant is a one-line source patch of PRODUCTION code, applied to the working tree, built, run for a seed range and then
# reverted with `git checkout`. Usage: scripts/relay_stress_mutants.sh [mutant ...]   (default: all)
# Env: RS_FROM RS_TO RS_SECS (per seed). Output: one line per mutant: failing seeds / seeds run.
set -u
cd "$(dirname "$0")/.."
FROM=${RS_FROM:-0}; TO=${RS_TO:-8}; SECS=${RS_SECS:-1.5}
FILES="crates/kf-chan/src/worker.rs crates/kf-trap/src/trap.rs crates/kf-chan/src/user_loop.rs"
patch() { python3 - "$1" <<'PY'
import sys
m=sys.argv[1]
def sub(path, old, new, cnt=1):
    s=open(path).read()
    assert old in s, (m, path, old)
    open(path,'w').write(s.replace(old,new,cnt))
W='crates/kf-chan/src/worker.rs'
if m=='control_look_removed':      # the retired behaviour (0d325edab): a busy worker never looks
    sub(W,'                AfterPass::Look => {\n','                AfterPass::Look => {\n                    if std::hint::black_box(true) { continue; }\n')
elif m=='backstop_removed':        # the safety net never peeks
    sub(W,'let _ = relay.peek(Peek::Backstop);','let _ = std::hint::black_box(0);')
elif m=='wake_not_bumped':         # a doorbell publishes the bit but never wakes a parked worker
    sub('crates/kf-trap/src/trap.rs','match self.worker_wake.bump() {','match { let _ = &self.worker_wake; Wake::NoOne } {')
elif m=='marker_not_repeeked':     # a marker is never seen as processed by any look (event, spin or backstop)
    sub('crates/kf-chan/src/user_loop.rs','Self::Marker { target } => marks >= target,','Self::Marker { target } => { let _ = target; false }')
elif m=='event_peek_removed':      # the engine event no longer re-peeks (the spin still does)
    s=open(W).read(); assert 'let _ = relay.peek(Peek::Event);' in s
    open(W,'w').write(s.replace('let _ = relay.peek(Peek::Event);','let _ = std::hint::black_box(0);'))
else:
    raise SystemExit('unknown mutant '+m)
PY
}
run() {
  name=$1; preset=${2:-random}
  git checkout -- $FILES 2>/dev/null
  [ "$name" != "none" ] && patch "$name"
  bin=$(nice -n 19 cargo test -j2 --release -p kf-chan --test relay_stress --no-run 2>&1 | grep -o '/[^ )]*release/deps/relay_stress-[0-9a-f]*' | tail -1)
  [ -x "$bin" ] || { echo "build failed for $name"; git checkout -- $FILES; return; }
  out=$(RS_PRESET=$preset RS_FROM=$FROM RS_TO=$TO RS_SECS=$SECS RS_BOUND_MS=${RS_BOUND_MS:-250} RS_HARD_MS=${RS_HARD_MS:-8000} RS_STRICT=${RS_STRICT:-1} nice -n 19 "$bin" --ignored many_seeds --nocapture 2>&1)
  echo "== mutant=$name preset=$preset: $(echo "$out" | grep RELAY_STRESS | cut -c1-300)"
  echo "$out" | grep -E "VIOLATION" | sed 's/^ *//' | cut -c1-230 | sort | uniq -c | sort -rn | head -5
  git checkout -- $FILES 2>/dev/null
}
if [ $# -eq 0 ]; then set -- none control_look_removed backstop_removed wake_not_bumped marker_not_repeeked event_peek_removed; fi
for m in "$@"; do run "$m" "${RS_PRESET:-random}"; done
