#!/usr/bin/env bash
# ★★★★★ **CATCH THE LONG TRAP IN THE ACT — owner, 2026-09-11:**
#   *"1 second is long enough to send SIGSTP to the kayfabe process under that trap, that
#    allows you to know exactly where its halted"*
#
# `[measured w448-w450]` `worst_trap=1 862 807us at=bar0+0x110c00` survived TWO wrong
# hypotheses of mine — "the drain holds the lock across all doorbells" (w448, no effect) and
# "one command's servicing is slow" (w450, `GSP-DRAIN` never printed, so no single command even
# reached 1 ms). ⊘ A trap that lasts 1.8 s does not need to be reasoned about. It needs to be
# looked at.
#
# ⚠ Sample, do not single-shot: the trap is ~1.8 s inside a boot of tens of seconds, so one
# well-timed look is luck. This takes N stacks and keeps the ones sitting in an MMIO write.
set -uo pipefail
# ⊘⊘⊘ **ONE MATCHER, AND NOT `pgrep`.** `[measured w455]` `pgrep -x qemu-system-x86` waited
# the full 300 s and never matched, on a boot that demonstrably ran and whose client PASSED.
# `pgrep -x` matches `/proc/PID/comm`, which is truncated to 15 characters — the exact trap
# this ledger already records for `qemu-system-x86_64` — and `pgrep -f` matches the asker's own
# command line. Both fail, in opposite directions, and both print as "no process".
#
# `ps -eo pid,comm` + an anchored awk match avoids both: comm is read as data, and our own
# shell is not named `qemu-system*`.
qemu_pid() { ps -eo pid=,comm= | awk '$2 ~ /^qemu-system/ { print $1; exit }'; }

# ★★★★★ **w470 — SAMPLE WITH `eu-stack`, NOT `gdb`, AND THE REASON IS NOT SPEED ALONE.**
# A `gdb -p` attach STOPS EVERY THREAD for as long as it takes to load symbols (seconds on a
# QEMU binary). If a trap is in flight during that stop, the stop is ADDED TO THAT TRAP'S
# MEASURED DURATION. ⇒ sampling with gdb MANUFACTURES the slow traps it is supposed to
# explain, and the run's `worst_trap` can no longer be read at all.
#
# `eu-stack` (elfutils) attaches, unwinds from CFI and detaches in milliseconds, so the
# perturbation is small enough to sample at 10 Hz — and at 10 Hz a 1.6 s hang is ~16
# CONSECUTIVE samples with one thread parked on one frame. ⊘ That is a far better detector
# than a well-timed single look: it does not depend on luck, and a run of identical stacks
# cannot be confused with a thread that merely passes through a frame often.
SNAP=${STACKSHOT_SNAP:-auto}
if [ "$SNAP" = auto ]; then
  if command -v eu-stack >/dev/null 2>&1; then SNAP=eu; else SNAP=gdb; fi
fi
echo "STACKSHOT unwinder=$SNAP"
[ "$SNAP" = gdb ] && echo "⚠ gdb sampling PERTURBS trap durations — do not read worst_trap from this run"

N=${STACKSHOT_N:-600}
GAP=${STACKSHOT_GAP:-0.1}
OUT=${STACKSHOT_OUT:-/workspace/bench/stackshots.txt}
: > "$OUT"
# ⚠ `yama/ptrace_scope=1` lets a process attach only to its own DESCENDANTS, and the QEMU we
# want is started by a different session. Root bypasses it via CAP_SYS_PTRACE, but say so
# out loud rather than discovering it as an empty file.
if [ -r /proc/sys/kernel/yama/ptrace_scope ]; then
  echo "ptrace_scope=$(cat /proc/sys/kernel/yama/ptrace_scope) euid=$(id -u)"
  [ "$(id -u)" -ne 0 ] && echo "⚠ not root with yama on — attaches to non-descendants WILL fail"
fi
echo "STACKSHOT start $(date -Is) n=$N gap=${GAP}s -> $OUT"
# ⊘⊘ **WAIT for the process, do not race it.** `[measured w453]` the sampler reported
# `samples with a backtrace: 0` AND `samples refused: 0` — it never found a process at all,
# because it started sampling the instant the boot was launched and the boot spends its first
# minute on content checks before QEMU exists. Zero attaches and zero refusals is the
# signature of a sampler that ran at the wrong time, and it prints the same as "nothing was
# ever in that state".
WAIT=${STACKSHOT_WAIT:-300}
echo "STACKSHOT waiting up to ${WAIT}s for qemu-system-x86 to appear"
for _ in $(seq 1 "$WAIT"); do
  [ -n "$(qemu_pid)" ] && break
  sleep 1
done
if [ -z "$(qemu_pid)" ]; then
  echo "⊘⊘ STACKSHOT: qemu never appeared in ${WAIT}s — NOTHING was sampled. This is a"
  echo "   harness result, not a finding about the device."
  exit 3
fi
echo "STACKSHOT qemu is up; sampling"
for i in $(seq 1 "$N"); do
  pid=$(qemu_pid)
  # ⊘ The process going away mid-run is the END of the window, not a sample to skip.
  [ -z "$pid" ] && { echo "[$i] qemu exited — sampling window over" >> "$OUT"; break; }
  # ⊘ `-batch` stops the process, dumps, and detaches. The stop is what the owner's SIGSTOP
  # does, with the backtrace we actually want on top of it.
  # ⊘⊘ **stderr is KEPT.** The first version had `2>/dev/null` here and produced a ZERO-BYTE
  # file across a whole boot — and a zero-byte file reads as *"nothing was in that state"*
  # when it actually meant *"gdb told us why and we threw it away"*. Third time this session a
  # redirect has eaten the evidence; the ledger's own rule is that an empty artefact is not
  # benign.
  echo "---- sample $i t=$(date +%s.%N) ----" >> "$OUT"
  if [ "$SNAP" = eu ]; then
    timeout 10 eu-stack -p "$pid" 2>&1 | awk -v n="$i" '{print "[" n "] " $0}' >> "$OUT"
  else
    timeout 25 gdb -p "$pid" -batch \
        -ex "set pagination off" \
        -ex "thread apply all bt 18" 2>&1 \
      | awk -v n="$i" '{print "[" n "] " $0}' >> "$OUT"
  fi
  sleep "$GAP"
done
echo "STACKSHOT done $(date -Is)"
# ★ The verdict: which frames appear in a thread that is inside our MMIO write path.
# ⊘ Say how many samples actually ATTACHED before reporting what they saw: "no frames" from
# 110 successful attaches and "no frames" from 110 refusals are opposite findings.
echo "=== attach outcome ==="
echo "  samples with a backtrace: $(grep -ac '^\[[0-9]*\] #' "$OUT")"
echo "  samples refused:          $(grep -acE 'ptrace|Operation not permitted|No such process' "$OUT")"
# ★★★★★ **THE VERDICT THAT DOES NOT NEED LUCK.** Find, per thread, the longest run of
# CONSECUTIVE samples whose top in-our-code frame is unchanged. A thread that merely calls a
# function often shows it in scattered samples; a thread PARKED in it shows a run. At 10 Hz a
# run of 16 is 1.6 s.
python3 - "$OUT" <<'PYEOF'
import re, sys, collections
samples = collections.defaultdict(dict)   # sample index -> {tid: frame}
cur = None
tid = None
for line in open(sys.argv[1], errors="replace"):
    m = re.match(r"---- sample (\d+) ", line)
    if m:
        cur = int(m.group(1)); tid = None; continue
    m = re.match(r"\[(\d+)\] TID (\d+)", line)
    if m:
        tid = m.group(2); continue
    if cur is None or tid is None:
        continue
    if tid in samples[cur]:
        continue
    # first frame naming our own code is the interesting one
    m = re.search(r"(kayfabe[A-Za-z0-9_]*(?:::[A-Za-z0-9_<>{}\.]+)+|nvkvm_[a-z0-9_]+)", line)
    if m:
        samples[cur][tid] = m.group(1)

runs = []   # (length, tid, frame, first_sample)
state = {}  # tid -> (frame, start, length)
for i in sorted(samples):
    seen = samples[i]
    for t, f in seen.items():
        prev = state.get(t)
        if prev and prev[0] == f and prev[2] + prev[1] == i:
            state[t] = (f, prev[1], prev[2] + 1)
        else:
            if prev and prev[2] >= 3:
                runs.append((prev[2], t, prev[0], prev[1]))
            state[t] = (f, i, 1)
for t, (f, st, ln) in state.items():
    if ln >= 3:
        runs.append((ln, t, f, st))
runs.sort(reverse=True)
print("=== longest CONSECUTIVE runs on one frame (>=3 samples) ===")
if not runs:
    print("  (none — no thread stayed on one of our frames across 3+ samples)")
for ln, t, f, st in runs[:12]:
    print(f"  {ln:4d} samples  tid={t}  from sample {st}  {f}")
PYEOF
echo "=== frames seen inside kayfabe MMIO/trap paths ==="
grep -aoE "kayfabe[a-z_]*::[a-zA-Z_:]+|nvkvm_[a-z_]+" "$OUT" | sort | uniq -c | sort -rn | head -25
