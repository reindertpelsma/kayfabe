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
DEPTHS=${R34_DEPTHS:-"0 2000"}
# ★★★★★ w419 — THE ADDRESS SWEEP, which is the whole question now.
#
# `[measured w419]` R34 passes in the guest at RM's own placement and passes on BARE METAL at
# every address below INCLUDING `0x7cac33600000` — the exact VA at which `CE2 HUBCLIENT_CE0`
# took `FAULT_PDE ACCESS_TYPE_VIRT_WRITE` under the LLM. Verified the dictation is not
# ignored: the rung reports `src 0x00007cac33600000`, not RM's `0x120000000`.
#
# ⇒ If any of these FAILS in the guest and passes on bare metal, the defect is kayfabe's and
# it is located to one address, with no CUDA runtime anywhere near it.
# ⊘ `rm` = RM-placed, the control. Every arm runs at decoys=0 so depth cannot confound.
ATS=${R34_ATS:-"rm 0x120000000 0x400000000 0x7cac33600000 0x768327600000 0x7f0000000000"}
# ★★★★★ w420 — THE DESTINATION'S APERTURE, which is the LLM's ACTUAL shape.
#
# The LLM's fault is a copy engine WRITING its destination, and `.to('cuda')` is an H2D
# upload: source guest RAM, destination **device memory**. R34's default puts both operands
# in sysmem, so its engine never writes to vidmem — R33's blind spot in mirror image.
# ⊘ A rung's blind spot is whatever its operands have in COMMON, and both rungs had a
# uniform pair. This arm breaks the pair.
H2D=${R34_H2D:-"1"}

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
# ★★ The marker is a string the rung PRINTS, never a flag name it parses. `[measured w420]`
# `grep -ac guest-ram-dst-vidmem` returns **0** on a binary where that flag demonstrably
# changes behaviour — rustc compiles an argument `match` to length-and-byte comparisons and
# the full literal need not survive. A content gate keyed on a flag name would refuse every
# correct binary. `refused at` is in a `println!`, so it is really there (10 occurrences).
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
  # ⊘⊘ `sudo timeout`, NOT `timeout sudo`. `[measured w419]` depth 13000 ran ~370 s past
  # `R34_TIMEOUT=300` and the hook hung with it. `timeout` signals the process it STARTED —
  # `sudo` — and sudo does not forward the signal to its child by default. So the timeout
  # kills the wrapper, the real work keeps running as root, and the ssh session blocks
  # forever on a pipe that will not close. Putting `timeout` INSIDE the privilege change
  # makes it the parent of the thing it is supposed to bound.
  #
  # ⚠ The symptom was maximally misleading: `pgrep -a` showed `[rmladder]` in brackets, which
  # reads as a kernel thread or a zombie, and the boot looked wedged rather than waiting.
  out=$($G "sudo timeout $TMO /tmp/rmladder --gpu 0 --guest-ram-decoys $d 2>&1" | grep -E 'R34' || true)
  echo "$out" | sed 's/^/  /' | cut -c1-200
  v=$(echo "$out" | grep -oE 'R34_OUTCOME=\([A-Z]\)' | tail -1)
  [ -z "$v" ] && v="R34_OUTCOME=(E)"
  verdicts="$verdicts depth$d:${v#R34_OUTCOME=}"
done

echo "R34_GUEST_VERDICTS =$verdicts"

# ★★★★★ w422 — THE OPEN ORDINAL, WHICH CONFOUNDS EVERY OTHER SWEEP IN THIS HOOK.
#
# `[measured w422, in the guest]` the address sweep read as *"the three HIGH addresses fail"*.
# They do not. With the raw output kept, all three say the same thing and it is not about an
# address at all:
#
#     FAIL  RM bring-up failed at R1 openat(nvidia<gpu>): Syscall { call: "openat", errno: 5 }
#
# EIO on opening the device node — the rung never reached a map. Count the processes: depth 0
# is the 1st open, then `rm`, `0x120000000`, `0x400000000`, and the arm that "failed" is the
# **5th**. That is the ledger's own [[the_harness_stopped_where_the_bug_starts]]: *"the 5th
# DEVICE-OPEN wedges the GPU"*.
#
# ⚠ **Every arm of this hook is a fresh process, so every arm is a device open.** Any sweep
# longer than four arms measures the wedge and attributes it to whatever the 5th arm varied.
# This block runs ONE fixed configuration N times so the ordinal is the only variable, and it
# is printed FIRST so later sweeps can be read knowing where the wall is.
ORDINALS=${R34_ORDINALS:-7}
echo "--- ★★★★★ OPEN-ORDINAL PROBE (one fixed config, repeated; the ONLY variable is the Nth open) ---"
ord_first_fail=""
for i in $(seq 1 "$ORDINALS"); do
  printf "  open #%-2s " "$i"
  oraw=$($G "sudo timeout $TMO /tmp/rmladder --gpu 0 --guest-ram-decoys 0 2>&1")
  ov=$(printf '%s' "$oraw" | grep -oE 'R34_OUTCOME=\([A-Z]\)' | tail -1)
  if [ -z "$ov" ]; then
    line=$(printf '%s' "$oraw" | grep -oE 'R1 openat[^}]*}|FAIL[^|]{0,90}' | tail -1)
    echo "(E) ${line:-no output}" | cut -c1-170
    [ -z "$ord_first_fail" ] && ord_first_fail="$i"
  else
    echo "${ov#R34_OUTCOME=}"
  fi
done
echo "R34_OPEN_ORDINAL_FIRST_FAIL =${ord_first_fail:-none-in-$ORDINALS}"
# ⊘ `none-in-N` is not "there is no wedge" — it is "not within N opens". Say which.

# ⊘ No backticks in this banner: inside double quotes they are COMMAND SUBSTITUTION. The
# first version said "`rm` = RM-placed control" and bash ran `rm`, printing
# "rm: missing operand" into the middle of the ledger. Same defect as the H2D banner's
# `.to('cuda')`, which killed that whole block with a syntax error.
echo "--- ★★★★★ ADDRESS SWEEP (decoys=0 on every arm; arm 'rm' = RM-placed control) ---"
at_verdicts=""
for a in $ATS; do
  if [ "$a" = "rm" ]; then arg=""; else arg="--guest-ram-at $a"; fi
  printf "  %-16s " "$a"
  # ⊘⊘ Capture RAW first, filter second. `[measured w421]` three arms printed a BLANK line and
  # graded `(E)`, and the evidence for why was discarded by this very `grep -oE` before anyone
  # could read it — an unmeasured arm is exactly the arm whose output you need.
  raw=$($G "sudo timeout $TMO /tmp/rmladder --gpu 0 --guest-ram-decoys 0 $arg 2>&1")
  aout=$(printf '%s' "$raw" \
         | grep -oE 'src 0x[0-9a-f]+ dst 0x[0-9a-f]+|R34_OUTCOME=\([A-Z]\)|refused at .[^`]*. by name: [A-Za-z0-9()]*' \
         | tr '\n' ' ')
  echo "$aout" | cut -c1-170
  av=$(echo "$aout" | grep -oE 'R34_OUTCOME=\([A-Z]\)' | tail -1)
  if [ -z "$av" ]; then
    av="R34_OUTCOME=(E)"
    # ⚠ Three DIFFERENT facts arrive as the same blank: the guest went away, the rung died on
    # a signal, or it printed something the filter does not know. Tell them apart.
    if ! $G true >/dev/null 2>&1; then
      echo "      ⊘ GUEST UNREACHABLE at this arm — the run was not made; says NOTHING about the address"
    else
      echo "      ⊘ no verdict, guest is UP ⇒ this is the rung's own output:"
      printf '%s\n' "$raw" | tail -6 | sed 's/^/        /' | cut -c1-190
    fi
  fi
  at_verdicts="$at_verdicts $a:${av#R34_OUTCOME=}"
done
echo "R34_AT_VERDICTS =$at_verdicts"

if [ "$H2D" = "1" ]; then
  echo "--- ★★★★★ H2D SHAPE: source GUEST RAM, destination DEVICE MEMORY (a .to(cuda) upload) ---"
  h2d_verdicts=""
  for a in rm 0x7cac33600000; do
    if [ "$a" = "rm" ]; then arg=""; else arg="--guest-ram-at $a"; fi
    printf "  h2d %-16s " "$a"
    hraw=$($G "sudo timeout $TMO /tmp/rmladder --gpu 0 --guest-ram-decoys 0 --guest-ram-dst-vidmem $arg 2>&1")
    hout=$(printf '%s' "$hraw" \
           | grep -oE 'src 0x[0-9a-f]+ dst 0x[0-9a-f]+|R34_OUTCOME=\([A-Z]\)|refused at .[^`]*. by name: [A-Za-z0-9()]*' \
           | tr '\n' ' ')
    echo "$hout" | cut -c1-170
    hv=$(echo "$hout" | grep -oE 'R34_OUTCOME=\([A-Z]\)' | tail -1)
    if [ -z "$hv" ]; then
      hv="R34_OUTCOME=(E)"
      if ! $G true >/dev/null 2>&1; then
        echo "      ⊘ GUEST UNREACHABLE at this arm — the run was not made"
      else
        echo "      ⊘ no verdict, guest is UP ⇒ the rung's own output:"
        printf '%s\n' "$hraw" | tail -6 | sed 's/^/        /' | cut -c1-190
      fi
    fi
    h2d_verdicts="$h2d_verdicts h2d-$a:${hv#R34_OUTCOME=}"
  done
  echo "R34_H2D_VERDICTS =$h2d_verdicts"
fi
# ⊘ The `src 0x…` echoed above is not decoration: a dictated address that is silently ignored
# would pass every arm identically and read as "the address does not matter". Check that the
# printed src MATCHES the arm before believing any row of this sweep.
# ⊘ One line the ledger can grade. A depth that did not report is `(E)` UNMEASURED, never a
# pass — an absent verdict has been read as a green in this tree before.
# ⊘⊘ A TIMEOUT IS NOT A REFUSAL, and grading them together would misattribute.
#
# `[measured w419]` depth 13000 declares 13 000 guest-RAM rows one RM call at a time inside
# the guest and ran past `R34_TIMEOUT` with no verdict line. The first version of this block
# folded that `(E)` in with `(F)` and printed *"BARE-METAL PASS + GUEST FAIL ⇒ indicts
# kayfabe"* — an indictment built on a stopwatch. `(F)` means an RM call said no, which is a
# claim about kayfabe; `(E)` means we ran out of time, which is a claim about the harness.
if echo "$verdicts" | grep -q '(F)'; then
  echo "R34_GUEST_OUTCOME=(F) ⊘ a depth was REFUSED in the guest — and R34 passes on BARE METAL"
  echo "  ⇒ BARE-METAL PASS + GUEST FAIL. Per the owner's ruling that indicts kayfabe, not the client."
elif echo "$verdicts" | grep -q '(E)'; then
  echo "R34_GUEST_OUTCOME=(E) ⊘ UNMEASURED — a depth produced no verdict inside R34_TIMEOUT=${TMO}s."
  echo "  ⊘ That is a HARNESS fact, not a kayfabe fact: nothing refused. Raise R34_TIMEOUT or"
  echo "  lower R34_DEPTHS, and do not read this as a failure. The depths that DID report are"
  echo "  above and each of them stands on its own."
else
  echo "R34_GUEST_OUTCOME=(P) ★ every depth moved its bytes"
fi
