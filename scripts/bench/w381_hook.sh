#!/usr/bin/env bash
# ★★★★★ w381 — THE GUEST HALF OF THE DIFFERENTIAL. Run the mapping-plane battery **inside
# the Mode-2 guest**, against our emulated GPU, with no libcuda in the process at all.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/w381_hook.sh scripts/bench/boot_capture.sh <tag>
#   needs: GQ_TIMEOUT >= 600
#          KAYFABE_W381_BIN  — a **statically linked** kayfabe-rm-ladder (musl). Default:
#            $CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
#          KAYFABE_W381_ARGS — default `--w381` (the whole battery on the copy probe).
#
# ## ★★★ WHY THIS EXISTS — the w379 battery could not run in a guest AT ALL
#
# Every w379 rung proved a VA live with `HostRmBackend::submit_release_at`, which emits the
# host-FIFO semaphore run. The Mode-2 CPU copy-engine emulator decodes that as
# `PushMethod::SemRelease` and **deliberately does not act on it** — so in the guest every
# rung reported its own CONTROL as failed and printed `NOTRUN`, and the battery was a
# native-only control rather than an iteration handle. `--w381` swaps the primitive for a
# four-byte `LAUNCH_DMA`, which that path serves end to end.
#
# ## ⊘ THE SCOPE CAVEAT, STATED HERE RATHER THAN DISCOVERED LATER
#
# Inherited verbatim from `r33_hook_ce_client.sh`, because it is just as true of these rungs:
# **the ladder builds its OWN `FERMI_VASPACE_A` inside the guest and probes THAT VAS.** It
# says nothing about the VA the GR engine faults on during `cuCtxCreate` — that channel
# belongs to the guest driver's own client with its own PDB. A probe in the wrong address
# space is this campaign's recorded failure shape.
#
# ⚠ **AND WHICH CE EXECUTOR THE BOOT RAN UNDER DECIDES WHAT THIS MEASURES.** With
# `KAYFABE_CE_EXECUTOR=local` (the default) the shell's CPU copy-engine serves every CE
# doorbell and these rungs measure **the emulator**. With `=host`,
# `forwarding_plane_owns_ce` hands a USER proc's addressable CE channel to the forwarding
# plane instead and they measure **that**. The hook prints the value; a run that does not
# say which one it took cannot be compared to any other run.
#
# ## ⊘ Three traps this hook is written against, all measured in this repo
#
#  - **A statically linked binary, always.** A dynamic binary that fails to load in the guest
#    reports as *"the ladder found no GPU"*, a completely different finding. ASSERTED.
#  - **Zero bytes is not "not yet".** The guest command writes a start marker and an explicit
#    `W381_RC=` terminator, so *"file exists, has no terminator"* is detectable at all. And
#    the `timeout` is INSIDE the guest, so `124` (the work ran out of time) and `143` (the
#    launcher killed it) stay distinguishable.
#  - **Grade on the rungs' own verdict lines, anchored.** `RUNG_<name>=` and
#    `RUNGCTL_<name>=`, nothing else. w377 printed prose while its grader looked for a name
#    nothing emitted, and a 3/3 run graded as UNMEASURED.
set -uo pipefail
SELFDIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SELFDIR/../.." && pwd)
G="$SELFDIR/gssh_nv"
TAG=${1:-w381}
TGT=${CARGO_TARGET_DIR:-$REPO/target}
BIN=${KAYFABE_W381_BIN:-$TGT/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder}
ARGS=${KAYFABE_W381_ARGS:---w381}
TMO=${KAYFABE_W381_TIMEOUT:-420}
OUT=/tmp/w381guest.out

# ⊘ The FIXED denominator, spelled once. A rung deleted in one place goes red here rather
#   than shrinking the total silently.
RUNG_NAMES=(map_propagation alias_two_vas alias_unmap missing_page map_stress rpc_mixed cross_client)

die() { echo "★★★ w381 hook FAILED: $*"; echo "W381_GUEST_OUTCOME=(F) ⊘ UNMEASURED_NO_GUEST — $*"; exit 2; }

echo "=== ★★★★★ w381 — THE MAPPING-PLANE BATTERY, IN THE GUEST  tag=$TAG  $(date -Is) ==="
echo "W381_GUEST_ARGS=$ARGS"
echo "W381_CE_EXECUTOR=[${KAYFABE_CE_EXECUTOR:-⊘unset ⇒ local ⇒ the SHELL CPU copy engine serves every CE doorbell}]"
echo "W381_ISOLATES=[${KAYFABE_ISOLATES:-⊘unset}]  W381_GR_ROUTE=[${KAYFABE_GR_ROUTE:-⊘unset}]"
[ -x "$G" ]   || die "no gssh_nv at $G"
[ -f "$BIN" ] || { echo "W381_GUEST_OUTCOME=(E) ⊘ UNMEASURED_NO_BINARY — no binary at $BIN"; exit 2; }

# ⊘ WHICH COPY RAN. The md5 is what lets the guest arm be joined against the native arm as
#   THE SAME PROGRAM — which is the entire content of the word "differential".
echo "--- the binary under test (⊘ the native arm MUST carry the same md5):"
printf '    %-72s %9s bytes\n' "$BIN" "$(stat -c %s "$BIN")"
echo "    W381_BIN_MD5=$(md5sum < "$BIN" | cut -d' ' -f1)"
case "$(file -b "$BIN")" in
  *static*) echo "    ★ STATIC — it will run in the guest's userland" ;;
  *) die "the binary is NOT statically linked; a load failure would report as 'no GPU'" ;;
esac

echo "=== guest preconditions (⊘ each is a DIFFERENT failure from 'the battery did not work') ==="
$G 'echo "GUEST_UNAME=$(uname -r)"; echo "GUEST_NVIDIA_DEVS=[$(ls /dev/nvidia* 2>/dev/null | tr "\n" " ")]"; echo "GUEST_NVRM_LOADED=$(lsmod | grep -c "^nvidia ")"; echo "GUEST_EUID=$(id -u)"' \
  || die "the guest did not answer ssh"

echo "=== push the binary into the guest ==="
$G "cat > /tmp/kayfabe-rm-ladder && chmod +x /tmp/kayfabe-rm-ladder" < "$BIN" || die "could not push the binary"
$G 'echo "GUEST_MD5=$(md5sum < /tmp/kayfabe-rm-ladder | cut -d" " -f1)"'

echo "=== run it, under its OWN deadline, with a START marker and an RC terminator ==="
$G "echo STARTED \$(date -Is) > /tmp/w381.started; timeout $TMO sudo /tmp/kayfabe-rm-ladder $ARGS > $OUT 2>&1; echo W381_RC=\$? >> $OUT"
echo "--- the battery's own output, verbatim ---"
$G "cat $OUT"
echo "--- end of the battery's output ---"

echo ""
echo "=== ★★★★★ THE GUEST GRADE — anchored on the rungs' own verdict lines ==="
$G "grep -a '^W381_PROBE=' $OUT | head -1" | sed 's/^/    /'
LOCAL=/tmp/w381guest_${TAG}.out
$G "cat $OUT" > "$LOCAL" 2>/dev/null
pass=0; fail=0; notrun=0; seen=0
for r in "${RUNG_NAMES[@]}"; do
  v=$(sed -n "s/^RUNG_${r}=//p" "$LOCAL" 2>/dev/null | tail -1)
  c=$(sed -n "s/^RUNGCTL_${r}=//p" "$LOCAL" 2>/dev/null | tail -1)
  printf '    %-18s RUNG=%-8s CONTROL=%s\n' "$r" "${v:-NONE}" "${c:-NONE}"
  [ -n "$v" ] && seen=$((seen+1))
  case "$v" in PASS) pass=$((pass+1));; FAIL) fail=$((fail+1));; NOTRUN) notrun=$((notrun+1));; esac
done
PROBE=$(sed -n 's/^W381_PROBE=\([^ ]*\).*/\1/p' "$LOCAL" 2>/dev/null | tail -1)
echo "    W381_TABLE_ROW arm=guest probe=${PROBE:-NONE} pass=$pass fail=$fail notrun=$notrun seen=$seen of=${#RUNG_NAMES[@]}"
echo "    W381_GUEST_RC=[$(grep -aoE '^W381_RC=[0-9]+' "$LOCAL" 2>/dev/null | tail -1)]  ⊘ 124 = the work ran out of time; an ABSENT line = no terminator was written"

echo ""
echo "=== ★★★★★ THE GUEST VERDICT — pre-registered, stated once"
if [ "$seen" -eq 0 ]; then
  echo "    W381_GUEST_OUTCOME=(D) ⊘ UNMEASURED — not one RUNG_ line. NOT a failure value."
elif [ "$fail" -gt 0 ]; then
  echo "    W381_GUEST_OUTCOME=(B) $fail rung(s) FAILED with their controls passing — a real red"
  echo "        ⚠ Compare against the NATIVE row before attributing it: a guest-only red cannot"
  echo "        distinguish \"we are broken\" from \"the probe is wrong\"."
elif [ "$notrun" -gt 0 ] && [ "$pass" -eq 0 ]; then
  echo "    W381_GUEST_OUTCOME=(C) ⊘ UNINTERPRETABLE — every selected rung's control failed."
  [ "$PROBE" = "sem-release" ] && echo "        ★ EXPECTED on this probe: the emulator does not act on \`SemRelease\`."
else
  echo "    W381_GUEST_OUTCOME=(A) $pass of ${#RUNG_NAMES[@]} rungs PASS, $notrun not selected, 0 red."
  echo "        ★★★ This is the result the native arm exists to be compared to."
fi

echo ""
echo "=== the guest driver's own word across the run (⊘ the HOST ring buffer does not carry it) ==="
$G 'sudo dmesg 2>/dev/null | grep -iE "nvrm|xid" | tail -15 | sed "s/^/    /"'
echo "=== ★★ HOOK SELF-CHECK — assert this block's own inputs exist ---"
echo "    guest output bytes = [$(wc -c < "$LOCAL" 2>/dev/null)]"
echo "    RUNG_ lines        = [$(grep -ac '^RUNG_' "$LOCAL" 2>/dev/null)]  (MUST be ${#RUNG_NAMES[@]})"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== w381 guest hook done $(date -Is) ==="
