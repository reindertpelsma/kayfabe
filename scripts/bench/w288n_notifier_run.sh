#!/usr/bin/env bash
# ★★★★★ w288n — THE ERROR NOTIFIER OVER THE GUEST'S OWN PAGES. Three pre-registered criteria.
#
# Arming is w287's, carried byte for byte, so this is comparable to the BASELINE boot
# (`rev 54af1d7d`, CUP2_RC=124, 6/6 arms). The ONLY thing that differs is the code under test.
#
# ★★ A CLEAN REFUTATION IS A FULL RESULT. If the host RM writes the guest's page and cup2 still
#    returns 124, that RETIRES the UVM-notifier hypothesis and is worth as much as a pass.
#    Nothing below is tuned to chase the number.
set -uo pipefail
PFX=${W288N_TAG_PREFIX:-w288nn}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W288NN EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W288NN START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w288n}
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w288n
export KAYFABE_SHIM_FEATURES=host-isolates
CLIENT=$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder

# ★★★ DELETE BOTH BINARIES FIRST: no build ⇒ no file ⇒ no run. The rm-ladder carries no
#     revision stamp, so for IT the reachable guarantee is EXISTENCE, not identity.
rm -f /workspace/bench/qemu-build/qemu-system-x86_64 "$CLIENT"

echo "=== BUILD CLIENT (static musl) $(date -Is) ==="
cargo build --release --target x86_64-unknown-linux-musl -p kayfabe-isolate-host --bin kayfabe-rm-ladder
CRC=$?
echo "=== CLIENT BUILD RC=$CRC ==="
[ -x "$CLIENT" ] || { echo "=== ★★★ NO CLIENT BINARY — a missing binary must be FATAL, not a 127 ==="; finish 95; }

echo "=== BUILD SHIM $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?
echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92
echo "=== BUILD ENOSPC/LLVM = $(grep -c 'No space left on device\|LLVM ERROR' "$OUT" 2>/dev/null) | df: $(df -h /workspace | tail -1) ==="

# ★★★ STAMP GATE, anchored to exactly 40 hex.
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 2>/dev/null \
        | grep -oE 'kayfabe-rev:[0-9a-f]{40}' | head -1 | cut -d: -f2)
echo "=== STAMP=$STAMP HEAD=$HEAD ==="
[ "$STAMP" = "$HEAD" ] || { echo "=== ★★★ STAMP GATE FAIL ==="; finish 93; }

# =========================================================================================
# ★★ CRITERION 2's NATIVE HALF — the KNOWN-POSITIVE and the NEGATIVE CONTROL, on bare metal,
#    from the SAME binary, minutes before the boot. Without the negative control a delivery
#    firing on any GPU event reads as a pass.
# =========================================================================================
echo "=== ★ NATIVE ARM A — NO deliberate fault. EXPECT: notifier QUIET. ==="
XID0=$(dmesg 2>/dev/null | grep -c Xid)
timeout 300 "$CLIENT" --ce-client --notifier-vidmem 2>&1 | tail -30
echo "NATIVE_NOFAULT_RC=$?"
XID1=$(dmesg 2>/dev/null | grep -c Xid)
echo "NATIVE_NOFAULT_XID_DELTA=$XID0->$XID1  (⊘ MUST be unchanged)"

echo "=== ★ NATIVE ARM B — WITH the deliberate fault. EXPECT: notifier status 0xffff. ==="
timeout 300 "$CLIENT" --ce-client-fault --notifier-vidmem 2>&1 | tail -40
echo "NATIVE_FAULT_RC=$?"
XID2=$(dmesg 2>/dev/null | grep -c Xid)
echo "NATIVE_FAULT_XID_DELTA=$XID1->$XID2  (expect +1 — the control FIRING, not a bug)"
dmesg 2>/dev/null | grep Xid | tail -1

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export GQ_TIMEOUT=420
export BOOT_TIMEOUT=180

# The carried arming — w287's, unchanged.
export KAYFABE_FB_JOIN=shared
export KAYFABE_GUEST_RING=ring
export KAYFABE_GUEST_PUSHBUF=pin
export KAYFABE_PT_WITNESS_EXEC=on
export KAYFABE_GUEST_SEMA=pin
export KAYFABE_GR_ROUTE=passthrough
export KAYFABE_GUEST_OPERAND=pin
unset KAYFABE_PT_SWEEP KAYFABE_RING_VIDMEM

# =========================================================================================
# BOOT 1 — cup2. THE GOAL METRIC.
# =========================================================================================
export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_gdbspin.sh
TAG=${PFX}_cup2
echo "=== BOOT $TAG (cup2) START $(date -Is) ==="
timeout 1500 "$REPO/scripts/bench/boot_capture.sh" "$TAG"
echo "=== BOOT $TAG RC=$? $(date -Is) ==="
Q=/workspace/bench/run_${TAG}_qemu.log
P=/workspace/bench/run_${TAG}_probe.log
D=/workspace/bench/run_${TAG}_hostdmesg.log

for pair in "FB-JOIN arm=shared" "GUEST-RING arm=ring" "GUEST-PUSHBUF arm=pin" \
            "GUEST-SEMA arm=pin" "GR-ROUTE arm=passthrough" "GUEST-OPERAND arm=pin"; do
  grep -q "kayfabe: $pair" "$Q" 2>/dev/null \
    && echo "    ★ CARRIED-ARM: PASS ($pair)" \
    || echo "    ★★★ CARRIED-ARM: FAIL — wanted '$pair'. ⊘ VOID for comparison."
done

echo "=== ★★★★★ DID OUR OWN PATH EVEN FIRE? (the known-negative was 0 at rev 54af1d7d) ==="
echo "    ERROR-NOTIFIER built   = [$(grep -c 'ERROR-NOTIFIER .*memory=' "$Q" 2>/dev/null)]"
echo "    ERROR-NOTIFIER REFUSED = [$(grep -c 'ERROR-NOTIFIER REFUSED' "$Q" 2>/dev/null)]"
echo "    --- every line, verbatim (identities, not tallies):"
grep -E 'ERROR-NOTIFIER' "$Q" 2>/dev/null | head -40 | sed 's/^/      /'
echo "    ⊘ ZERO BUILT means the path never ran ⇒ cup2's number says NOTHING about the hypothesis."

echo "=== ★★★★★ CUP2_RC — ANCHORED (baseline: 124 at rev 54af1d7d, 6/6 arms) ==="
rc=$(grep -oE '(^|[^A-Z_])CUP2_RC=[0-9]+' "$P" 2>/dev/null | grep -oE 'CUP2_RC=[0-9]+' | tail -1)
echo "--- ★★★★★ CUP2_RC = [${rc:-⊘ NO cup2 EXIT LINE — THE MEASUREMENT DID NOT HAPPEN. ⊘ This is NOT 0}]"
echo "    (⊘ disambiguated from GCC_CUP2_RC: $(grep -c 'GCC_CUP2_RC' "$P" 2>/dev/null) such line(s))"
echo "    unanchored, for contrast: [$(grep -oh 'CUP2_RC=[0-9]*' "$P" 2>/dev/null | tr '\n' ' ')]"
grep -E '^ok |^CUP2_RC|totalMem' "$P" 2>/dev/null | tail -8 | sed 's/^/      /'
echo "=== THE FAULT, BY IDENTITY ==="
grep -E 'Xid' "$D" 2>/dev/null | tail -3 | sed 's/^/      /'

# =========================================================================================
# BOOT 2 — the raw client IN THE GUEST, both polarities. CRITERIA 1 and 2.
# ⚠ Criterion 1 only exercises this rung if the client's notifier decodes as Sysmem.
#   A vidmem notifier decodes `Unreachable` and attaches NOTHING — see CRITERION1 note.
#   The ERROR-NOTIFIER census above is what distinguishes "tested" from "silently skipped".
# =========================================================================================
export POST_CAPTURE_HOOK=$REPO/scripts/bench/r33_hook_ce_client.sh
export KAYFABE_R33_BIN=$CLIENT
TAG2=${PFX}_client
echo "=== BOOT $TAG2 (raw client) START $(date -Is) ==="
timeout 1500 "$REPO/scripts/bench/boot_capture.sh" "$TAG2"
echo "=== BOOT $TAG2 RC=$? $(date -Is) ==="
Q2=/workspace/bench/run_${TAG2}_qemu.log
P2=/workspace/bench/run_${TAG2}_probe.log
echo "    ERROR-NOTIFIER built   = [$(grep -c 'ERROR-NOTIFIER .*memory=' "$Q2" 2>/dev/null)]"
grep -E 'ERROR-NOTIFIER' "$Q2" 2>/dev/null | head -20 | sed 's/^/      /'
echo "--- ★ THE GUEST-SIDE READ (criterion 1 is IN THE GUEST, not in a host log):"
grep -E '^(★|FAIL|\?\?|ok|info|⊘) +R33 ' "$P2" 2>/dev/null | sed 's/^/      /'
echo "      R33_RC = [$(grep -oE '^R33_RC=[0-9]+' "$P2" 2>/dev/null | tail -1)]"
echo "--- the ioctl census (⚠ failed=0 is NOT 'nothing refused' — RM puts status IN the struct):"
grep -E '^  total=[0-9]+ failed=' "$P2" 2>/dev/null | sed 's/^/      /'

# ⊘⊘⊘ **`finish 0` STOOD HERE, AND EVERYTHING BELOW IT HAD NEVER EXECUTED.**
#
# `[found by reading, 2026-08-13, w289]` The "SHARPENED BAR" section below — the fault-identity
# join that is this runner's entire headline — sat **after an unconditional exit**. It has never
# produced a line, on any run, and nothing said so: the runner exited 0, the log ended tidily,
# and the join's absence read exactly like a join that found nothing.
#
# ⚠ **This is the campaign's `A FEATURE GATE WITH A SILENT NO-OP SIBLING` class, in a harness:**
# *"never ran"* and *"ran and printed nothing"* are indistinguishable by default. It is also why
# `w288nc1` needed an ad-hoc `crit1` script — the join it wanted was right here, unreachable.
#
# ⇒ The exit is moved BELOW the section it was hiding. What the section produces when it runs is
# recorded in `traces/boots/w289/RESULT.md` §3: on `w289g` it yields the host's five `Xid` fields
# against the guest's `info32`/`info16`/`status`, and — crucially — `PLANE D UNMEASURED = 2`,
# which is the fact that had been invisible.

# =========================================================================================
# ★★★★★ THE SHARPENED BAR — FAULT IDENTITY, HOST vs GUEST, IN THE SAME RUN.
# ⊘ "An error arrived in the guest" is NOT the bar. Six fields, printed side by side.
# =========================================================================================
echo "=== ★★★★★ FAULT IDENTITY — HOST SIDE (the reference), from the host's own dmesg ==="
HOSTXID=$(dmesg 2>/dev/null | grep 'Xid' | tail -1)
echo "      HOST: $HOSTXID"
echo "      host Xid code   = [$(echo "$HOSTXID" | grep -oE '\): [0-9]+,' | grep -oE '[0-9]+')]"
echo "      host engine     = [$(echo "$HOSTXID" | grep -oE 'ENGINE [A-Z0-9_]+')]"
echo "      host address    = [$(echo "$HOSTXID" | grep -oE 'faulted @ 0x[0-9a-f_]+')]"
echo "      host fault type = [$(echo "$HOSTXID" | grep -oE 'type FAULT_[A-Z_]+')]"
echo "      host access     = [$(echo "$HOSTXID" | grep -oE 'ACCESS_TYPE_[A-Z_]+')]"

echo "=== ★★★★★ FAULT IDENTITY — GUEST SIDE (what the guest program itself observed) ==="
echo "    ⊘ These come from the GUEST's own process. A host log line is NOT criterion 1."
grep -E 'R33 arm 5 (CONTROL|NOTIFIER|WHERE|IOCTL)' "$P2" 2>/dev/null | sed 's/^/      /'
echo "    --- the notifier aperture actually used (SYSMEM is the one that exercises this rung):"
grep -E 'NOTIFIER-APERTURE|notifier aperture' "$P2" 2>/dev/null | sed 's/^/      /'

echo "=== ★★★★★ THE JOIN — do the two sides name the SAME fault? ==="
echo "    ⊘ Judge by IDENTITY, field by field. A count cannot see a substitution."
echo "    Xid code : host [$(echo "$HOSTXID" | grep -oE '\): [0-9]+,' | grep -oE '[0-9]+')] vs guest info32 [$(grep -oE 'info32 0x[0-9a-f]+' "$P2" 2>/dev/null | tail -1)]"
echo "    address  : host [$(echo "$HOSTXID" | grep -oE 'faulted @ 0x[0-9a-f_]+')] vs guest reported [$(grep -oE 'reported 0x[0-9a-f]+' "$P2" 2>/dev/null | tail -1)]"
echo "    ★ VA-IDENTITY assertion (guest asked vs reported): [$(grep -c 'VA-IDENTITY BROKEN' "$P2" 2>/dev/null)] BROKEN line(s) — MUST be 0"
echo "    ⊘ PLANE D UNMEASURED lines = [$(grep -c 'PLANE D UNMEASURED' "$P2" 2>/dev/null)] — a relay that never answered is NOT a pass"

finish 0
