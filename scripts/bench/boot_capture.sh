#!/usr/bin/env bash
# ★★★ Boot the bench guest against this port's QOM shim AND PERSIST THE GUEST'S OWN dmesg.
#
#   usage: scripts/bench/boot_capture.sh <tag> [-- extra qemu args...]
#   writes: /workspace/bench/run_<tag>_serial.log   (guest console — QEMU's -serial)
#           /workspace/bench/run_<tag>_qemu.log     (the device's own stderr report)
#           /workspace/bench/run_<tag>_dmesg.log    ★ the GUEST DRIVER'S OWN output
#           /workspace/bench/run_<tag>_probe.log    (nvidia-smi, /dev nodes, modinfo)
#
# ## ★★★ Why this script exists, and what was wrong before it
#
# `boot_nvkvm.sh` boots the guest; the NVIDIA driver is then `modprobe`d **over ssh, after
# boot**, so every `NVRM:` line it emits goes to the *invoking process's* stdout and NOWHERE
# ELSE. For six consecutive boot rungs on 2026-08-01 that invoking process was an agent, and
# the transcript is not a file on disk anybody can re-read.
#
# ⚠ `[measured]` 2026-08-01, this box: `grep -ci nvrm /workspace/bench/run_*_serial.log`
# returns **0** for every one of `bar0win`, `evt1`, `l2evict1`, `gmmu1`, `alloc1`, `alloc2` —
# the serial log carries the *guest console*, and the driver's ring buffer never reaches it
# because nothing on the console reads it. So `docs/design/boot_measured_2026_08_01.md` cited
# `RmInitAdapter failed! (0x…)` lines that had **no on-disk source**. The claims were true;
# the evidence was ephemeral. This script makes the evidence a file, every boot, by default.
#
# ## ⊘ A harness that writes an empty file and exits 0 is worse than no harness
#
# That is the whole reason for the `verify` phase below. `dmesg` over ssh can fail in at
# least four ways that all leave a *file* behind — the guest never booted, ssh authenticated
# but the module was not present, `modprobe` silently no-op'd because the module was already
# loaded from a previous run, or the driver loaded and printed nothing because it never
# reached `RmInitAdapter`. Three of the four produce a file that a `[ -f ]` test is happy
# with. So this script asserts on **content**: non-empty, and containing `NVRM`. If it
# cannot, it exits non-zero and says which phase failed. ⊘ Never read a green exit from this
# script as "the boot went well" — it means only "an observation was made and stored".
#
# ## Traps encoded inline (each one has cost somebody a cycle)
#
# - ★★ `pgrep -x qemu-system-x86_64` **can never match**: `/proc/PID/comm` truncates at 15
#   characters, so the name to match is `qemu-system-x86`. A "verify none running" built on
#   the long name passes vacuously and you boot a second QEMU onto the same tap and the same
#   qcow2. `pgrep -f` is worse — it matches this script's own command line.
# - ★ The guest needs **~20-25 s** to reach a login prompt and `-serial file:` output **lags**.
#   A slow boot is not a crash. Hence a poll loop, not a fixed sleep.
# - ★ The WPR2 state of the emulated GSP only resets on a full QEMU restart, so every clean
#   run needs a **fresh boot**. This script therefore always powers the guest down at the end
#   rather than leaving it up for the next caller.
# - ★ `modprobe nvidia` when the module is already loaded prints nothing and returns 0. The
#   run is only interpretable from a *cold* module, so this script `rmmod`s first and records
#   whether it had to.
# - ★★★ **`modprobe` does not run `RmInitAdapter`.** It registers the PCI driver and prints
#   one banner line; the adapter is initialised on the first `open()` of `/dev/nvidia0`, and
#   the node does not even exist until something (`nvidia-smi`, via `nvidia-modprobe`)
#   creates it. `[measured]` run `master7d16c37`, this script's own FIRST version: it
#   captured `dmesg` after `modprobe` and got **three lines** — the nvlink banner, a vgaarb
#   line, and `loading NVIDIA UNIX Open Kernel Module`. That file contains the string `NVRM`,
#   so the content check passed and the harness exited 0 on a capture with **no adapter
#   output in it at all**. ⊘ That is precisely the failure mode this script was written to
#   prevent, reproduced by the script itself on its first run. Hence: the device is opened
#   BEFORE `dmesg` is read, and the check below is on `RmInitAdapter`, not on `NVRM`.
set -uo pipefail

BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:?usage: boot_capture.sh <tag> [-- extra qemu args...]}
shift || true
[ "${1:-}" = "--" ] && shift

LOG=$BENCH/run_${TAG}
DMESG=${LOG}_dmesg.log
PROBE=${LOG}_probe.log
BOOT_TIMEOUT=${BOOT_TIMEOUT:-150}

say() { printf '[boot_capture:%s] %s\n' "$TAG" "$*"; }
die() { printf '[boot_capture:%s] ★ FAILED (%s): %s\n' "$TAG" "$1" "${*:2}"; exit "${DIE_RC:-2}"; }

# ---- phase 0: nothing else may own the bench ----------------------------------------
if pgrep -x qemu-system-x86 >/dev/null 2>&1; then
  die precondition "a QEMU is already running (pgrep -x qemu-system-x86 matched:" \
      "$(pgrep -x qemu-system-x86 | tr '\n' ' ')). GPU/bench runs are STRICTLY SERIAL."
fi
# ---- WHICH COPY OF THE HELPERS ACTUALLY RUNS -----------------------------------------
#
# ★★★ This script used to invoke `$BENCH/boot_nvkvm.sh` and `$BENCH/gssh_nv` — the
# **box-local** copies — while the repository carries its own versioned ones right beside
# this file. That is a silent-no-op generator: edit the tree, push, boot, and the boot runs
# the *other* file.
#
# ⊘ It has already fired, and the note recording it is a workaround rather than a fix:
# `bench_rebuild_notes.md` §"the differential is the instrument" — *"`/workspace/bench/
# boot_nvkvm.sh` on this box had drifted to `-m 8G` while `scripts/bench/boot_nvkvm.sh` in
# the tree says `-m 2048`. `boot_capture.sh` runs the bench copy, so the tree's value is not
# what boots."* A whole memory-size differential was taken by `sed -i` on the box copy.
#
# So: the REPO copy wins, and the choice plus a hash of what ran is recorded in the probe log
# — because a harness that silently picks one of two files is the same defect one level up.
# `KAYFABE_BENCH_HELPERS=box` forces the old behaviour for a box that genuinely needs a
# different tap or key; it is an explicit, logged departure rather than an accident.
HELPERS=${KAYFABE_BENCH_HELPERS:-repo}
SELFDIR=$(cd "$(dirname "$0")" && pwd)
pick_helper() {
  local name=$1 repo="$SELFDIR/$1" box="$BENCH/$1"
  if [ "$HELPERS" = "repo" ] && [ -x "$repo" ]; then echo "$repo"; return; fi
  [ -x "$box" ] || die precondition "neither $repo nor $box is an executable $name"
  echo "$box"
}
BOOTER=$(pick_helper boot_nvkvm.sh) || exit $?
GSSH=$(pick_helper gssh_nv)         || exit $?

# ⊘ Delete the previous run's evidence FIRST. A stale file from an earlier tag reading as
# this boot's output is the exact failure this script exists to prevent.
rm -f "$DMESG" "$PROBE"

# ---- phase 1: boot ------------------------------------------------------------------
say "booting (extra args: ${*:-none})"
"$BOOTER" "$TAG" "$@" &
QPID=$!
trap 'kill -9 $QPID 2>/dev/null' EXIT

deadline=$(( $(date +%s) + BOOT_TIMEOUT ))
up=0
while [ "$(date +%s)" -lt "$deadline" ]; do
  if ! kill -0 $QPID 2>/dev/null; then
    say "QEMU exited early — tail of its own report:"
    tail -20 "${LOG}_qemu.log" 2>/dev/null
    die boot "QEMU pid $QPID is gone before ssh came up"
  fi
  if "$GSSH" true >/dev/null 2>&1; then up=1; break; fi
  sleep 3
done
[ "$up" -eq 1 ] || die boot \
  "guest never answered ssh within ${BOOT_TIMEOUT}s. ⊘ Prove the OBSERVER works before
   calling this a VM failure: a missing ~/.ssh key or a down tap reads identically."
say "guest is up after $(( BOOT_TIMEOUT - (deadline - $(date +%s)) ))s"

# ---- gq: EVERY guest command past this point gets a deadline AND a liveness check ----
#
# ★★★ `[measured]` 2026-08-03. Phase 1 above is careful — a deadline, and `kill -0 $QPID`
# inside the loop so a dead QEMU is named rather than waited out. **Phases 2 and 3 had
# neither.** A real-isolate boot at `dac9610` aborted QEMU on the first guest register write
# (R1 lock violation, `lockwitness.rs:125`), and this script sat at *"opening the device"*
# for **nine minutes** before I killed it by hand. Every fact needed to diagnose it was
# already in `${LOG}_qemu.log` — a full Rust backtrace ending in `kayfabe_shim_regs_write`.
#
# ⊘ **A harness that hangs reports nothing while looking like it is still working**, which is
# strictly worse than one that fails: a red run gets read, a hung one gets waited on. Same
# family as the `pgrep` trap below and as `bench_rebuild_notes.md`'s empty-`dmesg` capture —
# every waiter gets a deadline, and every deadline names what it was waiting for.
#
# ⚠ The liveness check runs FIRST. `timeout` alone would eventually return, but it would
# return a *guest* failure for what is a *hypervisor* death, and the tag's evidence would say
# "the driver did not answer" rather than "the device aborted".
GQ_TIMEOUT=${GQ_TIMEOUT:-90}
gq() {
  if ! kill -0 "$QPID" 2>/dev/null; then
    say "★ QEMU IS GONE (pid $QPID) — the guest cannot answer, and the reason is already"
    say "  in ${LOG}_qemu.log. Its tail:"
    tail -30 "${LOG}_qemu.log" 2>/dev/null | sed 's/^/    /'
    die qemu-died "QEMU exited during phase 2/3. ⊘ This is NOT a guest or driver failure;
   do not cite this tag as a boot result. Read ${LOG}_qemu.log."
  fi
  timeout "$GQ_TIMEOUT" "$GSSH" "$@"
  local rc=$?
  if [ "$rc" -eq 124 ]; then
    if ! kill -0 "$QPID" 2>/dev/null; then
      say "★ the guest command timed out AND QEMU is gone — tail of its report:"
      tail -30 "${LOG}_qemu.log" 2>/dev/null | sed 's/^/    /'
      die qemu-died "QEMU exited while a guest command was in flight. Read ${LOG}_qemu.log."
    fi
    say "★ guest command exceeded ${GQ_TIMEOUT}s while QEMU is still alive — a WEDGE, which"
    say "  is a different fact from a crash and worth keeping: $*"
  fi
  return "$rc"
}

# ---- phase 2: load the driver COLD and capture its ring buffer -----------------------
{
  echo "=== boot_capture tag=$TAG at $(date -Is) ==="
  echo "=== source revision: $(git -C "${REPO:-$(dirname "$0")/../..}" rev-parse --short HEAD 2>/dev/null || echo UNKNOWN) ==="
  echo "=== qemu binary: $(ls -l "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null) ==="
  # ★★ The archive revision comes from INSIDE the binary, never from BUILD_REV.txt.
  # `[measured]` 2026-08-03, vast 46494693: that file named a third revision while the
  # binary's own stamp named a fourth. A file that claims to record a fact is not the fact.
  echo "=== archive rev STAMPED IN THE BINARY: $(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null | grep -o 'kayfabe-rev:[0-9a-f]*' | sort -u | tr '\n' ' ')"
  echo "=== BUILD_REV.txt says (informational, NOT authoritative): $(cat "$BENCH/BUILD_REV.txt" 2>/dev/null | head -1)"
  # ★★★ WHICH helper files actually ran, with hashes — see the note at the top of phase 0.
  echo "=== helpers (KAYFABE_BENCH_HELPERS=$HELPERS) ==="
  for h in "$BOOTER" "$GSSH"; do
    echo "    used: $h  sha256=$(sha256sum "$h" 2>/dev/null | cut -c1-16)"
    b="$BENCH/$(basename "$h")"
    if [ "$h" != "$b" ] && [ -e "$b" ] && ! cmp -s "$h" "$b"; then
      echo "    ⚠ the box copy $b DIFFERS from the one that ran (sha256=$(sha256sum "$b" 2>/dev/null | cut -c1-16))."
      echo "      That divergence is exactly what used to run silently; it is now only a note."
    fi
  done
} > "$PROBE"

gq 'lsmod | grep -q "^nvidia " && echo WAS_LOADED || echo WAS_COLD' >> "$PROBE" 2>&1
gq 'sudo rmmod nvidia_drm nvidia_modeset nvidia_uvm nvidia 2>/dev/null; sudo dmesg -C' >/dev/null 2>&1
say "module unloaded and ring buffer cleared; loading cold"

gq 'sudo modprobe nvidia; echo MODPROBE_RC=$?' >> "$PROBE" 2>&1

# ★★★ OPEN THE DEVICE. `modprobe` alone only registers the PCI driver; `RmInitAdapter` —
# the entire subject of every boot rung — runs on the first `open()` of `/dev/nvidia0`, and
# that node does not exist until `nvidia-modprobe` creates it. `nvidia-smi` does both. This
# must happen BEFORE the ring buffer is read; the first version of this script read it after
# `modprobe` and captured three lines of banner.
say "opening the device (this is what runs RmInitAdapter)"
{
  echo "=== nvidia-smi (the device open) ==="
  gq 'timeout 40 nvidia-smi 2>&1; echo SMI_RC=$?'
} >> "$PROBE" 2>&1

gq 'sudo dmesg' > "$DMESG" 2>&1
{
  echo "=== /dev nodes (AFTER the open — they are created by it) ==="
  gq 'ls -la /dev/nvidia* 2>&1'
  echo "=== lsmod ===";       gq 'lsmod | grep nvidia 2>&1'
} >> "$PROBE" 2>&1

# ---- phase 3: VERIFY THE CAPTURE, before anyone reads it as evidence -----------------
if [ ! -s "$DMESG" ]; then
  die capture "$DMESG is EMPTY. The boot may have been fine; the OBSERVATION was not made,
   and nothing downstream may cite this tag."
fi
n_lines=$(wc -l < "$DMESG")
n_nvrm=$(grep -c 'NVRM' "$DMESG" || true)
# ★★★ The check is on `RmInitAdapter`, NOT on `NVRM`. The module-load banner is an `NVRM:`
# line, so an `NVRM`-based check passes on a capture in which the adapter was never touched.
# `RmInitAdapter` appears on BOTH outcomes — `RmInitAdapter failed!` on failure, and
# `RmInitAdapter: ...` / the absence of the whole block plus a working `nvidia-smi` on
# success — so this is the one string whose presence means "the subject was exercised".
n_adapter=$(grep -c 'RmInitAdapter\|rm_init_adapter' "$DMESG" || true)
if [ "$n_adapter" -eq 0 ] && ! grep -q 'SMI_RC=0' "$PROBE"; then
  say "★ $DMESG has $n_lines lines ($n_nvrm NVRM) but NOTHING from RmInitAdapter,"
  say "  and nvidia-smi did not succeed either. The adapter was never exercised, so this"
  say "  capture cannot support any claim about where the boot stops. ⊘ Do not cite it."
  DIE_RC=3 die capture "no adapter output; see $PROBE for MODPROBE_RC / SMI_RC / lsmod"
fi
say "captured $n_lines dmesg lines, $n_nvrm NVRM, $n_adapter adapter → $DMESG"
say "--- the verdict lines ---"
grep -E 'RmInitAdapter|Cannot (load state into|initialize) the device' "$DMESG" | tail -5

# ---- phase 3b: the caller's own experiment, while the guest is still UP --------------
#
# ★★★ Added for `execution_plane_increments.md` E2. `boot_capture.sh` powers the guest down
# at the end (the emulated GSP's WPR2 only resets on a full QEMU restart), so a harness that
# wants to do anything *to the running guest* has nowhere to stand: by the time this script
# returns, the machine is gone and the device's teardown report is already written.
#
# `POST_CAPTURE_HOOK` is that place. It runs with the guest up, the driver loaded and the
# device opened — i.e. after everything above has been observed and recorded — and BEFORE
# the poweroff that flushes the device's own report. Its stdout is appended to the probe
# log, so whatever it observed is on disk beside everything else.
#
# ⊘ Its exit status is recorded and does NOT abort this script: the capture above is already
# valid evidence, and losing it because an experiment failed would be the opposite of what
# this file is for. The hook's own harness is what judges the hook.
if [ -n "${POST_CAPTURE_HOOK:-}" ]; then
  say "running POST_CAPTURE_HOOK: $POST_CAPTURE_HOOK"
  {
    echo "=== POST_CAPTURE_HOOK $POST_CAPTURE_HOOK at $(date -Is) ==="
    "$POST_CAPTURE_HOOK" "$TAG"
    echo "HOOK_RC=$?"
  } >> "$PROBE" 2>&1
  say "hook finished: $(grep -c '^HOOK_RC=0$' "$PROBE" >/dev/null && echo rc=0 || echo 'rc!=0 — see the probe log')"
fi

# ---- phase 4: fresh boot next time --------------------------------------------------
say "powering down (the emulated GSP's WPR2 only resets on a full QEMU restart)"
"$GSSH" 'sudo poweroff' >/dev/null 2>&1
for _ in $(seq 1 30); do kill -0 $QPID 2>/dev/null || break; sleep 2; done
kill -9 $QPID 2>/dev/null
trap - EXIT
wait $QPID 2>/dev/null

say "done. serial=${LOG}_serial.log qemu=${LOG}_qemu.log dmesg=$DMESG probe=$PROBE"
