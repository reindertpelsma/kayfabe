#!/usr/bin/env bash
# ★★★★★ w377 — THE LATE-MAP RACE AS A DIFFERENTIAL: bare metal vs the emulated GPU.
#
# OWNER 2026-09-06: *"if your raw clients exercises the path that is what you think missing
# pages, and it fails (intended), you have something to iterate over to get a proper
# architecture, without proprietary libcuda."*
#
# ⇒ THE POINT OF THIS RUNG IS TO GO RED IN THE GUEST. A red we own — 30 lines of raw client,
#   deterministic, no CUDA runtime — is an ITERATION HANDLE. Debugging libcuda is not.
#
# ★★★ AND A SINGLE ARM CANNOT SAY THAT. The native arm is not a nicety: it is what
# distinguishes *"we lose a race the driver wins"* (a defect, and the handle) from
# *"nobody wins this race"* (parity, and the probe needs redesigning). Both arms, one run,
# graded as a pair — a guest-only red would be read as a finding and might be neither.
set -uo pipefail
REPO=${KAYFABE_REPO:-/root/kayfabe}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w297}
export KAYFABE_TAG=${KAYFABE_TAG:-w377race}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/racemap_hook.sh"
export GQ_TIMEOUT=${GQ_TIMEOUT:-1200}
NATIVE_LOG=/workspace/bench/run_${KAYFABE_TAG}_native.log
STAMP=$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo unknown)

# ── ARM 1 — NATIVE. Bare-metal GA106, no QEMU, no guest, no emulated GPU. ───────────────
# ⚠ This tells us what the REAL driver guarantees, which no amount of ogkm source reading
#   settled: the deferred-invalidate path is client-reachable and `NVOS46_FLAGS_DEFER_TLB_
#   INVALIDATION` has no in-tree caller, so only hardware can answer.
echo "=== ★ w377 ARM 1/2 — NATIVE (bare metal, source $STAMP)  $(date -Is) ==="
BIN=""
for c in "$CARGO_TARGET_DIR"/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder \
         "$CARGO_TARGET_DIR"/release/kayfabe-rm-ladder "$REPO"/target/release/kayfabe-rm-ladder; do
  [ -x "$c" ] && BIN="$c" && break
done
if [ -z "$BIN" ]; then
  echo "NATIVE_ARM_A=NONE"; echo "NATIVE_ARM_B=NONE"
  echo "⊘ rmladder not built — the native arm is UNMEASURED, not passing." | tee "$NATIVE_LOG"
else
  sudo dmesg -c >/dev/null 2>&1 || true
  timeout 300 "$BIN" --late-map-race 2>&1 | tee "$NATIVE_LOG"
  echo "--- host dmesg after the native arm ---" | tee -a "$NATIVE_LOG"
  sudo dmesg 2>&1 | grep -iE "xid|nvrm" | tail -10 | tee -a "$NATIVE_LOG"
fi
NA=$(sed -n 's/^RACEMAP_ARM_A=//p' "$NATIVE_LOG" 2>/dev/null | tail -1)
NB=$(sed -n 's/^RACEMAP_ARM_B=//p' "$NATIVE_LOG" 2>/dev/null | tail -1)

# ── ARM 2 — THE EMULATED GPU. Same binary, same rung, inside a Mode-2 guest. ────────────
echo ""
echo "=== ★ w377 ARM 2/2 — MODE-2 GUEST (emulated GPU)  $(date -Is) ==="
"$REPO/scripts/bench/w290p_run.sh" "${W298_ARM:-drain}"
BRC=$?
OUT=/workspace/${KAYFABE_TAG}.log
# ⊘ THE HOOK'S OUTPUT IS NOT IN $OUT — boot_capture.sh routes POST_CAPTURE_HOOK stdout to
#   run_<tag>_probe.log and leaves only "hook finished: rc=0" behind. Grepping $OUT reads as
#   "the hook produced no result", which is exactly how w376's first run was misgraded.
PROBE=/workspace/bench/run_${KAYFABE_TAG}_probe.log
GA=$(sed -n 's/^GUEST_ARM_A=//p' "$PROBE" 2>/dev/null | tail -1)
GB=$(sed -n 's/^GUEST_ARM_B=//p' "$PROBE" 2>/dev/null | tail -1)

echo ""
echo "================================================================================"
echo "=== ★★★★★ W377 GRADING — THE 2×2, source $STAMP  inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
grep -aE "RACEMAP_|GUEST_ARM_|ARM [AB]|Xid|FAULT_PDE" "$PROBE" 2>/dev/null | tail -30
echo ""
printf '    %-22s A=%-8s B=%-8s\n' "NATIVE (bare metal)" "${NA:-NONE}" "${NB:-NONE}"
printf '    %-22s A=%-8s B=%-8s\n' "GUEST  (emulated)"   "${GA:-NONE}" "${GB:-NONE}"
echo ""
echo "=== ★★★★★ THE DIFFERENTIAL VERDICT — pre-registered, stated once"
if [ "${NA:-}" != PASS ]; then
  echo "    (0) ⊘ NOTHING IS INTERPRETABLE — the NATIVE positive control did not pass."
  echo "        The probe is broken on hardware that has no emulation in it at all."
elif [ "${NB:-}" = NOTRUN ] || [ "${GB:-}" = NOTRUN ]; then
  echo "    (C) ?? RACE NOT RUN on at least one arm — the acquire never blocked."
  echo "        Fix the PROBE. Neither colour may be reported."
elif [ "${NB:-}" != PASS ]; then
  echo "    (P) ⊘ PARITY, NOT A DEFECT — bare metal ALSO fails arm B (native B=${NB:-NONE})."
  echo "        The driver does not guarantee a late mapping reaches a running channel, so"
  echo "        our red is the same red. ⇒ Redesign the probe; do NOT 'fix' this."
elif [ "${GA:-}" != PASS ]; then
  echo "    (D) ⊘ GUEST ARM B UNINTERPRETABLE — guest positive control A=${GA:-NONE} failed."
  echo "        Our submit path is broken before the race is reached. Fix that first."
elif [ "${GB:-}" = PASS ]; then
  echo "    (A) BOTH GREEN — the late mapping reaches a running channel on the emulated GPU."
  echo "        ⊘ This class is covered by THIS probe only. Escalate: wide allocation,"
  echo "        two concurrent clients, an executing CE copy with work appended mid-flight."
else
  echo "    (B) ★★★★★ THE REPRO — native B=PASS, guest B=${GB}."
  echo "        Hardware wins this race and we lose it. A deterministic red we own, with no"
  echo "        proprietary runtime in it. ⇒ THIS IS THE ITERATION HANDLE the architecture"
  echo "        gets built against: publish so that arm B goes green, then re-run BOTH arms."
fi
echo "--- ⊘ EVERY RELAXATION THAT WAS ON — a relaxed green is a MAP, not the milestone ---"
for v in KAYFABE_PT_SWEEP KAYFABE_OPERAND_JOIN KAYFABE_FB_JOIN KAYFABE_VAS_PUBLISH \
         KAYFABE_GR_ROUTE KAYFABE_GUEST_RING KAYFABE_ISOLATES KAYFABE_CE_EXECUTOR; do
  echo "    $v = [$(grep -aoE "$v=[a-z]+" "$OUT" 2>/dev/null | tail -1)]"
done
echo "--- ★★ HARNESS SELF-CHECK — assert THIS block's own inputs exist ---"
echo "    native verdict lines = [$(grep -ac 'RACEMAP_ARM_' "$NATIVE_LOG" 2>/dev/null)]  (MUST be >= 2)"
echo "    guest  verdict lines = [$(grep -ac 'GUEST_ARM_'   "$PROBE"      2>/dev/null)]  (MUST be >= 2)"
echo "    native log bytes     = [$(wc -c < "$NATIVE_LOG" 2>/dev/null)]"
echo "    probe log bytes      = [$(wc -c < "$PROBE" 2>/dev/null)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W377 EXIT rc=$BRC at $(date -Is) ==="
