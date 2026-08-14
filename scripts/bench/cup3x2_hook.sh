#!/usr/bin/env bash
# ★★★★★ w299 POST_CAPTURE_HOOK — TWO CONCURRENT CUDA PROCESSES, ON THE COMPUTE PLANE.
#
# ## The question
#
# `cup3` crossed at `^CUP3_VAL=43` (w297, master `91f8b34b`): FIRST COMPUTE, one process, one
# context. **Does it survive a second concurrent process?** That is the `#14` shape from the C
# era — two CUDA apps hang at `cuCtxCreate` — explicitly deferred to this Rust rewrite and
# never tested at the compute plane.
#
# ## ⊘ THE ARMING DOES NOT MOVE
#
# Byte for byte the w297 arming, which is byte for byte w294's. Changing the arming AND the
# process count in one step would make any outcome unattributable. The ONLY variables here are
# **how many cup3 processes run** and **when the second one starts**.
#
# ## ★★★ THE HYPOTHESIS THIS IS AIMED AT — and why the instrument is what it is
#
# The kernel-CE completion runs **synchronously inline off the doorbell**
# (`kayfabe-abi/src/eventnotify.rs:191-193`). The coordinator sharpened this mid-rung with a
# fact from our own source: that doorbell path runs **under the QEMU BQL**
# (`crates/kayfabe-qemu-raw/src/shim.rs:4877`, `:6146`, `:6046`). ⇒ blocking in the doorbell
# handler does not stall only the ringing vCPU — it stalls **every vCPU and QEMU's main loop**,
# because they all serialise on the BQL.
#
# ★★★★★ **THAT IS WHY THIS HOOK CARRIES A BEACON.** The predicted symptom is NOT "B is slow
# while A runs"; it is **both processes stopping together, and the whole guest freezing**. A
# plain "process B hung" reading CANNOT separate:
#
#   (i)  a GLOBAL freeze  — the VM stopped executing (BQL held) ⇒ the beacon GAPS TOO
#   (ii) a PER-PROCESS wait — B blocked on a lock/completion ⇒ the beacon KEEPS TICKING
#
# ⇒ `beacon.sh` is a **GPU-free** shell loop in the guest that appends a timestamp 4×/s. It
# touches no CUDA, no `/dev/nvidia*`, nothing we emulate. Its only job is to answer *"was the
# guest executing at all?"*. **A gap in the beacon is the discriminator this rung exists for.**
# ⊘ Without it, (i) and (ii) arrive as the same word — a timeout.
#
# ## ★★★ SEPARATELY IDENTIFIABLE METRICS — the trap this tree has paid for repeatedly
#
# Two processes make `^CUP3_VAL=` AMBIGUOUS. A grader that cannot tell A's value from B's is
# exactly [[a_count_cannot_see_a_substitution]]: "one 43" and "two 43s" and "43 twice from the
# same process" would all read alike. ⇒ this hook emits **`^CUP3A_VAL=` and `^CUP3B_VAL=`**,
# per process, anchored at column 0, and **never** emits a bare `CUP3_VAL=`.
#
# ⊘ The two processes are launched as `./cup3 A` and `./cup3 B`. `cup3.c` is `main(void)` and
#   ignores argv, so the PROGRAM IS BYTE-IDENTICAL (md5 asserted below) — but `/proc/<pid>/cmdline`
#   now tells the two apart, so a stall can be attributed to A or B instead of to "a cup3".
#
# ## ⚠ Traps carried forward, each already paid for in this tree
#
# - **START marker + rc file, PER PROCESS.** "exists but has no terminator" must be detectable
#   at all. `143` (the job was killed) and `124` (the LAUNCHER expired, job fine) arrive as the
#   same word otherwise — and with two processes that doubles.
# - **Liveness checked DIRECTLY, per process**, by wrapper pid (`kill -0`), not inferred from an
#   empty file. Zero bytes is not "not yet"; it is a state that needs its own check.
# - **`pgrep -f <literal>` always matches the asker** ⇒ bracket trick everywhere below.
# - **The PTX JIT is a GUEST-ENVIRONMENT precondition**, checked BY NAME before the run, so a
#   MODULE-stage failure can be attributed to the guest image rather than to us.
# - **Delete the binary before building.** `[ -x ]` cannot tell fresh from stale.
# - **Guest `dmesg` is where NVRM/Xid/soft-lockup/RCU-stall land** — NOT the serial log. A guest
#   soft-lockup or RCU stall is STRONG evidence for a BQL-held stall and beats a bare timeout.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/cup3x2_hook.sh scripts/bench/boot_capture.sh <tag>
#   env:   KAYFABE_CUP3X2_MODE=concurrent|staggered   (default concurrent)
#   ⚠ needs GQ_TIMEOUT >= 600.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
CUP3_SRC=${KAYFABE_CUP3_SRC:-$SRC_DIR/cup3.c}
CUP3_TIMEOUT=${KAYFABE_CUP3_TIMEOUT:-300}
MODE=${KAYFABE_CUP3X2_MODE:-concurrent}

die() { echo "★ cup3x2 hook FAILED: $*"; exit 2; }

case "$MODE" in
  concurrent|staggered) ;;
  *) die "KAYFABE_CUP3X2_MODE must be concurrent|staggered, got [$MODE]" ;;
esac

echo "=== ★★★★★ w299 — TWO CONCURRENT CUDA PROCESSES.  MODE=$MODE  $(date -Is) ==="
echo ""
echo "=== source (md5 — a run cannot silently be a different copy) ==="
[ -f "$CUP3_SRC" ] || die "no such source: $CUP3_SRC"
MD5=$(md5sum < "$CUP3_SRC" | cut -d' ' -f1)
printf '    %-56s %5s lines  md5 %s\n' "$CUP3_SRC" "$(wc -l < "$CUP3_SRC")" "$MD5"
echo "    ⊘ w297 ran md5 3c90b0f5f9b7deedc9d9bea471ee551a — THE SAME PROGRAM, twice over."
if [ "$MD5" = "3c90b0f5f9b7deedc9d9bea471ee551a" ]; then
  echo "    ✔ md5 MATCHES w297's cup3.c — the single-process baseline and this rung run one program."
else
  echo "    ⚠ md5 DIFFERS from w297's — any comparison to CUP3_VAL=43 is NOT like-for-like."
fi

# ---------------------------------------------------------------------------------------
# ★★★ PRECONDITION, BY NAME. A missing JIT compiler fails at cuModuleLoadData and is
#     INDISTINGUISHABLE from our wall unless it is checked separately, first.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★ GUEST PRECONDITION — the PTX JIT (cuModuleLoadData needs it) ==="
JIT=$($G 'ls -1 /usr/lib/x86_64-linux-gnu/libnvidia-ptxjitcompiler.so* 2>/dev/null | head -3; \
          ls -1 /usr/lib/libnvidia-ptxjitcompiler.so* 2>/dev/null | head -3' 2>&1 | tr -d '\r')
if [ -z "$JIT" ]; then
  echo "    ★★★ ABSENT — libnvidia-ptxjitcompiler not found in the guest."
  echo "    ⊘ A MODULE-stage failure below is therefore NOT ATTRIBUTABLE to our stack."
  JIT_OK=no
else
  echo "$JIT" | sed 's/^/    /'
  JIT_OK=yes
fi
echo "CUP3X2_JIT_PRESENT=$JIT_OK"

echo ""
echo "=== guest CPU count (two processes need somewhere to run) ==="
$G 'nproc; grep -c ^processor /proc/cpuinfo' 2>&1 | sed 's/^/    /'

echo ""
echo "=== push + build in the guest (ONE binary, run TWICE) ==="
$G 'cat > /tmp/cup3.c' < "$CUP3_SRC" || die "could not push cup3.c"
$G 'rm -f /tmp/cup3'   # no build ⇒ no file ⇒ no run
$G 'gcc -O0 -o /tmp/cup3 /tmp/cup3.c -lcuda 2>&1; echo GCC_CUP3_RC=$?'
$G 'test -x /tmp/cup3' || die "cup3 did not build in the guest"

# ---------------------------------------------------------------------------------------
# ★★★★★ THE GLOBAL LIVENESS BEACON. GPU-FREE. This is the instrument that separates a
#        VM-WIDE freeze (BQL held) from a PER-PROCESS wait. Started BEFORE either cup3 so it
#        has a quiet baseline to be compared against.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★★★ STARTING THE GLOBAL LIVENESS BEACON (GPU-free; 4 Hz; ~1000 s) ==="
echo "    ⊘ It opens no CUDA, touches no /dev/nvidia*. A GAP in it means THE GUEST STOPPED"
echo "      EXECUTING — which is the BQL prediction. No gap + a stalled cup3 = a per-process wait."
$G 'cat > /tmp/beacon.sh' <<'GUESTEOF'
#!/bin/sh
rm -f /tmp/beacon.log
echo "BEACON_START $(date -Is)" > /tmp/beacon.started
setsid sh -c 'i=0; while [ $i -lt 4000 ]; do echo "$i $(date +%s.%N)" >> /tmp/beacon.log; i=$((i+1)); sleep 0.25; done' \
       </dev/null >/dev/null 2>&1 &
echo "BEACON_LAUNCHED"
GUESTEOF
$G 'sh /tmp/beacon.sh'
sleep 4
echo "    beacon baseline sample (should be ~4 lines/s, quiet guest, no CUDA yet):"
$G 'wc -l < /tmp/beacon.log 2>/dev/null | sed "s/^/    beacon lines after ~4s = /"'

# ---------------------------------------------------------------------------------------
# The per-process launcher. $1 = name (A|B), $2 = timeout.
#   ⊘ argv carries the name so /proc/<pid>/cmdline can TELL THE TWO APART. cup3.c is
#     main(void) and ignores it — the program is byte-identical to w297's.
# ---------------------------------------------------------------------------------------
$G 'cat > /tmp/run_cup3_one.sh' <<'GUESTEOF'
#!/bin/sh
N="$1"; T="$2"
rm -f /tmp/cup3_$N.out /tmp/cup3_$N.rc /tmp/cup3_$N.wpid
echo "STARTED $(date -Is)" > /tmp/cup3_$N.started
setsid sh -c "echo \$\$ > /tmp/cup3_$N.wpid; cd /tmp && timeout $T ./cup3 $N > /tmp/cup3_$N.out 2>&1; echo \$? > /tmp/cup3_$N.rc" \
       </dev/null >/dev/null 2>&1 &
sleep 0.3
echo "LAUNCHED_$N at $(date +%s.%N) wpid=$(cat /tmp/cup3_$N.wpid 2>/dev/null)"
GUESTEOF

# ---------------------------------------------------------------------------------------
# The stall diagnostic. $1 = name. Fired WHILE the process is still alive — after it is
# killed the evidence is gone.
# ---------------------------------------------------------------------------------------
$G 'cat > /tmp/diag_cup3.sh' <<'GUESTEOF'
#!/bin/sh
N="$1"
echo "--- DIAG for cup3 $N at $(date -Is) ---"
P=""
for p in $(pgrep -x cup3 2>/dev/null); do
  if tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null | grep -q " $N "; then P=$p; fi
done
if [ -z "$P" ]; then
  echo "    ⊘ no live cup3 with argv [$N] — it is GONE (finished or killed), not stalled."
  echo "    rc file = [$(cat /tmp/cup3_$N.rc 2>/dev/null || echo NO_RC_FILE)]"
  exit 0
fi
echo "    pid=$P state=$(ps -o stat= -p $P 2>/dev/null) elapsed=$(ps -o etime= -p $P 2>/dev/null)"
echo "    cmdline=[$(tr '\0' ' ' < /proc/$P/cmdline 2>/dev/null)]"
echo "    --- per-thread (R = userspace spin; S/D = blocked IN THE KERNEL; wchan NAMES the wait) ---"
ps -L -o tid,stat,pcpu,wchan:28,comm -p $P 2>&1 | sed 's/^/      /'
echo "    --- /proc/<tid>/syscall (\"running\" = userspace, i.e. NOT in a blocking syscall) ---"
for t in /proc/$P/task/*; do
  echo "      ${t##*/}: $(sudo cat $t/syscall 2>&1 | head -c 120)"
done
echo "    --- /proc/<tid>/wchan ---"
for t in /proc/$P/task/*; do
  echo "      ${t##*/}: $(sudo cat $t/wchan 2>/dev/null)"
done
echo "    --- last line of its own output ---"
echo "      $(tail -1 /tmp/cup3_$N.out 2>/dev/null)"
if command -v gdb >/dev/null 2>&1; then
  echo "    --- gdb backtrace (bounded 60 s) ---"
  sudo timeout 60 gdb -p "$P" -batch -nx -ex "set pagination off" -ex "set confirm off" \
       -ex "thread apply all bt 14" -ex "detach" 2>&1 | head -70 | sed 's/^/      /'
else
  echo "    --- ⊘ gdb NOT PRESENT in the guest — no userspace backtrace available ---"
fi
GUESTEOF

# ---------------------------------------------------------------------------------------
# LAUNCH
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★★★ LAUNCH — mode=$MODE, each under its own ${CUP3_TIMEOUT}s timeout ==="
echo "    ⊘ NOT comparable to the cup2 180 s baseline — a different program, a longer bound."
STAGGER_ACHIEVED=n/a
if [ "$MODE" = concurrent ]; then
  echo "    both launched back-to-back in ONE ssh call (skew ~0.3 s)"
  $G 'sh /tmp/run_cup3_one.sh A '"$CUP3_TIMEOUT"'; sh /tmp/run_cup3_one.sh B '"$CUP3_TIMEOUT" 2>&1 | sed 's/^/    /'
else
  # ★ The C's #14 reproduced at cuCtxCreate, not at launch. So B starts while A is
  #   DEMONSTRABLY INSIDE cuCtxCreate — gated on A's OWN output, never on a fixed sleep.
  echo "    A first; B starts when A is DEMONSTRABLY INSIDE cuCtxCreate (gated on A's own print)"
  $G 'sh /tmp/run_cup3_one.sh A '"$CUP3_TIMEOUT" 2>&1 | sed 's/^/    /'
  echo "    waiting for A to enter cuCtxCreate (it has printed cuDeviceGet, not yet CTX OK)..."
  STAGGER_ACHIEVED=no
  for i in $(seq 1 60); do
    ST=$($G 'if grep -q "cuDeviceGet(&d,0)" /tmp/cup3_A.out 2>/dev/null; then \
               if grep -q "^CTX OK" /tmp/cup3_A.out 2>/dev/null; then echo PAST; else echo INSIDE; fi; \
             else echo BEFORE; fi' 2>/dev/null | tr -d '\r\n ')
    if [ "$ST" = INSIDE ]; then STAGGER_ACHIEVED=yes; echo "    ✔ A is INSIDE cuCtxCreate after ~$((i))s — launching B NOW"; break; fi
    if [ "$ST" = PAST ]; then STAGGER_ACHIEVED=no-A-already-past; echo "    ⚠ A passed CTX OK after ~$((i))s before B started — the stagger did NOT land as intended"; break; fi
    sleep 1
  done
  $G 'sh /tmp/run_cup3_one.sh B '"$CUP3_TIMEOUT" 2>&1 | sed 's/^/    /'
fi
echo "CUP3X2_STAGGER_ACHIEVED=$STAGGER_ACHIEVED"

# ---------------------------------------------------------------------------------------
# POLL. Report BOTH ladders as they advance, and fire the diagnostic at the moment the
# (B) shape forms — one finished, one not — because that is when the evidence still exists.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== waiting for BOTH terminators (poll $((CUP3_TIMEOUT + 90))s) ==="
LIMIT=$(( (CUP3_TIMEOUT + 90) / 10 ))
DIAG_FIRED=no
LASTA=""; LASTB=""
for i in $(seq 1 "$LIMIT"); do
  S=$($G 'a=no; b=no; test -f /tmp/cup3_A.rc && a=yes; test -f /tmp/cup3_B.rc && b=yes; \
          la=$(tail -1 /tmp/cup3_A.out 2>/dev/null); lb=$(tail -1 /tmp/cup3_B.out 2>/dev/null); \
          echo "RC_A=$a RC_B=$b"; echo "A|$la"; echo "B|$lb"' 2>/dev/null | tr -d '\r')
  RCA=$(echo "$S" | grep -o 'RC_A=[a-z]*' | cut -d= -f2)
  RCB=$(echo "$S" | grep -o 'RC_B=[a-z]*' | cut -d= -f2)
  NA=$(echo "$S" | sed -n 's/^A|//p'); NB=$(echo "$S" | sed -n 's/^B|//p')
  if [ "$NA" != "$LASTA" ] || [ "$NB" != "$LASTB" ]; then
    echo "    [~$((i*10))s] A: ${NA:-<no output yet>}"
    echo "    [~$((i*10))s] B: ${NB:-<no output yet>}"
    LASTA="$NA"; LASTB="$NB"
  fi
  if [ "$RCA" = yes ] && [ "$RCB" = yes ]; then
    echo "    ✔ BOTH terminators present after ~$((i*10))s"; break
  fi
  # ★★★ THE (B) SHAPE IS FORMING — exactly one finished. Diagnose the OTHER one NOW.
  if [ "$DIAG_FIRED" = no ] && { [ "$RCA" = yes ] || [ "$RCB" = yes ]; }; then
    DIAG_FIRED=yes
    OTHER=A; [ "$RCA" = yes ] && OTHER=B
    echo ""
    echo "=== ★★★ ONE PROCESS TERMINATED AND THE OTHER DID NOT — DIAGNOSING [$OTHER] LIVE ==="
    echo "    (RC_A=$RCA RC_B=$RCB at ~$((i*10))s) ⊘ this is the pre-registered (B) shape forming."
    $G "sh /tmp/diag_cup3.sh $OTHER" 2>&1 | sed 's/^/    /'
    echo "=== ★ beacon state AT THIS MOMENT — is the GUEST still executing? ==="
    $G 'wc -l < /tmp/beacon.log; tail -2 /tmp/beacon.log' 2>&1 | sed 's/^/    /'
    echo ""
  fi
  sleep 10
done

# ---------------------------------------------------------------------------------------
# ★ FINAL LIVENESS, DIRECTLY. Not inferred from an empty file.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★ DIRECT LIVENESS CHECK, PER PROCESS (⊘ never inferred from an empty file) ==="
$G 'for N in A B; do
      W=$(cat /tmp/cup3_$N.wpid 2>/dev/null)
      if [ -n "$W" ] && kill -0 "$W" 2>/dev/null; then L=ALIVE; else L=GONE; fi
      echo "CUP3${N}_WRAPPER=$L wpid=${W:-NONE} started=[$(cat /tmp/cup3_$N.started 2>/dev/null)]"
      echo "CUP3${N}_OUT_BYTES=$(wc -c < /tmp/cup3_$N.out 2>/dev/null || echo NOFILE)"
    done
    echo "LIVE_CUP3_PIDS=[$(pgrep -x cup3 2>/dev/null | tr "\n" " ")]"' 2>&1 | sed 's/^/    /'

# ---------------------------------------------------------------------------------------
# ★★★★★ THE GRADED LINES — ANCHORED, and SEPARATELY IDENTIFIABLE PER PROCESS.
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★ EACH PROCESS'S FINAL OUTPUT, verbatim ==="
for N in A B; do
  echo "--- cup3 $N ---"
  $G "cat /tmp/cup3_$N.out 2>/dev/null" | sed 's/^/    /'
done

echo ""
for N in A B; do
  RC=$($G "cat /tmp/cup3_$N.rc 2>/dev/null" 2>/dev/null | tr -d '\r\n ')
  KLINE=$($G "grep -h '^KERNEL rv=' /tmp/cup3_$N.out 2>/dev/null | tail -1" 2>/dev/null | tr -d '\r')
  RV=$(printf '%s' "$KLINE" | sed -n 's/^KERNEL rv=\([0-9]*\) .*/\1/p')
  echo "CUP3${N}_RC=${RC:-NO_RC_FILE}"
  echo "CUP3${N}_VAL=${RV:-NO_KERNEL_LINE}"
  echo "CUP3${N}_KERNEL_LINE=${KLINE:-ABSENT}"
done
echo "    ⊘ NO bare 'CUP3_VAL=' is emitted by this hook, deliberately: with two processes it"
echo "      would be ambiguous, and an ambiguous headline is the substitution failure itself."

echo ""
echo "=== ★★★★★ THE VALUE LADDER, PER PROCESS ==="
for N in A B; do
  KLINE=$($G "grep -h '^KERNEL rv=' /tmp/cup3_$N.out 2>/dev/null | tail -1" 2>/dev/null | tr -d '\r')
  RV=$(printf '%s' "$KLINE" | sed -n 's/^KERNEL rv=\([0-9]*\) .*/\1/p')
  case "${RV:-}" in
    43)    echo "    $N: ★★★★★ 43 — the shader ran. UN-FORGEABLE (no copy/fill/forged completion makes 43)." ;;
    14)    echo "    $N: ★★★ 14 — THE INPUT CAME BACK. Something COPIED where a COMPUTE belonged." ;;
    0)     echo "    $N: ⊘ 0 — the cuMemsetD32 landed and the kernel wrote nothing." ;;
    61166) echo "    $N: ⊘ 61166 (0xeeee) — HOST SENTINEL INTACT; cuMemcpyDtoH never wrote." ;;
    "")    echo "    $N: ⊘ NO KERNEL LINE — UNMEASURED, NOT a failing value. Read its ladder." ;;
    *)     echo "    $N: ⚠ ${RV} — a value no stage produces by design. Reported raw." ;;
  esac
done

echo ""
echo "=== ★ THE STAGE LADDER, PER PROCESS (the furthest ✔ before the first ✘ is the wall) ==="
for N in A B; do
  echo "    --- cup3 $N ---"
  for s in "CTX OK" "MODULE OK" "FUNC OK" "MEMALLOC" "LAUNCH OK" "SYNC OK" "KERNEL rv=" "DONE"; do
    if $G "grep -q '^$s' /tmp/cup3_$N.out 2>/dev/null"; then echo "    ✔ $N $s"; else echo "    ✘ $N $s"; fi
  done
  echo "    --- $N's own FAIL line, if it named its failure ---"
  $G "grep -h '^FAIL' /tmp/cup3_$N.out 2>/dev/null" | sed 's/^/      /'
done

# ---------------------------------------------------------------------------------------
# ★★★★★ THE BEACON VERDICT — GLOBAL FREEZE vs PER-PROCESS WAIT
# ---------------------------------------------------------------------------------------
echo ""
echo "=== ★★★★★ THE BEACON — DID THE WHOLE GUEST STOP, OR ONLY A PROCESS? ==="
$G 'cat /tmp/beacon.log 2>/dev/null' > /tmp/w299_beacon.log 2>/dev/null
BL=$(wc -l < /tmp/w299_beacon.log 2>/dev/null || echo 0)
echo "    beacon samples captured = [$BL]   (⊘ 0 ⇒ THE BEACON NEVER RAN; every reading below is VACUOUS, not zero)"
if [ "$BL" -gt 10 ]; then
  awk '{ t=$2+0; if (prev>0) { d=t-prev; if (d>max) { max=d; maxat=prev }
           if (d>1.0) { n1++; if (nl<12) { printf "      GAP %8.3f s  at epoch %.3f (sample %d)\n", d, prev, $1; nl++ } } }
         prev=t }
       END { printf "    max inter-sample gap = %.3f s   (nominal 0.25 s)\n", max;
             printf "    gaps > 1.0 s         = %d\n", n1+0;
             printf "    beacon span          = %.1f s\n", t-first }
       NR==1 { first=$2+0 }' /tmp/w299_beacon.log
  echo ""
  echo "    ★★★ HOW TO READ THIS, pre-registered so neither answer can be the favourable one:"
  echo "      • a stalled cup3 WITH beacon gaps of the same order ⇒ GLOBAL freeze — consistent"
  echo "        with the BQL mechanism (doorbell handler holds the BQL; all vCPUs + main loop stop)."
  echo "      • a stalled cup3 with the beacon TICKING THROUGHOUT ⇒ a PER-PROCESS wait; the BQL"
  echo "        hypothesis is REFUTED for this shape and the wait is on a lock/completion."
  echo "      • both processes green + no gaps ⇒ inline is fine AT THIS WORK SIZE. ⊘ That is a"
  echo "        real result: the hazard would be LATENT, arriving when CE work is forwarded to a"
  echo "        real engine and the inline span grows."
else
  echo "    ⊘ TOO FEW SAMPLES TO ANALYSE — the beacon is UNMEASURED. Do not read this as 'no gaps'."
fi

echo ""
echo "=== ★★ GUEST dmesg — soft-lockup / RCU stall / NVRM / Xid (⊘ NOT in the serial log) ==="
echo "    ★ a soft-lockup or RCU stall here is STRONG evidence of a BQL-held stall and beats a"
echo "      bare timeout, which has no attribution at all."
$G 'sudo dmesg | grep -iE "soft lockup|softlockup|rcu.*stall|rcu_sched|hung task|blocked for more than|watchdog|NVRM|Xid" | tail -40' 2>&1 | sed 's/^/    /'
echo "    --- unfiltered tail, for context ---"
$G 'sudo dmesg | tail -40' 2>&1 | sed 's/^/    /'
echo "=== cup3x2 hook DONE $(date -Is) ==="
