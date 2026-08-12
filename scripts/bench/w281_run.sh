#!/usr/bin/env bash
# ★★★★★ w281 — THE PUSHBUFFER READ OUT OF OUR OWN FRAMEBUFFER. w279's named next step.
#
# Pre-registration: `traces/boots/w281/PREREGISTRATION.md` — committed BEFORE this runs.
#
# ⊘⊘ THE RUNG: `w279` ended at `PushbufferAperture { va: 0x1_2002_0000 }` — the ring is read
#    through the join, and the PUSHBUFFER THE RING POINTS AT is refused by a hard-coded
#    `VidmemRoute::Refuse` put there so that boot could attribute the ring's bytes. `w280`
#    measured the raw client's pushbuffer is `pb=V:0x40000` — VIDMEM — so that gate is on the
#    client's path and on NOTHING ELSE (16/16 of cup2's are `pb=S:`). This lifts it.
#
# ⊘⊘ IT IS ITS OWN FLAG — `KAYFABE_PUSHBUF_VIDMEM`, never route B's — per w279's explicit
#    ruling. It is NECESSARY-NOT-SUFFICIENT alone: the bytes come from route B's FbSource, so
#    `KAYFABE_RING_VIDMEM` must ALSO be on. Both polarities asserted, on both arms.
#
# THE ARMS:
#   arm             workload                            RING_VIDMEM   PUSHBUF_VIDMEM
#   w281_client     kayfabe-rm-ladder --ce-client (53)  on            ON     ← the rung
#   w281_clientoff  the SAME-REVISION CONTROL           on            OFF    ← isolates the flag
# ★ The rung's own arm runs FIRST: if the session is cut short, the control is what is
#   missing, and a control is recoverable where the measurement is not.
#
# ⊘ cup2 is NOT run this rung, deliberately: `w280` measured 16/16 `pb=S:`, so this gate is
#   not on its path and a cup2 arm could only reproduce a CUP2_RC=124 already committed twice.
#
# ★ START marker and EXIT line so "file exists but has no terminator" is detectable at all
#   (143 = the JOB was SIGTERMed; 124 = the LAUNCHER's ssh expired while the job ran on).
set -uo pipefail
PFX=${W281_TAG_PREFIX:-w281}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W281 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W281 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w281}
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
echo "  ★★ w281 WIDENING (reported, not enforced): client pushbuffer/ring/sema literals = $NCAP2 (want 0)"

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
for f in scripts/bench/r33_hook_ce_client.sh; do
  [ -x "$f" ] || { echo "=== ★★★ $f IS MISSING OR NOT EXECUTABLE — the hook cannot run ==="; finish 96; }
done
# ⊘ No cup2.c check: cup2 is not run this rung (see the header).

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
  # ★★★★★ THE RUNG'S OWN FLAG, and the CONTROL asserts the OPPOSITE — an unarmed control
  # that is silently armed is the same defect as an armed run that is silently unarmed.
  local wantpb='ON'; [ "$kind" = clientoff ] && wantpb='OFF'
  grep -qE "kayfabe: PUSHBUF-VIDMEM .*pushbuffer route $wantpb" "$Q" 2>/dev/null \
    && echo "    ★★★ PUSHBUF-VIDMEM: PASS (pushbuffer route $wantpb — this arm wanted $wantpb)" \
    || { echo "    ★★★ PUSHBUF-VIDMEM: FAIL — wanted 'pushbuffer route $wantpb'. ⊘ VOID for this rung."; \
         grep -o 'PUSHBUF-VIDMEM.\{0,160\}' "$Q" 2>/dev/null | head -1 | sed 's/^/       saw: /'; }
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
  echo "--- ★★★★ H4: DID THE METHODS DECODE? The client independently printed SET_OBJECT 0xc7b5"
  echo "    and the semaphore 0x120022000; w280_client's descent decoded pbm[16w of 64B]. A"
  echo "    pushbuffer READ THROUGH THE NEW ROUTE must decode to the SAME words:"
  grep -o 'pbm\[[0-9]*w of [0-9]*B\]:.\{0,320\}' "$Q" 2>/dev/null | sort -u | sed 's/^/      /' | head -6
  echo "--- ⚠ H5 THE REGRESSION ARM — a BLANK vidmem pushbuffer must forward NOTHING."
  echo "    ⊘ w281 measured that a blank page decodes to Opaque methods, NOT to zero methods,"
  echo "    so the discriminator is FACTS not COUNTS: no CE span, no semaphore, no SetObject."
  echo "      FWD-RING spans= values: [$(grep -oE 'spans=[0-9]+' "$Q" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      CE-SUBMIT lines = $(grep -c 'CE-SUBMIT' "$Q" 2>/dev/null)   (w280_client: 0)"
  grep -o 'CE-SUBMIT.\{0,200\}' "$Q" 2>/dev/null | sort -u | sed 's/^/      /' | head -5

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
}

# =============================================================================================
# ARM 1 — the raw CE client with the pushbuffer route ARMED. THE RUNG. Runs FIRST.
# =============================================================================================
ARMS=${W281_ARMS:-"client clientoff"}
echo "=== ARMS TO RUN: [$ARMS] $([ "$ARMS" = "client clientoff" ] || echo '⚠ NOT THE FULL PAIR — name the boot this borrows its other half from')"

for a in $ARMS; do
  TAG=${PFX}_${a}
  unset POST_CAPTURE_HOOK KAYFABE_R33_BIN GQ_TIMEOUT KAYFABE_RING_VIDMEM KAYFABE_PUSHBUF_VIDMEM
  case "$a" in
    # ★★★★★ THE RUNG. Route B ON (the supply) + PUSHBUF_VIDMEM ON (the route).
    client)    export KAYFABE_RING_VIDMEM=on
               export KAYFABE_PUSHBUF_VIDMEM=on
               export POST_CAPTURE_HOOK=$REPO/scripts/bench/r33_hook_ce_client.sh
               export KAYFABE_R33_BIN=$CLIENT; export GQ_TIMEOUT=240 ;;
    # ⊘ THE CONTROL. Route B still ON — only the NEW flag differs, so a difference between
    #   the two arms is THIS FLAG and not the device delta. (bf9f13e's own point: comparing
    #   against an older boot confounds the flag with everything else that changed.)
    clientoff) export KAYFABE_RING_VIDMEM=on
               # KAYFABE_PUSHBUF_VIDMEM deliberately UNSET.
               export POST_CAPTURE_HOOK=$REPO/scripts/bench/r33_hook_ce_client.sh
               export KAYFABE_R33_BIN=$CLIENT; export GQ_TIMEOUT=240 ;;
    *)         echo "=== ★★★ UNKNOWN ARM '$a' ==="; finish 99 ;;
  esac
  echo "=== BOOT $TAG START $(date -Is) — workload=$a hook=$POST_CAPTURE_HOOK RING_VIDMEM=[${KAYFABE_RING_VIDMEM:-unset}] PUSHBUF_VIDMEM=[${KAYFABE_PUSHBUF_VIDMEM:-unset}] ==="
  timeout 1500 "$REPO/scripts/bench/boot_capture.sh" "$TAG"
  echo "=== BOOT $TAG RC=$? $(date -Is) ==="
  echo "=== ★★★★★ GRADE $TAG ($a) ==="
  grade "$TAG" "$a"
done

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_${PFX}_* /workspace/bench/xid_${PFX}_* 2>/dev/null
finish 0
