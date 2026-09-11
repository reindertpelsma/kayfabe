#!/usr/bin/env bash
# ★★★★★ w401 — DOES THE WRITER-SIDE PUBLICATION TRIGGER BRING `P3 rpc-bind` BACK?
#
# `P3 rpc-bind` went red at commit af41c717, which deleted leg 8 — the doorbell publication
# trigger. Leg 8 re-published every VAS on EVERY doorbell, so it was a POLL, and the rung was
# riding on it rather than on a trigger. Underneath it the only real trigger latched inside the
# `GPU_PROMOTE_CTX` handler, which fires FOUR TIMES in a boot.
#
# The replacement asks `AddressTable::generation` — the writer-side fact — per VAS, so any bind
# by any path arms a publication. This boot asks whether that is enough.
#
# ⚠ ONE ARM, current defaults: no BAR passthrough, sweep ON. `KAYFABE_VAS_PUBLISH` is NOT
#   exported: leg 8 is gone, and the worker sets `VasPublishArm::Publish` on its own jobs. A run
#   that re-exported it would be measuring a knob whose path no longer exists — the exact shape
#   the leg-8 commit message calls out.
#
# GRADE, off the artefacts and never off an exit code:
#   PASS  = `P3 rpc-bind` is not red AND the client's W392D_OUTCOME=(P) AND THREADS 4 of 4
#   The publication census must ALSO show the trigger firing more than 4 times, or a green P3
#   is somebody else's doing and this change is unproven.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
tag=${PREFIX:-w401a}

export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_OPERAND_JOIN=join KAYFABE_PT_SWEEP=on \
       KAYFABE_PT_WITNESS_EXEC=on KAYFABE_CE_EXECUTOR=host \
       NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} BOOT_TIMEOUT=${BOOT_TIMEOUT:-180}
export POST_CAPTURE_HOOK="$SRC_DIR/w392d_mean_hook.sh"

echo "=== w401 TRIGGER BOOT $(date -Is) tag=$tag ==="
# ⚠ VERIFY THE BINARY BY CONTENT. The box's HEAD has lied before; a stamp is a claim and the
# string table is the thing that will actually run.
echo "qemu rev: $(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null | grep -o 'kayfabe-rev:[0-9a-f]*' | sort -u | tr '\n' ' ')"
# ⚠ The CONTENT check, and it must be `vas_changed=` — only the writer-side trigger emits it.
# `RPCBIND-PUBLISH` is in BOTH builds and would pass vacuously against the old latch.
echo "WRITER-TRIGGER in binary: $(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null | grep -c 'vas_changed=') (0 ⇒ this is the OLD promote-only latch — STOP, do not grade)"
# ★★★★★ w406 — the CONTENT check for THIS build: only the refresh-before-complete build emits
# `MMUINVAL-REFRESH` / `CE-LOCAL-REFRESH`. A stamp is a claim; the string table is what runs.
echo "W406-REFRESH in binary: $(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null | grep -c 'MMUINVAL-REFRESH\|CE-LOCAL-REFRESH') (0 ⇒ the refresh arms are NOT in this binary — STOP, do not grade)"

if pgrep -x qemu-system-x86 >/dev/null 2>&1; then echo "⊘ a QEMU is running; refusing"; exit 3; fi
bash "$SRC_DIR/boot_capture.sh" "$tag" > "$BENCH/run_${tag}_driver.log" 2>&1
echo "boot_capture rc=$? (⊘ rc=5 is the evidence-persist check; read the artefacts)"

Q="$BENCH/run_${tag}_qemu.log"; D="$BENCH/run_${tag}_probe.log"
echo "--- LEDGER tag=$tag ---"
echo "[client] $(grep -a 'W392D_OUTCOME=' "$D" | tail -1 | sed 's/^ *//' | cut -c1-90)"
echo "[client] $(grep -a 'THREADS ' "$D" | tail -1 | sed 's/^ *//' | cut -c1-80)"
echo "[client] $(grep -a 'MEAN_FALSIFIER' "$D" | tail -1 | sed 's/^ *//' | cut -c1-80)"
echo "--- THE LADDER (P1..Pn), which is what this change is graded on ---"
grep -aE '^\s+P[0-9]+ ' "$D" | sed 's/^ */[ladder] /' | cut -c1-120
echo "--- ★ THE GUEST-SIDE CLIENT, which is the rung the owner asked for ---"
echo "[guest]  $(grep -a 'W392D_MEAN_CONFIG' "$D" | tail -1 | sed 's/^ *//' | cut -c1-100)"
echo "[guest]  $(grep -a 'W392D_GUEST_OUTCOME' "$D" | tail -1 | sed 's/^ *//' | cut -c1-110)"
echo "[guest]  $(grep -a 'W392D_GUEST_RC' "$D" | tail -1 | sed 's/^ *//' | cut -c1-40)"
grep -aE 'VERIFIED|CONTENT MISMATCH|REFUSED at|THREADS |MEAN_FALSIFIER' "$D" | tail -12 | sed 's/^ */[guest]  /' | cut -c1-118
echo "--- ★★★ THE ORDERING, WHICH IS THE WHOLE GRADE ---"
# The bug was never coverage: the row WAS published, six lines after the host was rung.
# So the grade is a COMPARISON OF LINE NUMBERS, not a count of publications.
pub_ln=$(grep -an "leaf va=0x9140000000" "$Q" | head -1 | cut -d: -f1)
ring_ln=$(grep -an "DOORBELL-XLATE.*guest_token=0x00000009" "$Q" | head -1 | cut -d: -f1)
echo "[order]  publish of 0x9140000000 at line ${pub_ln:-NONE};  ring of token 9 at line ${ring_ln:-NONE}"
if [ -n "$pub_ln" ] && [ -n "$ring_ln" ]; then
  if [ "$pub_ln" -lt "$ring_ln" ]; then
    echo "[order]  ✔ PUBLISHED BEFORE THE RING — the page is backed when the engine may fetch"
  else
    echo "[order]  ⊘ RUNG FIRST — the engine may fetch a page we have not backed. THE BUG STANDS."
  fi
else
  echo "[order]  ⊘ UNMEASURED — one of the two lines is absent; do not grade this boot"
fi
echo "[order]  GR0 fault at 0x91_40000000: $(grep -ac "GR0_PBDMA0.*91_40000000" "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null) (must be 0)"
echo "[order]  CE0 bystander @0xa0_00000000: $(grep -ac "CE0.*a0_00000000" "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null) (present in GREEN runs too)"
echo "--- THE TRIGGER ITSELF ---"
echo "[trig]   RPCMAP-PUBLISH (inline, the fix): $(grep -ac "RPCMAP-PUBLISH" "$Q")"
echo "[trig]   RPCBIND-PUBLISH lines: $(grep -ac 'RPCBIND-PUBLISH' "$Q")"
grep -a 'RPCBIND-PUBLISH' "$Q" | head -8 | sed 's/.*kayfabe: /[trig] /' | cut -c1-150
echo "[trig]   MMUINVAL-PUBLISH lines: $(grep -ac 'MMUINVAL-PUBLISH' "$Q")"
# ★★★★★ w406 — the two synchronization points the owner named that are NOT a doorbell, each
# printing its own firing count. `0` on either is "never ran", which is a different finding
# from "ran and did not help" — three hooks in a row fired zero times before this was printed.
echo "[w406]   MMUINVAL-REFRESH firings: $(grep -ac 'MMUINVAL-REFRESH #' "$Q")  (the TLB-invalidate arm refreshed BEFORE completing)"
echo "[w406]   CE-LOCAL-REFRESH firings: $(grep -ac 'CE-LOCAL-REFRESH #' "$Q")  (the UVM emulated channel refreshed BEFORE its release was written)"
echo "[w406]   MMUINVAL-COMPLETE WITHHELD: $(grep -ac 'MMUINVAL-COMPLETE ⊘ WITHHELD' "$Q")  (a newer trigger arrived mid-refresh; expected 0 — RM serialises)"
echo "[w406]   CE-LOCAL COMPLETION NOT WRITTEN: $(grep -ac 'COMPLETION NOT WRITTEN' "$Q")  (must be 0: a stranded waiter)"
echo "[w406]   worst refresh_ms: $(grep -ao 'refresh_ms=[0-9.]*' "$Q" | cut -d= -f2 | sort -n | tail -1)  (the guest spins on TRIGGER for this long)"
grep -a 'MMUINVAL armed=' "$Q" | tail -1 | grep -ao 'worst_hold_us=[0-9]* over_budget=[0-9]* reentrant=[0-9]*' | sed 's/^/[w406]   invalidate census: /'
echo "[trig]   PROMOTE-BOUND lines:    $(grep -ac 'PROMOTE-BOUND' "$Q")"
echo "[xid]    host Xid lines: $(grep -ac 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null)"
echo "[boot]   arming banners: $(grep -ao 'GUEST-RING arm=[a-z]*\|FB-JOIN arm=[a-z]*\|GR-ROUTE arm=[a-z]*\|PT-SWEEP arm=[a-z]*' "$Q" | sort -u | tr '\n' ' ')"
echo "--- END LEDGER tag=$tag ---"

# ★★★★★ w440 — THE THREE INVARIANTS, IN ONE PLACE, SO ONE BOOT ANSWERS ALL OF THEM.
#
# Three changes landed unmeasured (w431 BAR defaults, w432 GSP deferral, w437 the channel
# invalidate). Each has a counter; none was on the ledger, so reading this boot would have
# meant three greps a reader has to know to run. ⊘ A signal nobody prints is the failure mode
# this whole session has been counting — SEVEN mechanisms built, wired and never reached.
echo "--- ★★★★★ THE THREE INVARIANTS (w431 BARs · w432 vCPU · w437 the barrier) ---"

echo "[bar]    $(grep -ao 'BAR1-PASSTHROUGH[^|]\{0,70\}' "$Q" 2>/dev/null | tail -1)"
echo "[bar]    bar1 touches: $(grep -aoc 'bar1' "$Q" 2>/dev/null) ⊘ w431 flipped BOTH defaults to untrapped; a boot where these are HIGH means the flip did not take"

# ⚠ `gsp_off_vcpu=0` with a guest that made progress means deferral is NOT armed and every RPC
# still ran inside a guest store — the 1.79 s trap. The census says so in its own words.
echo "[vcpu]   $(grep -ao 'LANE-CENSUS[^|]\{0,120\}' "$Q" 2>/dev/null | tail -1)"
echo "[vcpu]   $(grep -ao 'GSP-ASYNC [A-Z ]\{0,40\}' "$Q" 2>/dev/null | tail -1)"
# ⊘⊘⊘ **THIS LINE READ THE TARGET, NOT THE MEASUREMENT.** `TRAPWITNESS` ends with the literal
# text `(target: inline_exceptions=0)`, so `grep -o 'inline_exceptions=[0-9]*' | tail -1`
# returned **the goal string** — a hardcoded `0` — on every boot, whatever the real value.
# `[measured w448]` it printed `inline_exceptions=0` while the same line said
# `TRAPWITNESS … inline_exceptions=24`. I reported that 0 to the owner twice.
# ⚠ A metric whose GOAL is written beside it in the same format is a metric a lazy grep will
# read as already met. Anchor on the whole row.
echo "[vcpu]   $(grep -ao 'TRAPWITNESS[^|]\{0,160\}' "$Q" 2>/dev/null | tail -1 | cut -c1-170)"
echo "[vcpu]   ⊘ w394 measured inline_exceptions=166, worst_trap=1 771 955us — read the row above, not a bare number"


# ⊘ NO SEAM INSTALLED is a different fact from an empty refresh, and the device prints it by
# name. MEMOP-CENSUS seen=0 on a boot whose P2 passes means the invalidates never reached the
# decoder at all — a parsing question, not a consumption one.
echo "[barrier] $(grep -ao 'MEMOP-REFRESH[^|]\{0,110\}' "$Q" 2>/dev/null | tail -1)"
echo "[barrier] refresh passes: $(grep -aoc 'MEMOP-REFRESH proc=' "$Q" 2>/dev/null)   ⊘ NO-SEAM lines: $(grep -aoc 'NO SEAM INSTALLED' "$Q" 2>/dev/null)"
echo "[barrier] $(grep -ao 'MEMOP-CENSUS[^|]\{0,90\}' "$Q" 2>/dev/null | tail -1)"

# ★★★★★ w443 — **THE FOURTH TRANSPORT QUESTION, MADE ANSWERABLE.**
#
# Owner: *"but weren't there also map calls that came from pte updates?"* — yes, and the
# recorded answer is a ZERO that this tree has already flagged as untrustworthy.
# `kayfabe-device/src/mmuinval.rs` §1 says it in its own words:
#
#   `INVALIDATE_TLB` fn=200 = 0 · `MEM_OP` method = 0 · `DMA_FILL_PTE_MEM` = 0
#   "Every one of those numbers is correct. They are also a complete enumeration of the two
#    transports somebody thought to instrument, and RM's actual transport on GA106 is neither."
#
# ⇒ **A zero from an incomplete list reads identical to a zero from a complete one.** So do not
# rest on it: print what this boot actually served, and let the number be attributable.
echo "--- ★ PTE/PDE-CARRYING CONTROLS (is there a FOURTH entry point we never consumed?) ---"
echo "[pte]    DMA/PDE/PTE controls served: $(grep -aoiE 'DMA_(FILL_PTE_MEM|UPDATE_PDE_2)|FILL_PTE|UPDATE_PDE' "$Q" 2>/dev/null | sort | uniq -c | tr '\n' ' ')"
echo "[pte]    UpdateBarPde RPCs: $(grep -aoc 'UPDATE_PDE_BAR\|UpdateBarPde' "$Q" 2>/dev/null) ⊘ that is BAR2's own aperture root, NOT a guest mapping — a different fact"
echo "[pte]    unserviced controls: $(grep -ao 'UNSERVICED[^|]\{0,60\}' "$Q" 2>/dev/null | tail -1)"
echo "[pte]    ⊘ ZERO here is only trustworthy WITH the served-control census beside it: an"
echo "[pte]      absent transport and an uninstrumented one print the same 0."
