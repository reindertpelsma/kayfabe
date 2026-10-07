#!/usr/bin/env bash
# ★★★★ Run a command with the HOST kernel's Xid log watched across it, and PERSIST the log.
#
#   usage: scripts/bench/host_xid_watch.sh <tag> -- <command> [args...]
#   writes: /workspace/bench/xid_<tag>.log   ★ before / after / delta, plus the readability proof
#   exit:   0  the command succeeded AND no new Xid appeared
#           1  the command failed (its own status is preserved in the log and in $?)
#           6  ★ NEW HOST Xid across the command — the run is void whatever the command printed
#           7  ⊘ the instrument could not read the host log at all
#
# ## ★★★★ Why this exists — a host fault is INVISIBLE to every harness we have
#
# A host GPU that misses on a host VAS built from our addresses does **not** raise a guest
# fault. There is no guest involved: the host driver services it, kills the offending
# channel, and prints `NVRM: Xid (PCI:…): 31, pid=…, Ch …, MMU Fault … FAULT_PDE` into the
# **host** kernel ring buffer. Nothing else changes. The C artifact hit this twice (#13 and
# #14), and the port already knows the shape and wrote it down in two places:
#
#   `crates/kayfabe-isolate-host/src/rm.rs`  "no fault, no Xid, no RM status; only the
#                                             destination not changing says anything
#                                             happened wrongly"
#   `crates/kayfabe-isolate-host/src/rm.rs`  "zero utilisation and no Xid is the worst
#                                             failure shape available"
#
# ⇒ Without this script, **"the copy silently did nothing" and "the GPU faulted" are the
# same observation**: a semaphore that never landed, a destination that never changed, and
# every ioctl returning `NV_OK`. They call for opposite next moves — one is a reachability
# bug in our addresses, the other is a submission bug in our methods — so collapsing them
# is not a missing nicety, it is a fork in the debugging that gets guessed instead of read.
#
# ⊘ `scripts/bench/boot_capture.sh` does not cover this and cannot: it captures the
# **guest's** dmesg over ssh, which is the right instrument for a guest driver and is blind
# by construction to a fault the host serviced.
#
# ## ⊘ A harness that writes an empty file and exits 0 is worse than none
#
# That sentence is this repository's most expensive lesson and it applies here with a
# specific, nasty edge: **`dmesg` failing and `dmesg` finding no Xid produce the same empty
# grep.** `kernel.dmesg_restrict=1` (Ubuntu's default for non-root), a container without
# `CAP_SYSLOG`, or a ring buffer that has wrapped past the fault all yield "no Xid lines"
# from an instrument that measured nothing. A green from this script would then be a
# *stronger* claim than a green from no script at all, because it reads as a watched clean.
#
# ⇒ So the readability of the log is asserted POSITIVELY and SEPARATELY, before the Xid
# question is asked at all: `dmesg` must return zero AND produce a non-zero number of lines.
# If it cannot, this exits **7** and says the run is unwatched. ⊘ Never map 7 onto "clean".
#
# ★ And the assertion is on the number of lines the instrument SAW, not on the file
# existing: phase 0 records `host_dmesg_lines=N` in the log, so a reader months later can
# tell a watched-clean run from an unwatched one without re-running anything.
#
# ## Traps encoded inline (each has cost this project a cycle)
#
# - ⊘ **`grep -c` exits 1 on a zero count.** Every `grep` here is therefore either inside a
#   `$(…)` whose status is deliberately discarded, or guarded with `|| true`. A bare
#   `grep -c` under `set -e` turns "clean" into "the harness died", and a `&&`-chained one
#   turns it into a skipped assertion. `set -e` is NOT used in this file for that reason;
#   every status is checked by name.
# - ⊘ **`pgrep -f <script>` self-matches** and `pgrep -x qemu-system-x86_64` can never match
#   (`/proc/PID/comm` truncates to 15 chars). Neither is used here; the note is kept so the
#   next person who wants a "nothing else is running" check does not reinvent them.
# - ★ **The delta is a DIFF, not an after-count.** A box that already had 4 Xids from an
#   earlier run would make an absolute threshold either always-red or hand-tuned. The
#   before-snapshot is taken with the same command as the after-snapshot so the two are
#   comparable by construction.
# - ★ **`dmesg -c` is never used.** It drains the buffer, which would make this script
#   destructive to every other instrument on the box and to itself on a second run.
set -uo pipefail

OUTDIR=${XID_OUTDIR:-/workspace/bench}

if [ "$#" -lt 3 ] || [ "$2" != "--" ]; then
  echo "usage: $0 <tag> -- <command> [args...]" >&2
  exit 64
fi
TAG=$1
shift 2

mkdir -p "$OUTDIR" || { echo "★ cannot create $OUTDIR" >&2; exit 7; }
LOG="$OUTDIR/xid_${TAG}.log"

say() { echo "[xid-watch] $*"; }
# Everything this script says goes to the log AND to the caller. A watcher whose own
# reasoning is only on a terminal is the exact failure it was written to prevent.
exec > >(tee "$LOG") 2>&1

say "tag=$TAG  host=$(hostname)  utc=$(date -u +%FT%TZ)"
say "command: $*"

# ---- phase 0: ★★★ PROVE THE INSTRUMENT CAN READ, before asking it anything -------------
#
# ⊘ This phase is not a formality and it must come FIRST. "no Xid" and "cannot read the log"
# are the same empty grep, and only this phase separates them.
DMESG_RAW=$(dmesg 2>/dev/null)
DMESG_RC=$?
DMESG_LINES=$(printf '%s\n' "$DMESG_RAW" | grep -c '' || true)
if [ "$DMESG_RC" -ne 0 ] || [ "${DMESG_LINES:-0}" -lt 2 ]; then
  say "★★★ UNWATCHED — \`dmesg\` returned rc=$DMESG_RC with $DMESG_LINES line(s)."
  say "  ⊘ This run is NOT evidence of a clean Xid log; it is evidence of no instrument."
  say "  Likely: kernel.dmesg_restrict=1 (read it: sysctl kernel.dmesg_restrict), no"
  say "  CAP_SYSLOG in this container, or a non-root euid=$(id -u)."
  say "  ⊘ Do NOT rerun the measurement and quote its green: it would be equally unwatched."
  exit 7
fi
say "host_dmesg_lines=$DMESG_LINES  (readable — the clean/unwatched ambiguity is closed)"
say "host_gpu=$(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>/dev/null || echo 'nvidia-smi unavailable')"

# ★ The snapshot is the matching LINES, not a count. A count answers "how many"; the lines
# answer "which", and a new Xid's text is the whole diagnostic value (`31` vs `13` vs `109`
# are different bugs). `grep` here can legitimately match nothing — hence `|| true`.
xid_lines() { dmesg 2>/dev/null | grep -aiE 'xid|MMU Fault|fell off the bus|GPU has fallen' || true; }

BEFORE=$(xid_lines)
BEFORE_N=$(printf '%s' "$BEFORE" | grep -c '' || true)
[ -z "$BEFORE" ] && BEFORE_N=0
say "xid_before=$BEFORE_N"

# ---- phase 1: run the command ------------------------------------------------------------
say "--- command output begins ---"
"$@"
CMD_RC=$?
say "--- command output ends (rc=$CMD_RC) ---"

# ★ A fault is printed by the host driver's own bottom half and does not necessarily land
# in the ring buffer before the faulting process's ioctl returns. A zero-length settle would
# make a real Xid readable only on the NEXT run — which is how a fault gets attributed to
# the wrong rung. Two seconds, and it is a floor rather than a guess about the driver.
sleep 2

AFTER=$(xid_lines)
AFTER_N=$(printf '%s' "$AFTER" | grep -c '' || true)
[ -z "$AFTER" ] && AFTER_N=0
say "xid_after=$AFTER_N"

# ---- phase 2: the delta ------------------------------------------------------------------
NEW=$(comm -13 <(printf '%s\n' "$BEFORE" | sort) <(printf '%s\n' "$AFTER" | sort) || true)
NEW=$(printf '%s' "$NEW" | grep -v '^$' || true)

if [ -n "$NEW" ]; then
  say "★★★ NEW HOST Xid ACROSS THE COMMAND — the run is VOID whatever it printed:"
  printf '%s\n' "$NEW" | sed 's/^/    /'
  say '  ⊘ An "Xid 31 … FAULT_PDE" means the host MMU walked for an address that resolved'
  say "  to nothing — i.e. OUR placement, not the guest's methods. A rung that reports"
  say "  'the engine never fetched' beside this line has ONE cause, not two candidates."
  exit 6
fi

if [ "$AFTER_N" -ne "$BEFORE_N" ]; then
  # ★ Reached only if the counts moved and the line-diff did not — e.g. the ring buffer
  # wrapped and dropped older matches. That is not a clean run and it is not a fault; it is
  # an instrument that lost its baseline, and it must not be scored as either.
  say "★★ INCONCLUSIVE — the Xid line COUNT moved ($BEFORE_N -> $AFTER_N) and no new line"
  say "  could be diffed out. The ring buffer probably wrapped. ⊘ Not clean, not a fault."
  exit 7
fi

say "★ HOST Xid CLEAN across the command — $BEFORE_N before, $AFTER_N after, read from a"
say "  ring buffer PROVED READABLE in phase 0 ($DMESG_LINES lines). ⊘ That proof is what"
say "  separates this line from the same line printed by a blind instrument."
say "evidence: $LOG"
exit "$CMD_RC"
