#!/usr/bin/env bash
# ★★★★★ w290p — LEG 8, THE WHOLE-VAS PUBLICATION. One boot per arm.
#
#   $1 = assert | publish     (⊘ REQUIRED — never defaulted, for the same reason the arm
#                              itself is not: a defaulted arm makes an evidence run and its
#                              own control indistinguishable)
#
# ⊘ RELAXED ARM, LABELLED: KAYFABE_PT_SWEEP=on + KAYFABE_OPERAND_JOIN=join carried byte for
#   byte from w290cup2, plus KAYFABE_VAS_PUBLISH=$1. A relaxed green is never the milestone.
#
# ★★★ PRE-REGISTERED, BEFORE THE BOOT — all four outcomes, so none of them can be read as
#     the favourable one after the fact:
#   (1) TOO SLOW      ⇒ report wall ms and the leaf count. Working-but-slow is a SUCCESS with
#                       an optimization problem. The wall budget prints when it fires and
#                       says an unpublished row is NOT a refused one.
#   (2) EXHAUSTS      ⇒ name the limit and the number (RM status, host VAS, descriptors).
#                       `refused=` is per-leaf and `join_one_fb_leaf` prints each by name.
#   (3) PUBLISHED AND STILL FAULTS ⇒ publication was NECESSARY AND NOT SUFFICIENT. Name the
#                       next wall BY IDENTITY: Xid, address, engine, access, descent level.
#                       ★ This is a FULL RESULT and it is the honest possibility.
#   (4) THE ROWS ARE NOT ASKABLE ⇒ RM places fixed mappings in 64 KiB granules and the verb
#                       takes exactly one table row, so a 4 KiB row cannot be published at
#                       all. `not_granular=` measures this. If it dominates, THE BUILD IS
#                       BLOCKED BY RM's GRANULARITY and that is the result — not a failure to
#                       report.
set -uo pipefail
ARM="${1:-}"
case "$ARM" in
  assert|publish|pinrate|both|drain) ;;
  *) echo "usage: $0 assert|publish|pinrate|both|drain" >&2; exit 64 ;;
esac
TAG=w290p${ARM}
OUT=/workspace/${TAG}.log
exec >"$OUT" 2>&1
finish() { echo "=== W290P EXIT rc=$1 arm=$ARM at $(date -Is) ==="; exit "$1"; }
echo "=== W290P START arm=$ARM $(date -Is) pid=$$ ==="
export PATH=/root/.cargo/bin:$PATH
# ⊘ Overridable so a lane can use ITS OWN clone and ITS OWN target dir (the standing rule), and
#   DEFAULTED to w290's so an existing caller keeps behaving byte-identically.
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w290}
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD); echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w290}
export KAYFABE_SHIM_FEATURES=host-isolates

# ★★★ DELETE THE SHIM FIRST: no build => no file => no run.
rm -f /workspace/bench/qemu-build/qemu-system-x86_64
echo "=== BUILD SHIM $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?; echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 2>/dev/null | grep -oE "kayfabe-rev:[0-9a-f]{40}" | head -1 | cut -d: -f2)
echo "=== STAMP=$STAMP HEAD=$HEAD ==="
[ "$STAMP" = "$HEAD" ] || { echo "=== ★★★ STAMP GATE FAIL ==="; finish 93; }

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export GQ_TIMEOUT=420
export BOOT_TIMEOUT=180
export KAYFABE_FB_JOIN=shared
export KAYFABE_GUEST_RING=ring
export KAYFABE_GUEST_PUSHBUF=pin
export KAYFABE_PT_WITNESS_EXEC=on
export KAYFABE_GUEST_SEMA=pin
export KAYFABE_GR_ROUTE=passthrough
export KAYFABE_GUEST_OPERAND=pin
export KAYFABE_PT_SWEEP=on          # ⊘ RELAXATION 1 (carried)
export KAYFABE_OPERAND_JOIN=join    # ⊘ RELAXATION 2 (carried)
export KAYFABE_VAS_PUBLISH=$ARM     # ★★★★★ THE RUNG
unset KAYFABE_RING_VIDMEM

export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_gdbspin.sh
echo "=== BOOT $TAG START $(date -Is)  HOOK=[cup2_hook_gdbspin] ==="
timeout 1800 "$REPO/scripts/bench/boot_capture.sh" "$TAG"
echo "=== BOOT $TAG RC=$? $(date -Is) ==="
Q=/workspace/bench/run_${TAG}_qemu.log
P=/workspace/bench/run_${TAG}_probe.log
D=/workspace/bench/run_${TAG}_hostdmesg.log

echo "=== ★ THE ARMING ACTUALLY IN FORCE (a boot happening is not an arm running) ==="
grep -oE 'VAS-PUBLISH arm=[a-z]+ fb_join=[a-z]+ host_isolates=[a-z]+' "$Q" 2>/dev/null | head -1 | sed 's/^/      /'
grep -oE 'OPERAND-JOIN arm=[a-z]+' "$Q" 2>/dev/null | head -1 | sed 's/^/      /'
grep -oE 'PT-SWEEP tasks=[0-9]+ skipped=[0-9]+ ran=[0-9]+' "$Q" 2>/dev/null | tail -1 | sed 's/^/      /'
echo "    ⊘ VAS-PUBLISH lines total = [$(grep -c 'VAS-PUBLISH token=' "$Q" 2>/dev/null)] — if 0 the pass NEVER RAN and every zero below is VACUOUS"
echo "    ⊘ NOT ARMABLE lines       = [$(grep -c 'VAS-PUBLISH.*NOT ARMABLE' "$Q" 2>/dev/null)]  (MUST be 0)"
echo "    ⊘ host_isolates=NO stub   = [$(grep -c 'VAS-PUBLISH.*host_isolates=NO' "$Q" 2>/dev/null)]  (MUST be 0)"

echo ""
echo "=== ★★★★★ LEG 8 — THE LAST EMISSION, VERBATIM ==="
grep -o 'VAS-PUBLISH token=.*' "$Q" 2>/dev/null | tail -1 | fold -w 200 | sed 's/^/      /'
echo "--- the FIRST emission (the picture BEFORE anything was published):"
grep -o 'VAS-PUBLISH token=.*' "$Q" 2>/dev/null | head -1 | fold -w 200 | sed 's/^/      /'
echo "--- published / refused / wall, every distinct value seen:"
grep -oE 'published=[0-9]+ refused=[0-9]+ in [0-9]+ ms' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -12 | sed 's/^/      /'
echo "--- ★★★★★ w291 PINRATE (only on arm=pinrate; ⊘ MEASUREMENT, NOT THE MERGE):"
grep -o 'PINRATE(w291.*' "$Q" 2>/dev/null | tail -1 | fold -w 200 | sed 's/^/      /'
echo "    ⊘ PINRATE emissions = [$(grep -c 'PINRATE(w291' "$Q" 2>/dev/null)] — 0 on this arm means UNMEASURED, not 0 ms"
echo "    per-row + degrade, every distinct reading:"
grep -oE 'pinned=[0-9]+ refused=[0-9]+ in [0-9]+ ms, per_row=[^,]*, degrade\[[^]]*\]' "$Q" 2>/dev/null | sort -u | head -8 | sed 's/^/      /'
echo "    ⊘ rows the VMM would not resolve = [$(grep -o 'UNRESOLVED-BY-VMM' "$Q" 2>/dev/null | wc -l)]  (never reached the verb — NOT pin failures)"
echo "    ⊘ REFUSED pins, by name          = [$(grep -o '⊘REFUSED `[^`]*`' "$Q" 2>/dev/null | sort | uniq -c | tr '\n' ' ')]"
echo ""
echo "=== ★★★★★ w292 THE DRAIN — SCOPED TO THE DOORBELLED VAS, AND ITS COST ON THE DOORBELL PATH ==="
echo "⊘ On any arm but \`drain\` every clause below MUST read \`⊘ NOT ARMED\`. If it does on"
echo "  arm=drain, the rung DID NOT RUN and every number after it is VACUOUS, not zero."
echo "--- SCOPE[...] — which VAS was the target, or by NAME why there was none:"
grep -o 'SCOPE\[[^]]*\]' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -6 | sed 's/^/      /'
echo "--- DRAIN[...] — every distinct reading (visited / asked / pinned / refused / DRAIN_MS / complete):"
grep -o 'DRAIN\[[^]]*\]' "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -12 | sed 's/^/      /'
echo "    ⊘ DRAIN[ ] emissions total = [$(grep -c 'DRAIN\[' "$Q" 2>/dev/null)] — 0 means the pass NEVER PRINTED, so nothing below is measured"
echo "    ⊘ TARGET NEVER VISITED     = [$(grep -c 'TARGET NEVER VISITED' "$Q" 2>/dev/null)]  (pinned=0 there is UNREACHED, not 'already complete')"
echo "    ⊘ complete=false           = [$(grep -o 'complete=false' "$Q" 2>/dev/null | wc -l)]   complete=true = [$(grep -o 'complete=true' "$Q" 2>/dev/null | wc -l)]"
echo "    ⚠ DRAIN WALL BUDGET hits   = [$(grep -c 'DRAIN WALL BUDGET' "$Q" 2>/dev/null)]   DRAIN ROW CAP hits = [$(grep -c 'DRAIN ROW CAP' "$Q" 2>/dev/null)]"
echo "--- ★ THE COST ON THE DOORBELL PATH — every distinct DRAIN_MS, and the largest:"
grep -oE 'DRAIN_MS=[0-9]+' "$Q" 2>/dev/null | sort -t= -k2 -n | uniq -c | tail -12 | sed 's/^/      /'
echo "      max DRAIN_MS = [$(grep -oE 'DRAIN_MS=[0-9]+' "$Q" 2>/dev/null | cut -d= -f2 | sort -n | tail -1)]"
echo "      ⊘ this is WALL TIME of the drained VAS's loop (VMM resolve + verb + refusals), not a sum of pin times"
echo "--- ★★ THE DRAINED VAS's own row — last emission, verbatim (last_pinned_va says HOW FAR it got):"
grep -o '\[proc=[0-9]* pdb=0x[0-9a-f]* ★DRAINED[^]]*\]' "$Q" 2>/dev/null | tail -2 | fold -w 200 | sed 's/^/      /'
echo "    ⊘ ★DRAINED row emissions = [$(grep -c '★DRAINED' "$Q" 2>/dev/null)]  (0 on arm=drain ⇒ the target was never reached)"
echo ""
echo "--- ⚠ wall-budget exhaustions = [$(grep -c 'WALL BUDGET' "$Q" 2>/dev/null)]  (an unpublished row is NOT thereby refused)"
echo "--- ⊘ bucket-identity violations (sum_ok=false) = [$(grep -o 'sum_ok=false' "$Q" 2>/dev/null | wc -l)]  (MUST be 0)"
echo "--- every REFUSED leaf, by name (this is where 'exhausts something' would appear):"
grep -oE 'VAS-PUBLISH\(proc=[0-9]+ pdb=0x[0-9a-f]+\) leaf va=0x[0-9a-f]+.{0,110}' "$Q" 2>/dev/null | grep -i 'REFUSED\|COULD NOT' | sort | uniq -c | sort -rn | head -10 | sed 's/^/      /'

echo ""
echo "=== ★★★★★ THE INDEPENDENT PROOF — HOST-PUBLISHED, BEFORE vs AFTER ==="
echo "⊘ This number proves the build did its job whether or not cup2 passes."
echo "--- FIRST emission:"
grep -o 'HOST-PUBLISHED .*' "$Q" 2>/dev/null | head -1 | fold -w 200 | sed 's/^/      /'
echo "--- LAST emission:"
grep -o 'HOST-PUBLISHED .*' "$Q" 2>/dev/null | tail -1 | fold -w 200 | sed 's/^/      /'
echo "--- every distinct host_rows= reading:"
grep -oE 'host_rows=[0-9]+ of [0-9]+' "$Q" 2>/dev/null | sort -u | head -20 | sed 's/^/      /'

echo ""
echo "=== ⚠ THE PRE-REGISTERED COST — a published row is FROZEN against the guest's own edits ==="
echo "    RepointsPublished = [$(grep -o 'RepointsPublished' "$Q" 2>/dev/null | wc -l)]"
echo "    UnbindsPublished  = [$(grep -o 'UnbindsPublished' "$Q" 2>/dev/null | wc -l)]"
echo "    (⊘ these are refusal-KIND mentions across the log, not distinct VAs — the by_kind"
echo "     map on the sweep line carries the per-doorbell counts)"
grep -o 'by_kind={[^}]*}' "$Q" 2>/dev/null | tail -1 | sed 's/^/      /'

echo ""
echo "=== ★ THE FAULT, BY IDENTITY (host dmesg, watermarked to THIS boot) ==="
grep -E 'Xid' "$D" 2>/dev/null | sed 's/^/      /'
echo "      Xid count = [$(grep -c Xid "$D" 2>/dev/null)]"
# ★★★★★ w292 — THE DESCENT LEVEL, REPORTED EXPLICITLY WHICHEVER WAY IT GOES. It is the axis
# that moved last rung (FAULT_PDE → FAULT_PTE, the first time in this campaign), and a
# report that only prints it when it is favourable is the absent-artefact class.
echo "      ★★ DESCENT LEVEL, every distinct value = [$(grep -oE 'FAULT_(PDE|PTE)[0-9]*' "$D" 2>/dev/null | sort | uniq -c | tr '\n' ' ')]"
echo "         ⊘ empty here means NO FAULT LEVEL WAS PRINTED — UNMEASURED, not 'no fault'"
echo "      ★★ ENGINE / HUBCLIENT / ACCESS = [$(grep -oE 'ENGINE [A-Z0-9_]+|HUBCLIENT_[A-Z0-9_]+|ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | sort | uniq -c | tr '\n' ' ')]"
echo "      ★★ CHANNEL(S) named by the fault = [$(grep -oE 'ch 0x[0-9a-f]+' "$D" 2>/dev/null | sort | uniq -c | tr '\n' ' ')]"
FVA=$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | tail -1 | grep -oE '0x[0-9a-f_]+' | tr -d '_')
echo "      fault VA, normalised = [${FVA:-⊘ NONE — the join below is VACUOUS, not negative}]"

echo ""
echo "=== ★★★★★ THE JOIN — after publication, who owns the faulting VA? ==="
python3 - "$Q" "${FVA:-}" <<'PY'
import re, sys
qlog, fva = sys.argv[1], sys.argv[2]
if not fva:
    print("    ⊘ NO FAULT VA ON THIS BOOT — the join is VACUOUS, not negative."); raise SystemExit
f = int(fva, 16)
txt = open(qlog, errors="replace").read()
def last(tag):
    # ⚠ MULTILINE, or `$` anchors to end-of-FILE and the LAST tag on every line but the
    # final one silently returns an empty picture that reads as "nothing there".
    m = re.findall(re.escape(tag) + r" (.*?)(?= \| [A-Z-]+ |$)", txt, re.MULTILINE)
    return m[-1] if m else ""
print(f"    faulting VA = 0x{f:x}")
for tag in ("GUEST-DESCRIBES", "TABLE-DESCRIBES", "HOST-PUBLISHED"):
    s = last(tag)
    owner = []
    for blk in re.finditer(r"\[proc=(\d+) gpu=(\d+) pdb=(0x[0-9a-f]+)([^\]]*)\]", s):
        for r in re.finditer(r"0x([0-9a-f]+)\+0x([0-9a-f]+)", blk.group(4)):
            st, ln = int(r.group(1), 16), int(r.group(2), 16)
            if st <= f < st + ln:
                owner.append(f"proc={blk.group(1)} pdb={blk.group(3)} run=0x{st:x}+0x{ln:x}")
    trunc = "⚠ SOME ROW WAS CAPPED — an absence here is NOT a measured absence" if "CAPPED" in s else ""
    print(f"    {tag:16s} OWNS-FAULT = {('YES ' + '; '.join(owner)) if owner else 'NO'}  {trunc}")
    if not s:
        print(f"      ⊘ {tag} ABSENT FROM THE LOG — an UNMEASURED row, not a zero.")
PY

echo ""
echo "=== ★★★★★ CUP2_RC — ANCHORED (baseline 1 at w290cup2 @ c2b2a22; 1 again at w290pboth) ==="
# ★★★ THE ANCHOR TRAP HAS FIRED FIVE CONSECUTIVE RUNGS. `^CUP2_RC=` is the primary read; the
# looser form and the fully unanchored one are printed BESIDE it so a reader can see what the
# unanchored read would have said, rather than being told it was avoided.
strict=$(grep -oE '^CUP2_RC=[0-9]+' "$P" 2>/dev/null | tail -1)
loose=$(grep -oE '(^|[^A-Z_])CUP2_RC=[0-9]+' "$P" 2>/dev/null | grep -oE 'CUP2_RC=[0-9]+' | tail -1)
echo "--- ★★★★★ CUP2_RC, ^ANCHORED = [${strict:-⊘ NO LINE BEGINS WITH CUP2_RC= — THE MEASUREMENT DID NOT HAPPEN. ⊘ This is NOT 0}]"
echo "    loose anchor (^|non-word), for contrast: [${loose:-⊘ NONE}]"
echo "    UNANCHORED, for contrast: [$(grep -oh 'CUP2_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
echo "    ⊘ the unanchored read has printed [CUP2_RC=0 CUP2_RC=1] on five consecutive rungs — the 0 is the harness's own echo, not a result"
echo "=== ★ THE LAST PRINT — where it got to if it did not cross ==="
grep -E '^(ok|FAIL|cu[A-Z]|--- cup2|totalMem|bad=|maxerr)' "$P" 2>/dev/null | tail -12 | sed 's/^/      /'

echo ""
echo "=== ★★ HARNESS SELF-CHECK — assert this block's own output exists ==="
echo "    lines written so far = [$(wc -l < "$OUT")]"
echo "    qemu log bytes       = [$(stat -c%s "$Q" 2>/dev/null || echo MISSING)]"
echo "    probe log bytes      = [$(stat -c%s "$P" 2>/dev/null || echo MISSING)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."

finish 0
