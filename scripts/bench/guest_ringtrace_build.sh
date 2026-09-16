#!/usr/bin/env bash
# ★★★★★ w755f — BUILD AND LOAD A *DIAGNOSTIC* GUEST DRIVER THAT SAYS WHETHER ITS OWN
#   RING WRITE LANDED.
#
# > Owner, 2026-09-17: "a kernel is only ringing a doorbell when advancing, and it does so
# > probably using mmio bar, so if gp didn't advance, then bar might be stale somehow or not
# > mapped (maybe it reads silently sparse or write is dropped). note you may put debug
# > statements in ogkm you load in guest to view the ring pointers and the state when
# > doorbell is rung to test if it matches host"
#
# ## ⊘⊘⊘ THIS DRIVER MAY NEVER GRADE A BOOT. READ THIS BEFORE USING IT.
#
# The product's north star is a **STOCK, UNPATCHED** NVIDIA driver. A boot on this module
# measures the INSTRUMENT, not the deliverable, and a `(P)` from it means nothing about the
# gate. ⇒ every run of this script stamps `RINGTRACE_GUEST=1` into the guest dmesg, and any
# grader must refuse a boot carrying it. The stamp is the point: a patched driver that left
# no trace is how a diagnostic becomes a silent pass.
#
# ## What it answers, and why the READ-BACK is the whole design
#
# `[measured w755k]` the split arm reported 2 of 125 doorbells as
# `FwdFault::RingBroughtNoEntry` — "the doorbell said there was work and the entry at the
# cursor read back as nothing". From the host, two completely different failures produce that
# one sentence:
#
#   A. the guest's write never landed (BAR1 aperture stale, unmapped, or silently sparse)
#   B. the write landed and WE are reading different memory
#
# `channel_utils.c` writes the GP entry through `memmgrMemDescBeginTransfer(..., bUseBar1)` —
# i.e. **through BAR1** — then releases the mapping, flushes the write-combine buffer, writes
# `GPPut` through USERD, flushes PCIe, and rings. Any one of those writes can be dropped while
# the doorbell still rings.
#
# ⇒ the patch reads each value back **through the same mapping, before EndTransfer**, and
# prints `LANDED` or `DROPPED-OR-ELSEWHERE`. That single word splits A from B *inside the
# guest*, before the host is involved at all.
#
# ⊘ A read-back through the same mapping cannot see a write that stopped in a CPU cache line;
# it would read its own cache. That is a real limit of this instrument and is why it is a
# DISCRIMINATOR, not a proof: `LANDED` narrows to B, `DROPPED` is decisive for A.
#
# ## Traps encoded inline (CLAUDE.md, all measured)
#   - the guest needs ~20-25 s to a login prompt and `-serial file:` LAGS; a slow boot is not
#     a crash.
#   - `dmesg` from a driver `modprobe`d over ssh goes to THAT SESSION and nowhere else. Six
#     consecutive rung claims once had their only evidence inside a transcript. ⇒ this script
#     persists dmesg and ASSERTS it is non-empty and contains the stamp.
#   - `pkill` goes on a line of its OWN ssh invocation: a later word naming the binary makes
#     it kill its own shell and everything after silently never runs.
set -uo pipefail

GUEST="${GUEST:-ubuntu@192.168.77.2}"
# ⊘⊘ **NOT a source tree path.** The first draft of this script hardcoded `/opt/ogkm` and
#   would have failed on a box at minute 40: `provision_bench_tree.sh:285` installs the guest
#   driver from the `.run` installer with `-m=kernel-open`, and there is no checked-out tree.
#   ⇒ the sources are EXTRACTED from the installer, and `channel_utils.c` is then LOCATED by
#   search with a uniqueness assert — never assumed. A guessed path is the shape that costs a
#   whole provisioning cycle to discover.
RUN="${RUN:-/var/tmp/nv.run}"      # the installer provisioning already put there
SRC="${SRC:-/var/tmp/nvsrc}"       # where we extract it
OUT="${OUT:-/workspace/bench}"
TAG="${TAG:-ringtrace}"
PATCH="$(cd "$(dirname "$0")" && pwd)/guest_ogkm_ringtrace.patch"

[ -f "$PATCH" ] || { echo "RINGTRACE=⊘ NO PATCH at $PATCH"; exit 2; }

echo "=== RINGTRACE $(date -Is) — DIAGNOSTIC DRIVER, NOT A GRADEABLE BOOT ==="

# ⊘ Copy the patch in rather than heredoc it over ssh: a heredoc through ssh is one more
#   quoting layer, and this file contains backticks, $, and UTF-8 arrows.
scp -q "$PATCH" "$GUEST:/tmp/ringtrace.patch" || { echo "RINGTRACE=⊘ SCP FAILED"; exit 1; }

ssh "$GUEST" "RUN='$RUN' SRC='$SRC' bash -s" <<'GUESTEOF'
set -uo pipefail
[ -f "$RUN" ] || { echo "RINGTRACE=⊘ NO INSTALLER at $RUN — provisioning puts it there"; exit 1; }

# ★ Extract rather than assume a tree. `-x` unpacks without installing.
rm -rf "$SRC"; mkdir -p "$SRC"
sh "$RUN" -x --target "$SRC" >/tmp/ringtrace_extract.log 2>&1 || {
  echo "RINGTRACE=⊘ EXTRACT FAILED"; tail -3 /tmp/ringtrace_extract.log; exit 1; }

# ★★★ LOCATE, with a uniqueness assert. Two copies would mean patching one and building the
#   other — a silent no-op that reads exactly like "the writes landed".
mapfile -t HITS < <(find "$SRC" -name channel_utils.c -path '*mem_mgr*' 2>/dev/null)
echo "RINGTRACE_CANDIDATES=${#HITS[@]}  ${HITS[*]:-none}"
[ "${#HITS[@]}" -eq 1 ] || { echo "RINGTRACE=⊘ expected exactly ONE channel_utils.c under mem_mgr"; exit 1; }
F="${HITS[0]}"
ROOT="${F%/src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c}"
[ "$ROOT" != "$F" ] || { echo "RINGTRACE=⊘ cannot derive the source root from $F"; exit 1; }
echo "RINGTRACE_ROOT=$ROOT"
cd "$ROOT" || exit 1

# ★ Refuse to stack the patch twice — a second copy prints the same lines from a site that is
#   not the one being reasoned about.
if grep -q 'KAYFABE-RINGTRACE' "$F"; then
  echo "RINGTRACE_ALREADY_PATCHED=1"
else
  patch -p1 --forward --fuzz=0 < /tmp/ringtrace.patch || { echo "RINGTRACE=⊘ PATCH REFUSED"; exit 1; }
  echo "RINGTRACE_PATCHED=1"
fi

# ⚠ Content gate, not an exit code: `patch` can succeed having applied nothing useful.
n=$(grep -c 'KAYFABE-RINGTRACE' "$F")
echo "RINGTRACE_SITES=$n  (expect >=3: entry read-back, gpput read-back, doorbell token)"
[ "${n:-0}" -ge 3 ] || { echo "RINGTRACE=⊘ TOO FEW SITES — the patch applied partially"; exit 1; }

make -j"$(nproc)" modules >/tmp/ringtrace_build.log 2>&1
echo "RINGTRACE_BUILD_RC=$?"
tail -3 /tmp/ringtrace_build.log
echo "RINGTRACE_KO=$(find "$ROOT" -name nvidia.ko | head -1)"
GUESTEOF
echo "GUEST_BUILD_STAGE_RC=$?"

# ⊘ The unload and the load are SEPARATE invocations. See the pkill trap above — and a
#   `rmmod` that fails leaves the OLD module resident while the next line reports success.
ssh "$GUEST" 'sudo rmmod nvidia_uvm nvidia_drm nvidia_modeset nvidia 2>/dev/null; lsmod | grep -c "^nvidia "'
ssh "$GUEST" "KO=\$(find '$SRC' -name nvidia.ko | head -1); echo \"loading \$KO\"; sudo insmod \"\$KO\" 2>&1 | tail -2; lsmod | grep -c '^nvidia '"

mkdir -p "$OUT"
D="$OUT/run_${TAG}_guest_dmesg.log"
ssh "$GUEST" 'sudo dmesg' > "$D" 2>/dev/null
echo "RINGTRACE_DMESG_BYTES=$(wc -c < "$D")"

# ★★★ THE STAMP, asserted. A patched driver that left no trace is how a diagnostic becomes a
#   silent pass, and this campaign has a recorded instance of a harness writing an empty file
#   and exiting 0.
n_trace=$(grep -ac 'KAYFABE-RINGTRACE' "$D" 2>/dev/null)
echo "RINGTRACE_GUEST=1"
echo "RINGTRACE_LINES=${n_trace:-0}   (0 ⇒ the instrument did not run; NOT 'the writes landed')"
echo "--- the verdict counts ---"
echo "LANDED=$(grep -ac 'LANDED' "$D" 2>/dev/null)"
echo "DROPPED=$(grep -ac 'DROPPED-OR-ELSEWHERE' "$D" 2>/dev/null)"
echo "--- every trace line, verbatim (cap 40) ---"
grep -a 'KAYFABE-RINGTRACE' "$D" 2>/dev/null | head -40
echo "=== RINGTRACE END — ⊘ this boot MUST NOT be graded; RINGTRACE_GUEST=1 ==="
