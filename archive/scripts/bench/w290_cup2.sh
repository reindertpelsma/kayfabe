#!/usr/bin/env bash
# ★★★★★ w290 — THE PARK, BY IDENTITY, JOINED AGAINST THE FAULT, ON ONE BOOT.
#
# THE QUESTION (brief step 2): does any parked promote half cover the GR fault's VA?
#   YES ⇒ root cause found.  NO ⇒ say so plainly and report what DOES own that VA.
#
# ⊘ THE ARM IS RELAXED AND IS LABELLED AS SUCH: KAYFABE_PT_SWEEP=on +
#   KAYFABE_OPERAND_JOIN=join, byte for byte the w289cup2 arming, so the fault reproduces
#   by identity. A relaxed green is never the milestone.
#
# ★★★ HARNESS SELF-CHECK, because knowing a trap by name does not prevent committing it:
#   the predecessor put its grading block AFTER the inherited `exit` line and printed
#   NOTHING. Here every graded line is before `finish 0`, `finish` is the LAST statement in
#   the file, and the block below ASSERTS ITS OWN OUTPUT EXISTS before any zero is trusted.
set -uo pipefail
OUT=/workspace/w290_cup2.log
exec >"$OUT" 2>&1
finish() { echo "=== W290 CUP2 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W290 CUP2 START $(date -Is) pid=$$ ==="
export PATH=/root/.cargo/bin:$PATH
REPO=/workspace/kayfabe_w290
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD); echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w290
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
# the carried arming — w289cup2, byte for byte
export KAYFABE_FB_JOIN=shared
export KAYFABE_GUEST_RING=ring
export KAYFABE_GUEST_PUSHBUF=pin
export KAYFABE_PT_WITNESS_EXEC=on
export KAYFABE_GUEST_SEMA=pin
export KAYFABE_GR_ROUTE=passthrough
export KAYFABE_GUEST_OPERAND=pin
export KAYFABE_PT_SWEEP=on          # ⊘ RELAXATION 1
export KAYFABE_OPERAND_JOIN=join    # ⊘ RELAXATION 2
unset KAYFABE_RING_VIDMEM

export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_gdbspin.sh
TAG=w290cup2
echo "=== BOOT $TAG START $(date -Is)  HOOK=[cup2_hook_gdbspin] ==="
timeout 1500 "$REPO/scripts/bench/boot_capture.sh" "$TAG"
echo "=== BOOT $TAG RC=$? $(date -Is) ==="
Q=/workspace/bench/run_${TAG}_qemu.log
P=/workspace/bench/run_${TAG}_probe.log
D=/workspace/bench/run_${TAG}_hostdmesg.log

echo "=== ★ THE ARMING ACTUALLY IN FORCE (a boot happening is not an arm running) ==="
grep -oE 'OPERAND-JOIN arm=[a-z]+' "$Q" 2>/dev/null | head -1 | sed 's/^/      /'
grep -oE 'PT-SWEEP tasks=[0-9]+ skipped=[0-9]+ ran=[0-9]+' "$Q" 2>/dev/null | tail -1 | sed 's/^/      /'
for pair in "FB-JOIN arm=shared" "GUEST-RING arm=ring" "GUEST-PUSHBUF arm=pin" \
            "GUEST-SEMA arm=pin" "GR-ROUTE arm=passthrough" "GUEST-OPERAND arm=pin"; do
  grep -q "kayfabe: $pair" "$Q" 2>/dev/null && echo "    ★ CARRIED-ARM: PASS ($pair)" \
    || echo "    ★★★ CARRIED-ARM: FAIL — wanted [$pair]. ⊘ VOID for comparison."
done

# =========================================================================================
# ★★★★★ THE GOAL METRIC — ANCHORED, WITH THE UNANCHORED CONTRAST PRINTED BESIDE IT.
# ⊘ Unanchored has printed `[CUP2_RC=0 CUP2_RC=1]` on THREE consecutive rungs: it would
#   report the campaign's headline success value on a FAILING arm. Both are printed, always.
# =========================================================================================
echo ""
echo "=== ★★★★★ CUP2_RC — ANCHORED (baseline 1 at w289cup2 @ 3929ec3) ==="
rc=$(grep -oE '(^|[^A-Z_])CUP2_RC=[0-9]+' "$P" 2>/dev/null | grep -oE 'CUP2_RC=[0-9]+' | tail -1)
echo "--- ★★★★★ CUP2_RC = [${rc:-⊘ NO cup2 EXIT LINE — THE MEASUREMENT DID NOT HAPPEN. ⊘ This is NOT 0}]"
echo "    unanchored, for contrast: [$(grep -oh 'CUP2_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
echo "=== ★ THE LAST PRINT — where it got to if it did not cross ==="
grep -E '^(ok|FAIL|cu[A-Z]|--- cup2|totalMem|bad=|maxerr)' "$P" 2>/dev/null | tail -12 | sed 's/^/      /'

echo ""
echo "=== ★ THE FAULT, BY IDENTITY (host dmesg, watermarked to THIS boot) ==="
grep -E 'Xid' "$D" 2>/dev/null | sed 's/^/      /'
echo "      Xid count = [$(grep -c Xid "$D" 2>/dev/null)]"
FVA=$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | tail -1 | grep -oE '0x[0-9a-f_]+' | tr -d '_')
echo "      fault VA, normalised = [${FVA:-⊘ NONE — every join below is VACUOUS}]"

# =========================================================================================
# ★★★★★ THE RUNG. Four pictures of one address space, printed in full, then joined against
# the fault VA offline. ⊘ A count cannot see a substitution, so nothing here is a count.
# =========================================================================================
echo ""
echo "=== ★★★★★ THE FOUR PICTURES — last emission of each, VERBATIM ==="
LASTLINE=$(grep -n 'PROMOTE-PARKED' "$Q" 2>/dev/null | tail -1 | cut -d: -f1)
echo "    (from qemu log line ${LASTLINE:-NONE})"
for tag in GUEST-DESCRIBES TABLE-DESCRIBES HOST-PUBLISHED PROMOTE-PARKED; do
  echo "--- $tag:"
  grep -o "$tag .*" "$Q" 2>/dev/null | tail -1 | sed "s/ | /\n        | /g" | head -1 \
    | fold -w 200 | sed 's/^/        /'
done

echo ""
echo "=== ★★★ INSTRUMENT KNOWN-POSITIVE, ON THIS BOOT ==="
echo "⊘ If EVERY VAS reports bound=0=[] while parked>0, the enumerator may only know how to"
echo "  print parks. w289cup2 reached CUMULATIVE joined=4, so a non-empty bound=[…] is a"
echo "  reading THIS boot can produce. Offline control: tests/tests/promote_ctx.rs::"
echo "  the_parked_half_census_names_the_id_the_half_and_the_address."
echo "    PROMOTE-PARKED emissions      = [$(grep -c 'PROMOTE-PARKED' "$Q" 2>/dev/null)]"
echo "    HOST-PUBLISHED emissions      = [$(grep -c 'HOST-PUBLISHED' "$Q" 2>/dev/null)]"
echo "    rows with bound=0=[]          = [$(grep -o 'bound=0=\[\]' "$Q" 2>/dev/null | wc -l)]"
echo "    rows with a NON-EMPTY bound=  = [$(grep -oE 'bound=[1-9][0-9]*=\[[^]]*\]' "$Q" 2>/dev/null | wc -l)]"
echo "    distinct non-empty bound rows:"
grep -oE 'bound=[1-9][0-9]*=\[[^]]*\]' "$Q" 2>/dev/null | sort -u | head -10 | sed 's/^/      /'
echo "    the cumulative promote tally (the other projection of the same fact):"
grep -o 'CUMULATIVE bound=.*' "$Q" 2>/dev/null | tail -1 | sed 's/^/      /'
grep -o 'promote-ctx TALLY.*' "$Q" 2>/dev/null | tail -1 | cut -c1-400 | sed 's/^/      /'
grep -oE 'parked=[0-9]+ half_already=[0-9]+ half_unusable=[0-9]+ orphans\([^)]*\)' "$Q" 2>/dev/null | tail -1 | sed 's/^/      /'

echo ""
echo "=== ★★★★★ THE JOIN — does ANY of the four pictures own the faulting VA? ==="
python3 - "$Q" "${FVA:-}" <<'PY'
import re, sys
qlog, fva = sys.argv[1], sys.argv[2]
if not fva:
    print("    ⊘ NO FAULT VA ON THIS BOOT — the join is VACUOUS, not negative."); raise SystemExit
f = int(fva, 16)
txt = open(qlog, errors="replace").read()
def last(tag):
    # ⚠ MULTILINE, or `$` anchors to end-of-FILE and the LAST tag on every line but the
    # final one silently fails to match — an empty picture that reads as "nothing there".
    m = re.findall(re.escape(tag) + r" (.*?)(?= \| [A-Z-]+ |$)", txt, re.MULTILINE)
    return m[-1] if m else ""
print(f"    faulting VA = 0x{f:x}")
# 1) the parked halves, by identity
parked = last("PROMOTE-PARKED")
hits = []
for m in re.finditer(r"\{bid=(0x[0-9a-f]+) AwaitingPhysical va=0x([0-9a-f]+)\}", parked):
    hits.append((m.group(1), int(m.group(2), 16)))
print(f"    PROMOTE-PARKED AwaitingPhysical halves = {len(hits)}")
for bid, va in hits:
    mark = "  ★★★★★ ← THE FAULTING VA" if va == f else ""
    print(f"        bid={bid} va=0x{va:x}{mark}")
for m in re.finditer(r"\{bid=(0x[0-9a-f]+) AwaitingVa phys=0x([0-9a-f]+) len=0x([0-9a-f]+) ap=(\w+)\}", parked):
    print(f"        bid={m.group(1)} AwaitingVa phys=0x{m.group(2)} len=0x{m.group(3)} ap={m.group(4)}  (no VA — cannot cover any VA)")
exact = [b for b, v in hits if v == f]
print(f"    ⇒ PARKED-HALF-COVERS-FAULT = {'YES bid(s)=' + ','.join(exact) if exact else 'NO'}")
print("      ⊘ 'covers' here is EXACT START only — a park carries a VA and no length, so a")
print("        containment test would need an extent its producer never wrote.")
# 2) which run in each picture contains it
for tag in ("GUEST-DESCRIBES", "TABLE-DESCRIBES", "HOST-PUBLISHED"):
    s = last(tag)
    owner = []
    for blk in re.finditer(r"\[proc=(\d+) gpu=(\d+) pdb=(0x[0-9a-f]+)([^\]]*)\]", s):
        body = blk.group(4)
        for r in re.finditer(r"0x([0-9a-f]+)\+0x([0-9a-f]+)", body):
            st, ln = int(r.group(1), 16), int(r.group(2), 16)
            if st <= f < st + ln:
                owner.append(f"proc={blk.group(1)} pdb={blk.group(3)} run=0x{st:x}+0x{ln:x}")
    trunc = "⚠ SOME ROW WAS CAPPED — an absence here is NOT a measured absence" if "CAPPED" in s else ""
    print(f"    {tag:16s} OWNS-FAULT = {('YES ' + '; '.join(owner)) if owner else 'NO'}  {trunc}")
    if not s:
        print(f"      ⊘ {tag} ABSENT FROM THE LOG — this is an UNMEASURED row, not a zero.")
PY

echo ""
echo "=== ★★ HARNESS SELF-CHECK — assert this block's own output exists ==="
echo "    (the predecessor's grading block was dead code after an exit line and printed nothing)"
echo "    lines written so far = [$(wc -l < "$OUT")]"
echo "    qemu log bytes       = [$(stat -c%s "$Q" 2>/dev/null || echo MISSING)]"
echo "    probe log bytes      = [$(stat -c%s "$P" 2>/dev/null || echo MISSING)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."

finish 0
