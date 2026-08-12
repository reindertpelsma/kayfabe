#!/usr/bin/env bash
# ★★★★★ w282 — LEG 7: THE CE OPERAND LEAVES, PRESENTED TO THE JOIN THAT ALREADY EXISTS.
#
# Pre-registration: `traces/boots/w282/PREREGISTRATION.md` — committed BEFORE this runs.
#
# ⊘⊘ THE RUNG. `w281_client` got a REAL HOST COPY ENGINE to fetch and execute the guest's own
#    methods, and it faulted `Xid 31 CE0 HUBCLIENT_CE1 FAULT_PTE ACCESS_TYPE_VIRT @
#    0x1_20010000` — the destination the guest's own pushbuffer declared. `w281b_clientsweep`
#    armed the whole-VAS sweep, which BOUND both operand VAs (2 MISS → 0 MISS) — to
#    **`Vidmem@0x10000` / `Vidmem@0x20000`**, our EMULATED framebuffer, which
#    `Representability::Fabricated` routes to `CeExecutor::Ours`, which `ce_copy` refuses by
#    name under a standing owner ruling. ⇒ BOTH reachable configurations are walls and both are
#    the SAME missing thing: the operand is not memory a real engine can be pointed at.
#
# ⊘⊘ THE FIX IS A CALLER, NOT A MECHANISM. `join_one_fb_leaf` — the four-step join — has been
#    in the tree since w260 and `Regs::back_census_framebuffer_leaves` already drives it off an
#    OPERAND census. That caller hangs off `declare_gr_completion`, which `SharedDoorbell::ring`
#    calls on the two **GR** dispositions and on **NO CE PATH AT ALL**. Leg 7 presents the CE
#    plane's operand leaves to the same join.
#
# THE ARMS — ONE VARIABLE, and the control is w281b's exact configuration:
#   arm             RING_VIDMEM  PUSHBUF_VIDMEM  PT_SWEEP  OPERAND_JOIN
#   w282_client     on           on              on        join   ← the rung
#   w282_clientoff  on           on              on        (unset) ← == w281b_clientsweep
# ★ The rung's own arm runs FIRST: if the session is cut short, the control is what is
#   missing, and a control is recoverable where the measurement is not.
#
# ⊘ PT_SWEEP is ON on BOTH arms and is NOT this rung's variable: leg 7's candidate selection
#   reads the address table, and without the sweep the operand VAs are `2 MISS` (w281) — so a
#   difference between the arms would be un-attributable between the sweep and the join.
#
# ★ START marker and EXIT line so "file exists but has no terminator" is detectable at all
#   (143 = the JOB was SIGTERMed; 124 = the LAUNCHER's ssh expired while the job ran on).
set -uo pipefail
PFX=${W282_TAG_PREFIX:-w282}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W282 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W282 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w282}
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w268
export KAYFABE_SHIM_FEATURES=host-isolates

# ★★★ THE CLIENT IS BUILT FIRST, and STATIC. ⊘ ASSERT THE ARTEFACT, NEVER THE EXIT STATUS —
#     `[w279, measured]` `cargo build` returned 0 while writing the binary somewhere else, the
#     guest ran NO client, and the boot printed that rung's PREDICTED SUCCESS.
echo "=== BUILD the static client $(date -Is) ==="
cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder
echo "=== CLIENT BUILD RC=$? (⊘ this number is NOT the check; the next one is) ==="
CLIENT=${CARGO_TARGET_DIR:-$REPO/target}/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
if [ ! -s "$CLIENT" ]; then
  echo "=== ★★★ CLIENT MISSING OR EMPTY at $CLIENT ==="
  ls -l "${CARGO_TARGET_DIR:-$REPO/target}/x86_64-unknown-linux-musl/release/" 2>&1 | head -20
  finish 97
fi
file "$CLIENT"
CLIENT_MD5=$(md5sum < "$CLIENT" | cut -d' ' -f1)
echo "=== CLIENT md5=$CLIENT_MD5 ==="
[ -n "$CLIENT_MD5" ] || { echo "=== ★★★ EMPTY md5 — refusing to boot over an unidentified binary ==="; finish 98; }

# ★★★★★ THE NATIVE ARM, FROM THIS EXACT BINARY, HERE, BEFORE THE BOOTS. ⊘ Not carried from a
#     document: a differential whose reference is a number in a document is a differential
#     against a document.
echo "=== ★★★★★ NATIVE ARM (bare metal, same binary) $(date -Is) ==="
timeout 240 ./scripts/bench/host_xid_watch.sh ${PFX}_native -- "$CLIENT" --ce-client
echo "=== NATIVE ARM RC=$? ==="

echo "=== BUILD the QOM shim $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?
echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92
echo "=== BUILD ENOSPC/LLVM = $(grep -c 'No space left on device\|LLVM ERROR' "$OUT" 2>/dev/null) | df: $(df -h /workspace | tail -1) ==="

# ★★★ THE STAMP GATE, anchored to exactly 40 hex.
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 2>/dev/null \
        | grep -oE 'kayfabe-rev:[0-9a-f]{40}' | head -1 | cut -d: -f2)
echo "=== STAMP=$STAMP HEAD=$HEAD ==="
[ "$STAMP" = "$HEAD" ] || { echo "=== ★★★ STAMP GATE FAIL: the binary is not this HEAD ==="; finish 93; }

# ⊘⊘ THE cap2b GUARD, carried BYTE FOR BYTE from w277 — no address this rung reasons about may
#    be a literal in the binary: a pass that RECALLS an address instead of DECODING one makes
#    every row below read as evidence for a mechanism that is a constant.
NCAP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 \
       | grep -c '0x2_04420000\|0x204420000\|0x2_04428000\|0x204428000\|0x2_0440fff0\|0x20440fff0')
echo "  ★★★ cap2b GUARD: fault/semaphore address literals in the binary = $NCAP (MUST be 0)"
[ "$NCAP" -eq 0 ] || { echo "  ★★★ AN ADDRESS IS A LITERAL — the pass recalls, it does not decode"; finish 94; }
# ⊘ WIDENED, and REPORTED not ENFORCED: THIS rung's own addresses — the client's pushbuffer
#   VA, its fb offset, its ring VA, its semaphore. A hard gate on a NEW pattern can only kill a
#   run on its first use, and an untested gate that aborts the boot is worse than a number
#   nobody reads. If this is non-zero, every H1/H4 row below is suspect and must be re-read
#   before it is believed: a pass that RECALLS an address is not a pass that DECODES one.
NCAP2=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 \
        | grep -c '0x120020000\|0x120021000\|0x120022000\|0x40000000000')
echo "  ★★ w282 WIDENING (reported, not enforced): client pushbuffer/ring/sema literals = $NCAP2 (want 0)"

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export BOOT_TIMEOUT=180

# The carried arming — `w271_pin`'s, byte for byte, identical on both arms.
export KAYFABE_FB_JOIN=shared
export KAYFABE_GUEST_RING=ring
export KAYFABE_GUEST_PUSHBUF=pin
export KAYFABE_PT_WITNESS_EXEC=on
export KAYFABE_GUEST_SEMA=pin
export KAYFABE_GR_ROUTE=passthrough
export KAYFABE_GUEST_OPERAND=pin
# ⊘ PT_SWEEP is set PER ARM below (the `clientsweep` arm), never globally.
unset KAYFABE_PT_SWEEP

# ★★★★★ ROUTE B — ON for BOTH arms. It is the SUPPLY (an `FbSource` registration), not this
# rung's variable, and with it OFF there is nothing for the pushbuffer route to read, so an
# unchanged refusal would be un-attributable.
# ⊘ It is UNREACHABLE with the witness disarmed (`plan_gpfifo_ring` returns `RingVaUnbound`
#   BEFORE `VidmemRoute` is computed), and the witness is `on` above.
#
# ★★★★★ THE RUNG'S VARIABLE IS `KAYFABE_PUSHBUF_VIDMEM`, set per arm in the loop below.
#     ⊘ It is the ROUTE, deliberately separate from route B's SUPPLY, per w279's ruling: one
#     flag would make a boot unable to say which of the two reads produced a byte.
#     ⚠ NECESSARY-NOT-SUFFICIENT alone — both must be on, and both are asserted per arm.

[ -f scripts/bench/guest_spinprobe.c ] || { echo "=== ★★★ MISSING the spin probe source ==="; finish 96; }
for f in scripts/bench/r33_hook_ce_client.sh scripts/bench/cup2_hook_deadline.sh; do
  [ -x "$f" ] || { echo "=== ★★★ $f IS MISSING OR NOT EXECUTABLE — the hook cannot run ==="; finish 96; }
done
# ★★★ THE cup2 ARM'S OWN INPUT, asserted HERE and not discovered at boot 3. `[measured, w226]`
#     a hook that cannot find its source exits 127, the guest runs NOTHING, and every other
#     signal says the boot happened. ⊘ Assert the ARTEFACT, never the exit status.
CUP2_SRC=${CUP2_SRC:-/workspace/bench/cup2.c}
if [ ! -s "$CUP2_SRC" ]; then
  echo "=== ★★★ cup2 SOURCE MISSING OR EMPTY at $CUP2_SRC — the cup2 arm would run NOTHING ==="
  ls -l /workspace/bench/cup2.c 2>&1 | head -3
  finish 95
fi
echo "=== cup2 source $CUP2_SRC = $(stat -c %s "$CUP2_SRC") bytes, md5=$(md5sum < "$CUP2_SRC" | cut -d' ' -f1) ==="

# =============================================================================================
# THE GRADER. ⊘ Every zero is gated on a KNOWN-POSITIVE on its own grep, and every arm is read
# by an ADDRESS or a KEY — a count cannot see a substitution.
# =============================================================================================
grade() {
  local tag=$1 kind=$2
  local Q=/workspace/bench/run_${tag}_qemu.log
  local P=/workspace/bench/run_${tag}_probe.log
  local D=/workspace/bench/run_${tag}_hostdmesg.log

  echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
  echo "--- ENOSPC_LLVM=$(grep -c 'No space left on device\|LLVM ERROR' "$Q" 2>/dev/null || echo '?')  df: $(df -h /workspace | tail -1)"
  echo "--- ARTEFACT SIZES: qemu=$(stat -c %s "$Q" 2>/dev/null || echo MISSING) probe=$(stat -c %s "$P" 2>/dev/null || echo MISSING) hostdmesg=$(stat -c %s "$D" 2>/dev/null || echo MISSING)"

  # ---- ARM ASSERTIONS: a mis-armed boot must not be a data point that looks like one --------
  echo "--- ★★★ ARM ASSERTIONS (read out of the DEVICE's own lines):"
  # ⊘ Matched on the DEVICE's verdict word (`route B ON`/`OFF`), not on the env var's spelling:
  #   the flag accepts `1` and `on`, and w246d's log says `=1`. A gate keyed on the spelling
  #   would fail an armed boot. ★ And the CONTROL asserts the OPPOSITE — an unarmed control that
  #   is silently armed is the same defect as an armed run that is silently unarmed.
  # ⊘ Route B (the SUPPLY) is ON on BOTH arms — it is not this rung's variable.
  grep -qE "kayfabe: RING-VIDMEM .*route B ON" "$Q" 2>/dev/null \
    && echo "    ★ ROUTE-B: PASS (route B ON — the supply, ON for both arms)" \
    || { echo "    ★★★ ROUTE-B: FAIL — wanted 'route B ON'. ⊘ THIS BOOT IS VOID: with no"; \
         echo "        FbSource there is nothing for the pushbuffer route to read, so an"; \
         echo "        UNCHANGED refusal here would be un-attributable."; \
         grep -o 'RING-VIDMEM.\{0,90\}' "$Q" 2>/dev/null | head -1 | sed 's/^/       saw: /'; }
  # ⊘ PUSHBUF-VIDMEM is ON on ALL arms this rung — it is the CARRIED configuration, not the
  #   variable. A control that differed here too would confound two flags.
  grep -qE "kayfabe: PUSHBUF-VIDMEM .*pushbuffer route ON" "$Q" 2>/dev/null \
    && echo "    ★ PUSHBUF-VIDMEM: PASS (pushbuffer route ON — carried on every arm)" \
    || { echo "    ★★★ PUSHBUF-VIDMEM: FAIL — wanted 'pushbuffer route ON'. ⊘ VOID for this rung."; \
         grep -o 'PUSHBUF-VIDMEM.\{0,160\}' "$Q" 2>/dev/null | head -1 | sed 's/^/       saw: /'; }
  # ★★★★★ **THE RUNG'S OWN FLAG, AND THE CONTROL ASSERTS THE OPPOSITE** — an unarmed control
  # that is silently armed is the same defect as an armed run that is silently unarmed, and it
  # is the ONLY variable between `client` and `clientoff`.
  local wantoj='join'; [ "$kind" = clientoff ] && wantoj='assert'
  grep -qE "kayfabe: OPERAND-JOIN arm=$wantoj " "$Q" 2>/dev/null \
    && echo "    ★★★★★ OPERAND-JOIN: PASS (arm=$wantoj — this arm wanted $wantoj)" \
    || { echo "    ★★★★★ OPERAND-JOIN: FAIL — wanted 'arm=$wantoj'. ⊘ THIS BOOT IS VOID for"; \
         echo "        this rung: the one variable did not take, so any difference below is"; \
         echo "        un-attributable."; \
         grep -o 'OPERAND-JOIN arm=.\{0,120\}' "$Q" 2>/dev/null | head -1 | sed 's/^/       saw: /'; }
  echo "      the flag's own startup line, verbatim (carries fb_join= AND host_isolates=):"
  grep -o 'kayfabe: OPERAND-JOIN arm=.\{0,240\}' "$Q" 2>/dev/null | head -1 | sed 's/^/        /'
  echo "      ⊘ NO LINE ABOVE ⇒ this binary predates leg 7 — NOT `armed and nothing moved`."
  # ⊘ PT_SWEEP is ON on ALL arms this rung: leg 7's candidate selection reads the address
  #   table, and w281 measured the operand VAs `2 MISS` without it.
  echo "    ★ PT-SWEEP wanted ON (carried on every arm); the device's own line:"
  grep -o 'PT-SWEEP arm=.\{0,60\}' "$Q" 2>/dev/null | head -1 | sed 's/^/        /'
  echo "      the flag's own line, verbatim (carries armed= AND reachable=):"
  grep -o 'PUSHBUF-VIDMEM.\{0,200\}' "$Q" 2>/dev/null | head -1 | sed 's/^/        /'
  for pair in "FB-JOIN arm=shared" "GUEST-RING arm=ring" "GUEST-PUSHBUF arm=pin" \
              "GUEST-SEMA arm=pin" "GR-ROUTE arm=passthrough" "GUEST-OPERAND arm=pin"; do
    grep -q "kayfabe: $pair" "$Q" 2>/dev/null \
      && echo "    ★ CARRIED-ARM: PASS ($pair)" \
      || echo "    ★★★ CARRIED-ARM: FAIL — wanted '$pair'. ⊘ VOID for comparison."
  done
  grep -q 'EXEC-WITNESS ARMED' "$Q" 2>/dev/null \
    && echo "    ★ WITNESS-ARM: PASS (EXEC-WITNESS ARMED — route B is REACHABLE)" \
    || echo "    ★★★ WITNESS-ARM: FAIL — route B is UNREACHABLE without it (w246). ⊘ VOID."

  # ---- THE VOID GUARDS ----------------------------------------------------------------------
  echo "--- ⊘⊘ VOID GUARDS — known-positives on MY OWN greps, before any zero is read:"
  echo "      RING-PROJ lines  = [$(grep -c 'RING-PROJ' "$Q" 2>/dev/null)]   ⊘ 0 ⇒ no doorbell descent happened AT ALL"
  echo "      fbRING rows      = [$(grep -c 'fbRING' "$Q" 2>/dev/null)]   ⊘ 0 ⇒ VOID, not 'no join'"
  echo "      DOORBELL-XLATE   = [$(grep -c 'DOORBELL-XLATE' "$Q" 2>/dev/null)]   (w277: 88)"
  echo "      ★ DID THE CLIENT RUN?  NATIVE md5 = $CLIENT_MD5"
  grep -E 'GUEST_MD5=|GUEST_EXECUTABLE=|GUEST_NVIDIA_DEVS=|GUEST_NVRM_LOADED=' "$P" 2>/dev/null | sed 's/^/        /'
  echo "        ioctl census (native reference total=53 failed=0):"
  grep -E '^  total=[0-9]+ failed=' "$P" 2>/dev/null | sed 's/^/        /'

  # =========================================================================================
  # ★★★★★ H2 — WAS THE NEW ROUTE ACTUALLY TAKEN? ⊘ Armed is not taken.
  # =========================================================================================
  echo "--- ★★★★★ H2: FWD-PUSHBUF — the device's own line, printed ONLY when a planned run"
  echo "    resolves into OUR framebuffer (⊘ an armed-and-never-taken route must not read as taken):"
  echo "      FWD-PUSHBUF lines = $(grep -c 'FWD-PUSHBUF' "$Q" 2>/dev/null)   (clientoff control: expect 0)"
  grep -o 'FWD-PUSHBUF.\{0,240\}' "$Q" 2>/dev/null | sort -u | sed 's/^/      /' | head -8

  # =========================================================================================
  # ★★★★★ H11a — THE RUNG'S OWN QUESTION, GRADED BY ADDRESS
  # =========================================================================================
  echo "--- ★★★★★ H11a: THE STANDING ON EVERY fbRING PAGE, by ADDRESS (⊘ never by count):"
  grep -oE 'fbRING\[p[01]\]@0x[0-9a-f]+=[0-9a-f]+ nz[0-9]+/[0-9]+ [A-Za-z?-]+' "$Q" 2>/dev/null \
    | sort -u | sed 's/^/      /'
  echo "    --- ★ the CLIENT's RING page @0x41000 (w279/w280: nz4/4096 JOINED-one-memory):"
  grep -oE 'fbRING\[p0\]@0x41000=[0-9a-f]+ nz[0-9]+/[0-9]+ [A-Za-z?-]+' "$Q" 2>/dev/null \
    | sort -u | sed 's/^/      /'
  echo "      ⊘ NO ROW ABOVE ⇒ this address was never dumped this boot — NOT MEASURED, not a change."
  echo "    --- standings, summed (context only; the address rows above are the grade):"
  echo "        JOINED-one-memory  = $(grep -c 'JOINED-one-memory' "$Q" 2>/dev/null)"
  echo "        resN-NEVER-WRITTEN = $(grep -c 'resN-NEVER-WRITTEN' "$Q" 2>/dev/null)"
  echo "        resY               = $(grep -c 'nz[0-9]*/4096 resY' "$Q" 2>/dev/null)"

  # =========================================================================================
  # ★★★★★ H14/H15 — THE REFUSALS, BY VA. A COUNT CANNOT SEE A SUBSTITUTION.
  # =========================================================================================
  echo "--- ★★★★★ H14/H15: EVERY refusal, by NAME and by ADDRESS:"
  echo "      DOORBELL-REFUSED = $(grep -c 'DOORBELL-REFUSED' "$Q" 2>/dev/null)   (w280_client: 1)"
  echo "      every distinct FwdFault named this boot:"
  grep -oE 'FwdFault::[A-Za-z]+' "$Q" 2>/dev/null | sort | uniq -c | sed 's/^/        /'
  echo "      ⊘ NONE ABOVE ⇒ zero refusals — which is w246d corner D's result, not an absence of measurement"
  echo "      --- ★★★★★ H1: PushbufferAperture, by VA. ⊘ w280_client: GpuVa(4831969280) ="
  echo "          0x1_2002_0000 = THE PUSHBUFFER, pb=V:0x40000. H1 predicts that VA is GONE on the"
  echo "          armed arm and PRESENT on the control. A COUNT CANNOT SEE THIS — grade the VA."
  grep -oE 'PushbufferAperture \{ va: GpuVa\([0-9]+\)' "$Q" 2>/dev/null | sort | uniq -c | sed 's/^/        /'
  python3 - "$Q" <<'PY' 2>/dev/null || echo "        ⊘ VA DECODE FAILED — no verdict"
import re, sys
seen = set()
for line in open(sys.argv[1], errors='replace'):
    for m in re.findall(r'GpuVa\((\d+)\)', line):
        seen.add(int(m))
    for m in re.findall(r'va: GpuVa\((\d+)\)', line):
        seen.add(int(m))
if seen:
    print("        decoded VAs: " + " ".join(f"0x{v:x}" for v in sorted(seen)))
    print("        ⊘ 0x120020000 = THE PUSHBUFFER (w280_client's wall); 0x120021000 = the RING")
    print("        ⊘ 0x120022000 = the SEMAPHORE the client declares — a refusal THERE is H6, the wall moving on")
else:
    print("        ⊘ no GpuVa printed this boot")
PY
  echo "      --- ★★★ RingFbNeverWritten, BY PHYS (H14: a hit at a phys ≠ 0x1024000 is a NEWLY"
  echo "          REACHABLE guard on another channel, NOT a refutation of H11a):"
  echo "          count = $(grep -c 'RingFbNeverWritten' "$Q" 2>/dev/null)"
  grep -oE 'RingFbNeverWritten \{[^}]*\}' "$Q" 2>/dev/null | sort | uniq -c | sed 's/^/          /'
  echo "      --- the FIRST doorbell refusal, WHOLE (the only line carrying the VA + identity):"
  grep -o 'first doorbell refusal.\{0,700\}' "$Q" 2>/dev/null | head -2 | sed 's/^/        /'

  # =========================================================================================
  # ★★★★★ H12/H13 — DID THE RING GET READ, AND ARE THEY THE SAME BYTES?
  # =========================================================================================
  echo "--- ★★★★★ H12: FWD-RING (w246d, before the join: 8 lines, bytes=65536 live=1 spans=0):"
  echo "      FWD-RING lines = $(grep -c 'FWD-RING' "$Q" 2>/dev/null)   (w277, route B OFF: 0)"
  grep -o 'FWD-RING.\{0,240\}' "$Q" 2>/dev/null | sort -u | sed 's/^/      /' | head -20
  echo "      --- ★★★ graded on the KEY: the walling channel is key=0xc1d0000c:0x5c00004b"
  echo "          FWD-RING lines carrying that key = $(grep 'FWD-RING' "$Q" 2>/dev/null | grep -c '0x5c00004b')"
  echo "--- ★★★★★ H13 THE ONE OPEN DEVICE QUESTION — does the JOIN serve the SAME BYTES?"
  echo "      ⊘ w279/w280_client both read: gp[0]@0x120021000=0x120020000+0x40, nonzero=[0]=0x0000400120020000"
  echo "      this boot's gp[0] entries (all distinct):"
  grep -oE 'gp\[0\]@0x[0-9a-f]+=0x[0-9a-f]+\+0x[0-9a-f]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
  echo "      this boot's nonzero=[0] words (all distinct):"
  grep -oE 'nonzero=\[0\]=0x[0-9a-f]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
  echo "      the pushbuffer's APERTURE and decoded methods (⊘ `pb=S:` = sysmem ⇒ lib.rs:4752's"
  echo "      hard-coded VidmemRoute::Refuse is NOT on this path; `pb=V:` = vidmem ⇒ it IS):"
  grep -oE 'pb=[SV]:0x[0-9a-f]+ pbm\[[0-9]+w of [0-9]+B\]' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
  # =========================================================================================
  # ★★★★★ w282b — THE OPERAND TABLE. THE NAMED BLOCKER, graded by VA.
  # =========================================================================================
  echo "--- ★★★★★ THE OPERAND TABLE — w282 walled here (2 page(s) asked, 2 MISS):"
  grep -o 'OPERAND-TABLE.\{0,300\}' "$Q" 2>/dev/null | sort -u | sed 's/^/      /' | head -4
  echo "      ⊘ NO ROW ⇒ the CE operand decode never happened — NOT MEASURED, not 'resolved'."
  grep -o 'OPERAND-SOURCE-CE.\{0,200\}' "$Q" 2>/dev/null | sort -u | sed 's/^/      /' | head -3
  echo "      OPERAND-PIN lines = $(grep -c 'OPERAND-PIN' "$Q" 2>/dev/null)"
  grep -o 'OPERAND-PIN.\{0,180\}' "$Q" 2>/dev/null | sort -u | sed 's/^/      /' | head -3
  # =========================================================================================
  # ★★★★★ LEG 7 — THE RUNG. GRADED BY IDENTITY, NEVER BY COUNT.
  #
  # ⊘⊘ w281b's pre-registered falsifier had BOTH HALVES FIRE while the rung failed, because
  #    the Xid vanished for the wrong reason (`by=HostCe` → `by=Ours`). THIRD instance in
  #    three rungs. ⇒ Every row below names an ADDRESS or a KEY.
  # =========================================================================================
  echo "--- ★★★★★ LEG 7: THE OPERAND JOIN. ⊘ clientoff MUST print ZERO of these:"
  echo "      OPERAND-JOIN-TABLE lines = $(grep -c 'OPERAND-JOIN-TABLE' "$Q" 2>/dev/null)"
  grep -o 'OPERAND-JOIN-TABLE.\{0,320\}' "$Q" 2>/dev/null | sort -u | sed 's/^/      /' | head -4
  echo "      ⊘ NO ROW on the ARMED arm ⇒ leg 7 never ran — NOT MEASURED, not 'nothing to join'."
  echo "    --- the per-leaf JOIN replies, BY fb_phys (⊘ w281b: the ONLY adopted leaf was"
  echo "        fb_phys=0x40000, the RING's; the operands are 0x10000 and 0x20000):"
  grep -oE 'CE-OPERAND\(chan=[0-9]+ fb_phys=0x[0-9a-f]+\) leaf va=0x[0-9a-f]+ len=0x[0-9a-f]+ fb_phys=0x[0-9a-f]+ → [A-Z-]+[^,]*' \
    "$Q" 2>/dev/null | sort -u | sed 's/^/        /' | head -8
  echo "        every JOINED/REFUSED verdict this boot carrying CE-OPERAND, verbatim:"
  grep -o 'CE-OPERAND.\{0,260\}' "$Q" 2>/dev/null | sort -u | sed 's/^/        /' | head -8
  echo "        the pass's own tally:"
  grep -oE 'JOINED [0-9]+ leaf/leaves, [0-9]+ REFUSED, over [0-9]+ distinct leaf/leaves' \
    "$Q" 2>/dev/null | sort | uniq -c | sed 's/^/        /' | head -4
  echo "    --- ★★★★★ THE WALK, by VA → leaf (⊘ a `NO FRAMEBUFFER LEAF` row is the finding):"
  grep -oE 'va=0x[0-9a-f]+ → leaf va=0x[0-9a-f]+ len=0x[0-9a-f]+ fb_phys=0x[0-9a-f]+' \
    "$Q" 2>/dev/null | sort -u | sed 's/^/        /' | head -8
  grep -o 'va=0x[0-9a-f]* → ⊘ NO FRAMEBUFFER LEAF.\{0,160\}' "$Q" 2>/dev/null | sort -u | sed 's/^/        /' | head -4
  # =========================================================================================
  # ★★★★★ #255 — THE OWNER'S ASSERTION, AND IT IS THIS RUNG'S FALSIFIER.
  #
  # ★ It has a GUARANTEED KNOWN-POSITIVE: on `clientoff` w281b measured both operands
  #   Vidmem@0x10000 / Vidmem@0x20000 with no host object, so #255 MUST print FIRED there.
  #   ⊘ A `QUIET` on the control means the INSTRUMENT did not run — a census zero needs a
  #   known-positive, and this is it. On `client` it must go QUIET, naming the host VAs.
  # =========================================================================================
  echo "--- ★★★★★ #255 FAKE-FB-IN-USERSPACE-VAS — clientoff MUST say FIRED, client MUST say QUIET:"
  echo "      lines = $(grep -c '#255 FAKE-FB-IN-USERSPACE-VAS' "$Q" 2>/dev/null)   ⊘ 0 ⇒ the instrument NEVER RAN"
  echo "      ⊘⊘ A ZERO ON THE CONTROL IS THE INSTRUMENT NOT RUNNING, NOT AN ABSENCE OF THE"
  echo "         CONDITION — that is exactly what w282_clientoff measured on the two-arm draft."
  echo "      FIRED = $(grep -c '#255 FAKE-FB-IN-USERSPACE-VAS.*FIRED' "$Q" 2>/dev/null)  QUIET = $(grep -c '#255 FAKE-FB-IN-USERSPACE-VAS.*QUIET' "$Q" 2>/dev/null)  NOT-ASKED = $(grep -c '#255 FAKE-FB-IN-USERSPACE-VAS.*NOT ASKED' "$Q" 2>/dev/null)"
  echo "      the build it compiled as (⊘ release REPORTS ONLY — the debug_assert is additive):"
  grep -oE '#255 FAKE-FB-IN-USERSPACE-VAS build=[a-z]+' "$Q" 2>/dev/null | sort -u | sed 's/^/        /'
  echo "      every verdict, verbatim and BY ADDRESS:"
  grep -o '#255 FAKE-FB-IN-USERSPACE-VAS.\{0,400\}' "$Q" 2>/dev/null | sort -u | sed 's/^/        /' | head -6
  echo "--- ★★★★★ THE SWEEP's own numbers (w276: bound=0 swept_binds=0 on ALL 88 rows, on a GR"
  echo "    arm where parse_pushbuffer never ran. This arm is CE and it DOES run):"
  grep -oE 'bound=[0-9]+ swept_binds=[0-9]+[^ ]*' "$Q" 2>/dev/null | sort | uniq -c | sed 's/^/      /' | head -6
  echo "--- ★★★★ H4: DID THE METHODS DECODE? The client independently printed SET_OBJECT 0xc7b5"
  echo "    and the semaphore 0x120022000; w280_client's descent decoded pbm[16w of 64B]. A"
  echo "    pushbuffer READ THROUGH THE NEW ROUTE must decode to the SAME words:"
  grep -o 'pbm\[[0-9]*w of [0-9]*B\]:.\{0,320\}' "$Q" 2>/dev/null | sort -u | sed 's/^/      /' | head -6
  echo "--- ⚠ H5 THE REGRESSION ARM — a BLANK vidmem pushbuffer must forward NOTHING."
  echo "    ⊘ w282 measured that a blank page decodes to Opaque methods, NOT to zero methods,"
  echo "    so the discriminator is FACTS not COUNTS: no CE span, no semaphore, no SetObject."
  echo "      FWD-RING spans= values: [$(grep -oE 'spans=[0-9]+' "$Q" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      CE-SUBMIT lines = $(grep -c 'CE-SUBMIT' "$Q" 2>/dev/null)   (w280_client: 0)"
  echo "    --- ★★★★★ w283: WAS THE GUEST'S OWN RELEASE CARRIED? graded BY ADDRESS."
  echo "        ⊘ CARRIED means EMITTED into the pushbuffer, NOT observed at the guest's VA."
  echo "        Only the client's own 'semaphore 0x...' read is the witness for that."
  echo "        guest_rel=CARRIED = $(grep -c 'guest_rel=CARRIED' "$Q" 2>/dev/null)   guest_rel=NONE = $(grep -c 'guest_rel=NONE' "$Q" 2>/dev/null)"
  grep -oE 'guest_rel=CARRIED@0x[0-9a-f]+ payload=0x[0-9a-f]+' "$Q" 2>/dev/null | sort -u | sed 's/^/        /' | head -4
  echo "        ⊘ the guest DECLARED 0x120022000 payload=0x1 — a CARRIED at any OTHER address"
  echo "          is us naming a semaphore the guest did not, and is a FAILURE not a pass."
  grep -o 'CE-SUBMIT.\{0,200\}' "$Q" 2>/dev/null | sort -u | sed 's/^/      /' | head -5
  echo "    --- ★★★★★ THE RUNG'S FALSIFIER, ON IDENTITY. ⊘ w281's falsifier was over a COUNT"
  echo "        and BOTH HALVES FIRED while the rung failed, because \`by=\` was substituted"
  echo "        underneath it. The ONLY reading that means progress is: CE-SUBMIT still says"
  echo "        \`by=HostCe\` AND the Xid is gone. \`by=Ours\` is a REGRESSION wearing a green."
  echo "        by=HostCe lines = $(grep -c 'CE-SUBMIT.*by=HostCe' "$Q" 2>/dev/null)   by=Ours lines = $(grep -c 'CE-SUBMIT.*by=Ours' "$Q" 2>/dev/null)"
  echo "        REFUSED BEFORE SUBMISSION = $(grep -c 'REFUSED BEFORE SUBMISSION' "$Q" 2>/dev/null)   NEVER-RETIRED = $(grep -c 'NEVER-RETIRED' "$Q" 2>/dev/null)"

  # =========================================================================================
  # ★★ H16 — GP_GET. ⊘ NOT A DISCRIMINATOR: already GET=1 PUT=1 in w277 with route B OFF.
  # =========================================================================================
  echo "--- ★★ H16: USERD cursors (⊘ w277 already had fbuserd@0x1026088 GET=1 PUT=1 with route B"
  echo "    OFF — a moving GP_GET here is NOT this rung's doing and must not be reported as one):"
  grep -oE 'fbuserd@0x[0-9a-f]+ GET=[0-9]+ PUT=[0-9]+ [A-Za-z?-]+' "$Q" 2>/dev/null \
    | sort | uniq -c | sort -k2 | sed 's/^/      /' | head -24
  echo "      --- the WALLING channel's USERD specifically (@0x1026088):"
  grep -oE 'fbuserd@0x1026088 GET=[0-9]+ PUT=[0-9]+ [A-Za-z?-]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
  echo "      ⚠ BAR1 GP_PUT witness has MEASURED false positives (guest DATA at page offset 0x8c"
  echo "        of a non-USERD page). Labelled, never filtered:"
  grep -o 'BAR1 GP_PUT.\{0,200\}' "$Q" 2>/dev/null | tail -3 | sed 's/^/      /'

  # =========================================================================================
  # ★★★★★ THE VERDICT LINES
  # =========================================================================================
  echo "--- ★★★★★ THE CLIENT'S VERDICT, ANCHORED (an unanchored 'R33' matches the info banner"
  echo "    that CONTAINS the words of the success line — the CUP2_RC/GCC_CUP2_RC class):"
  echo "      R33_VERDICT_LINES = [$(grep -c '^★     R33 raw CE client' "$P" 2>/dev/null)]  (1 = all arms met)"
  echo "      R33_RC            = [$(grep -oE '^R33_RC=[0-9]+' "$P" 2>/dev/null | tail -1)]   (w279/w280: R33_RC=1)"
  echo "      ⊘ unanchored, for contrast: [$(grep -c 'R33 raw CE client' "$P" 2>/dev/null)] line(s)"
  echo "--- ★★★★★ H3 THE THREE PASS-CRITERIA, from the client's OWN arm-1 line. ⊘ NAME WHICH"
  echo "    OF THE THREE IS MET; anything less is a stage, not a pass:"
  echo "      (1) GP_GET catches GP_PUT  (2) the bytes MOVED, read back  (3) the semaphore"
  echo "      carries the DECLARED payload at the DECLARED address"
  grep -E '^(★|FAIL|\?\?)  *R33 arm 1 COPY' "$P" 2>/dev/null | sed 's/^/      /'
  echo "--- EVERY arm, verbatim, from the guest's own output:"
  grep -E '^(★|FAIL|\?\?|ok|info) +R33 ' "$P" 2>/dev/null | sed 's/^/      /'
  echo "--- guest dmesg persisted (⊘ an EMPTY file reads as capture — assert the CONTENT):"
  echo "      run_${tag}_dmesg.log = $(stat -c %s /workspace/bench/run_${tag}_dmesg.log 2>/dev/null || echo MISSING) bytes, NVRM lines = $(grep -ci nvrm /workspace/bench/run_${tag}_dmesg.log 2>/dev/null)"
  echo "--- ★★★★★ Xid IDENTITY (⊘ never by count — a count cannot see a substitution):"
  grep -o 'Xid.*' "$D" 2>/dev/null | cut -c1-200 | sort -u | sed 's/^/      /'
  echo "      ENGINE/CLIENT pairs = [$(grep -oE 'ENGINE [A-Z0-9_]+ HUBCLIENT_[A-Z0-9_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      faulted @           = [$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      ⊘ an EMPTY host dmesg is 'no fault' ONLY if the watermark says so:"
  grep -E 'HOST_DMESG_(LINES|XID)=|watermark' "$P" 2>/dev/null | sed 's/^/        /' | head -4
  # =========================================================================================
  # ★★★★★ cup2 — THE OWNER'S ACTUAL QUESTION, ANCHORED.
  #
  # ⊘⊘ `^CUP2_RC=` ANCHORED. `GCC_CUP2_RC=0` matched UNANCHORED and reported the campaign's
  #    headline success value on a hanging arm. The unanchored count is printed BESIDE it as
  #    the contrast, never as the verdict.
  # ⊘ And a KNOWN-POSITIVE on my own grep: the pattern is run over the file I name, and the
  #   file's own size is printed, so `the decisive grep ran over zero files` is visible.
  # =========================================================================================
  if [ "$kind" = cup2 ]; then
    echo "--- ★★★★★ cup2 — THE OWNER'S QUESTION. probe=$P size=$(stat -c %s "$P" 2>/dev/null || echo MISSING) bytes"
    echo "      ^CUP2_RC= ANCHORED  = [$(grep -oE '^CUP2_RC=[A-Z0-9_]+' "$P" 2>/dev/null | tail -1)]"
    echo "      ⊘ unanchored, for CONTRAST ONLY (never the verdict): [$(grep -c 'CUP2_RC=' "$P" 2>/dev/null)] line(s):"
    grep -oE '[A-Z_]*CUP2_RC=[A-Z0-9_]+' "$P" 2>/dev/null | sort | uniq -c | sed 's/^/        /'
    echo "      ⊘ NO ANCHORED ROW ⇒ the hook did not produce a verdict — that is NOT a 124 and"
    echo "        is NOT a pass; it is an unrun workload. GCC_RC and the hook's own lines:"
    grep -E '^(GCC_RC=|HOOK_RC=|★ cup2_hook)' "$P" 2>/dev/null | sed 's/^/        /' | head -6
    echo "      the guest's own cup2 output, verbatim:"
    grep -E 'bad=|maxerr=|cuCtxCreate|CUDA error|matmul' "$P" 2>/dev/null | sed 's/^/        /' | head -10
  fi
}

# =============================================================================================
# ARM 1 — the raw CE client with the pushbuffer route ARMED. THE RUNG. Runs FIRST.
# =============================================================================================
ARMS=${W282_ARMS:-"client clientoff cup2"}
echo "=== ARMS TO RUN: [$ARMS] $([ "$ARMS" = "client clientoff cup2" ] || echo '⚠ NOT THE FULL SET — name the boot this borrows its other arms from')"

for a in $ARMS; do
  TAG=${PFX}_${a}
  unset POST_CAPTURE_HOOK KAYFABE_R33_BIN KAYFABE_CUP2_BIN GQ_TIMEOUT \
        KAYFABE_RING_VIDMEM KAYFABE_PUSHBUF_VIDMEM KAYFABE_PT_SWEEP KAYFABE_OPERAND_JOIN
  case "$a" in
    # ★★★★★ THE RUNG. w281b's exact configuration PLUS leg 7.
    client)    export KAYFABE_RING_VIDMEM=on
               export KAYFABE_PUSHBUF_VIDMEM=on
               export KAYFABE_PT_SWEEP=on
               export KAYFABE_OPERAND_JOIN=join
               export POST_CAPTURE_HOOK=$REPO/scripts/bench/r33_hook_ce_client.sh
               export KAYFABE_R33_BIN=$CLIENT; export GQ_TIMEOUT=240 ;;
    # ⊘ THE CONTROL — byte-identical arming to `w281b_clientsweep`, so this rung's ONE
    #   variable is `KAYFABE_OPERAND_JOIN` and nothing else. ⚠ It is re-run rather than
    #   borrowed from w281b: the device source changed between them, and comparing against an
    #   older boot confounds the flag with everything else that changed (bf9f13e's own point).
    clientoff) export KAYFABE_RING_VIDMEM=on
               export KAYFABE_PUSHBUF_VIDMEM=on
               export KAYFABE_PT_SWEEP=on
               # ★★★ `assert`, NOT unset. ⊘⊘ The first draft used `off`, and w282_clientoff
               #   MEASURED the defect: with #255 inside the armed path the control printed
               #   ZERO #255 lines, so the instrument's guaranteed known-positive was
               #   unreachable and `QUIET` and `never ran` were the same observation.
               #   `assert` classifies and states #255 and JOINS NOTHING — behaviourally
               #   identical to `off`, and its expected reading is `#255 ... FIRED`, a
               #   POSITIVE observation rather than an absence.
               export KAYFABE_OPERAND_JOIN=assert
               export POST_CAPTURE_HOOK=$REPO/scripts/bench/r33_hook_ce_client.sh
               export KAYFABE_R33_BIN=$CLIENT; export GQ_TIMEOUT=240 ;;
    # ★★★★★ THE OWNER'S ACTUAL QUESTION — *"I am curious if it passes the cup2 boundary"*.
    # ⊘ Armed IDENTICALLY to `client`. cup2's 16 pushbuffers are `pb=S:` (w280), so the
    #   PUSHBUF_VIDMEM route is NOT on its path — but leg 7 is keyed on where the OPERAND
    #   lands, not on where the pushbuffer lands, and w260 measured cuCtxCreate's census
    #   naming THREE framebuffer leaves. ⇒ this arm is not a foregone 124 and is graded
    #   `^CUP2_RC=` ANCHORED (`GCC_CUP2_RC=0` matched unanchored and reported the campaign's
    #   headline success value on a hanging arm).
    cup2)      export KAYFABE_RING_VIDMEM=on
               export KAYFABE_PUSHBUF_VIDMEM=on
               export KAYFABE_PT_SWEEP=on
               export KAYFABE_OPERAND_JOIN=join
               export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_deadline.sh
               export GQ_TIMEOUT=420 ;;
    *)         echo "=== ★★★ UNKNOWN ARM '$a' ==="; finish 99 ;;
  esac
  echo "=== BOOT $TAG START $(date -Is) — workload=$a hook=$POST_CAPTURE_HOOK RING_VIDMEM=[${KAYFABE_RING_VIDMEM:-unset}] PUSHBUF_VIDMEM=[${KAYFABE_PUSHBUF_VIDMEM:-unset}] PT_SWEEP=[${KAYFABE_PT_SWEEP:-unset}] OPERAND_JOIN=[${KAYFABE_OPERAND_JOIN:-unset}] ==="
  timeout 1500 "$REPO/scripts/bench/boot_capture.sh" "$TAG"
  echo "=== BOOT $TAG RC=$? $(date -Is) ==="
  echo "=== ★★★★★ GRADE $TAG ($a) ==="
  grade "$TAG" "$a"
done

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_${PFX}_* /workspace/bench/xid_${PFX}_* 2>/dev/null
finish 0
