#!/usr/bin/env bash
# ★★★★★ w287 — PASSTHROUGH FOR REAL: the raw CE client in the guest, with BOTH FALLBACKS CUT.
#
# Derived from `w278_run.sh`, arming carried byte for byte. ⊘ **THE WORKLOAD IS UNCHANGED
# FROM w278 AND SO IS THE ARMING.** Exactly two things differ, and both are code:
#
#   1. USERD now lives INSIDE the client's own ring object at `+0x3000`
#      (`rm.rs USERD_OFFSET_IN_RING`) instead of in a second `alloc_device_local`. w284
#      measured the old layout across three identical boots: ring leaf fb 0x40000 len 0x10000,
#      USERD fb 0x50000 — the FIRST BYTE PAST THE END — so `adopted_guest_userd`'s containment
#      test declined and leg B could never fire for our own client.
#   2. Ring CONTENT is no longer forwarded on a `Passthrough` channel
#      (`ring_content_is_forwardable` now takes `GuestChannelKind`). w283d had ONE CE doorbell
#      ring the adopted channel (host_token=0x6) AND run `ce_copy` on host_token=0x7.
#
# ★★★★★ THE NATIVE ARM IS THE KNOWN-POSITIVE FOR CHANGE 1, and it is run below from the same
# binary, minutes before the boot: it asks whether real RM accepts `hUserdMemory` = the ring
# object with `userdOffset=0x3000` at all. `[measured 2026-08-13, vh, bare metal GA106]` it
# does — `GP_GET 1 caught GP_PUT 1`, all four facts met, teardown clean.
#
# ⊘ WHAT THIS RUN CANNOT SAY: nothing here bounds `cup2`. The ladder builds its OWN
#   `FERMI_VASPACE_A`; the GR channel `cuCtxCreate` walls on belongs to the guest driver's
#   client with its own PDB. `cup2` is deliberately NOT run.
#
# ONE boot. A second arm would vary the device, and the device is not what is being varied.
set -uo pipefail
PFX=${W287_TAG_PREFIX:-w287}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W287 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W287 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w287}
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w268
export KAYFABE_SHIM_FEATURES=host-isolates

# ★★★ THE CLIENT IS BUILT FIRST, and STATIC. A dynamic binary that fails to load in the
#     guest reports as "no GPU", which is a different finding wearing this one's clothes.
echo "=== BUILD the static client $(date -Is) ==="
cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder
CRC=$?
echo "=== CLIENT BUILD RC=$CRC ==="
[ $CRC -eq 0 ] || finish 95
# ⊘⊘ **CARGO_TARGET_DIR REDIRECTS THE OUTPUT, AND THE INHERITED PATH DID NOT KNOW IT.**
# `[measured 2026-08-13, w287 boot 1]` this line read `$REPO/target/...` (carried from
# `w278_run.sh`, which ran without a redirect). With `CARGO_TARGET_DIR` exported 20 lines
# above, the binary is built to `$CARGO_TARGET_DIR/...` and `$REPO/target` does not exist.
# ⇒ The build reported `CLIENT BUILD RC=0`, the native arm reported `NATIVE ARM RC=127`, the
# md5 line printed EMPTY, the boot ran to completion and the grade block printed `R33_RC=[]`.
# ★ Every one of those reads as a result. **A missing binary must be fatal HERE**, not a
# blank in a verdict eight steps later — the whole `zero-bytes-is-not-not-yet` class.
CLIENT=${CARGO_TARGET_DIR:-$REPO/target}/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
[ -x "$CLIENT" ] || { echo "=== ★★★ NO CLIENT BINARY AT $CLIENT — every arm below would be VOID ==="; finish 97; }
[ -s "$CLIENT" ] || { echo "=== ★★★ CLIENT BINARY IS ZERO BYTES ==="; finish 97; }
file "$CLIENT"
echo "=== CLIENT md5=$(md5sum < "$CLIENT" | cut -d' ' -f1) ==="

# ★★★★★ THE NATIVE ARM RUNS FROM THIS EXACT BINARY, HERE, IMMEDIATELY BEFORE THE BOOT.
#     ⊘ Not carried from an earlier session: a differential whose reference is a number in a
#     document is a differential against a document. Both halves of this rung's diff are
#     produced by one script, from one file, within minutes of each other.
echo "=== ★★★★★ NATIVE ARM (bare metal, same binary) $(date -Is) ==="
timeout 240 ./scripts/bench/host_xid_watch.sh ${PFX}_native -- "$CLIENT" --ce-client
NRC=$?
echo "=== NATIVE ARM RC=$NRC ==="
# ★★★★★ **THE NATIVE ARM IS THIS RUNG'S KNOWN-POSITIVE, so a red one VOIDS the guest arm
# rather than merely accompanying it.** It is the only thing that says real RM accepts
# `hUserdMemory` = the ring object at `userdOffset=0x3000`. If it fails, a red guest arm
# cannot be attributed to the guest path — and `[measured, w287 boot 1]` an RC of 127 from a
# missing binary sat in this log looking exactly like a measurement.
[ $NRC -eq 0 ] || { echo "=== ★★★ NATIVE ARM FAILED (rc=$NRC) — the guest arm would be UNINTERPRETABLE ==="; finish 98; }

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

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export KAYFABE_R33_BIN=$CLIENT
export POST_CAPTURE_HOOK=$REPO/scripts/bench/r33_hook_ce_client.sh
export GQ_TIMEOUT=240
export BOOT_TIMEOUT=180
[ -x scripts/bench/r33_hook_ce_client.sh ] || { echo "=== ★★★ THE HOOK IS NOT EXECUTABLE ==="; finish 96; }

# The carried arming — w271_pin's, unchanged. Named so a mis-armed boot is not a data point
# that looks like one.
export KAYFABE_FB_JOIN=shared
export KAYFABE_GUEST_RING=ring
export KAYFABE_GUEST_PUSHBUF=pin
export KAYFABE_PT_WITNESS_EXEC=on
export KAYFABE_GUEST_SEMA=pin
export KAYFABE_GR_ROUTE=passthrough
export KAYFABE_GUEST_OPERAND=pin
unset KAYFABE_PT_SWEEP

# ★★★★★ THE SECOND ARM'S ONE VARIABLE — `route B`, and it is ALREADY CALIBRATED.
#
# `[measured, w246, the four-corner square in shim.rs's RING_VIDMEM_ENV docs]`
#   witness=on RING_VIDMEM=off -> PushbufferAperture = 8
#   witness=on RING_VIDMEM=ON  -> PushbufferAperture = 0
# ⊘ And route B is UNREACHABLE with the witness disarmed (`plan_gpfifo_ring` returns
#   `RingVaUnbound` BEFORE `VidmemRoute` is computed) — this rung carries `witness=on`, so the
#   flag is the only variable. Never measure it with the witness off.
#
# ⚠ `w246` also recorded that route B **enumerates a ring; it does not submit work** — its
#   `CE-SUBMIT` was 0 in all four corners. So a green pushbuffer read here is NOT the first
#   forwarded work, and this arm must not be read as one.
unset KAYFABE_RING_VIDMEM
[ "${W287_RING_VIDMEM:-off}" = "on" ] && export KAYFABE_RING_VIDMEM=on
echo "=== ARM: KAYFABE_RING_VIDMEM=[${KAYFABE_RING_VIDMEM:-unset}] (route B) ==="

TAG=${PFX}_guest
echo "=== BOOT $TAG START $(date -Is) ==="
timeout 1200 "$REPO/scripts/bench/boot_capture.sh" "$TAG"
echo "=== BOOT $TAG RC=$? $(date -Is) ==="

Q=/workspace/bench/run_${TAG}_qemu.log
P=/workspace/bench/run_${TAG}_probe.log
D=/workspace/bench/run_${TAG}_hostdmesg.log

echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
echo "--- ENOSPC_LLVM=$(grep -c 'No space left on device\|LLVM ERROR' "$Q" 2>/dev/null || echo '?')"
echo "--- ARTEFACT SIZES: qemu=$(stat -c %s "$Q" 2>/dev/null || echo MISSING) probe=$(stat -c %s "$P" 2>/dev/null || echo MISSING) hostdmesg=$(stat -c %s "$D" 2>/dev/null || echo MISSING)"

# ★★ The six carried arms, asserted out of the device's own lines.
for pair in "FB-JOIN arm=shared" "GUEST-RING arm=ring" "GUEST-PUSHBUF arm=pin" \
            "GUEST-SEMA arm=pin" "GR-ROUTE arm=passthrough" "GUEST-OPERAND arm=pin"; do
  grep -q "kayfabe: $pair" "$Q" 2>/dev/null \
    && echo "    ★ CARRIED-ARM: PASS ($pair)" \
    || echo "    ★★★ CARRIED-ARM: FAIL — wanted '$pair'. ⊘ VOID for comparison."
done

# =========================================================================================
# ★★★★★ THE GRADE — anchored, by IDENTITY, and the native half is in this same log
# =========================================================================================
echo "=== ★★★★★ R33 IN THE GUEST — THE VERDICT ==="
echo "--- did the binary reach the guest at all (⊘ a DIFFERENT failure from 'it did not work'):"
grep -E 'GUEST_MD5=|GUEST_EXECUTABLE=|GUEST_NVIDIA_DEVS=|GUEST_NVRM_LOADED=|GUEST_UNAME=' "$P" 2>/dev/null | sed 's/^/      /'
echo "--- ⊘ the two md5s MUST match, or the arms are not the same program:"
echo "      NATIVE md5 = $(md5sum < "$CLIENT" | cut -d' ' -f1)"

echo "--- ★★★★★ THE VERDICT LINE, ANCHORED (an unanchored 'R33' matches the info banner that"
echo "    CONTAINS the words of the success line — the CUP2_RC/GCC_CUP2_RC class):"
echo "      R33_VERDICT_LINES = [$(grep -c '^★     R33 raw CE client' "$P" 2>/dev/null)]  (1 = all arms met)"
echo "      R33_RC            = [$(grep -oE '^R33_RC=[0-9]+' "$P" 2>/dev/null | tail -1)]"
echo "      ⊘ unanchored, for contrast: [$(grep -c 'R33 raw CE client' "$P" 2>/dev/null)] line(s)"

echo "--- ★★★★★ EVERY ARM, verbatim, from the guest's own output:"
grep -E '^(★|FAIL|\?\?|ok|info) +R33 ' "$P" 2>/dev/null | sed 's/^/      /'

echo "--- ★★★★★ THE IOCTL CENSUS IN THE GUEST (native reference: total=53 failed=0):"
grep -E '^  total=[0-9]+ failed=' "$P" 2>/dev/null | sed 's/^/      /'
echo "    --- by phase:"
sed -n '/--- by phase/,/--- by request/p' "$P" 2>/dev/null | head -12 | sed 's/^/      /'
echo "    --- by request (the driver's own NV_ESC numbers):"
sed -n '/--- by request/,/--- THE SEQUENCE/p' "$P" 2>/dev/null | head -14 | sed 's/^/      /'
echo "    --- the LAST three ioctls (where it died, if it did):"
grep -E '^ +[0-9]+: nr ' "$P" 2>/dev/null | tail -3 | sed 's/^/      /'
echo "    ⚠ A MATCHING TOTAL IS NECESSARY, NOT SUFFICIENT — a count cannot see a"
echo "      substitution. The arms above are graded by IDENTITY (payload, GP_GET/GP_PUT,"
echo "      the read-back words), never by the count."

echo "--- the guest driver's own word (⊘ an empty section here is evidence of NOTHING):"
grep -A14 "the guest driver's own word" "$P" 2>/dev/null | tail -14 | sed 's/^/      /'
echo "--- guest dmesg NVRM lines captured to the boot's own file:"
echo "      run_${TAG}_dmesg.log = $(stat -c %s /workspace/bench/run_${TAG}_dmesg.log 2>/dev/null || echo MISSING) bytes, NVRM lines = $(grep -ci nvrm /workspace/bench/run_${TAG}_dmesg.log 2>/dev/null)"
echo "--- HOST dmesg delta across the boot (a host Xid is OURS, not the guest's):"
cat "$D" 2>/dev/null | sed 's/^/      /' | head -20

# =========================================================================================
# ★★★★★ WHAT OUR DEVICE DID WITH THE CLIENT'S DOORBELL — the half the client cannot see
# =========================================================================================
echo "=== ★★★★★ THE DEVICE'S OWN VIEW OF THE CLIENT'S SUBMISSION ==="
echo "--- ⊘ ATTRIBUTION FIRST: the client's ring, in the device's own roster (its hClient and"
echo "    its 64-entry GPFIFO are what identify it — a token alone would not):"
grep -E 'RING-ROSTER' "$Q" 2>/dev/null | grep 'entries=64' | sed 's/^/      /'
echo "--- DOORBELL-XLATE (how many doorbells our device translated at all): $(grep -c 'DOORBELL-XLATE' "$Q" 2>/dev/null)"
grep -E 'DOORBELL-XLATE' "$Q" 2>/dev/null | sed 's/^/      /'
echo "--- ★★★ DID OUR CHIP CODEC DECODE THE CLIENT'S PUSHBUFFER? (the operand and semaphore"
echo "    addresses below are the CLIENT'S OWN — compare them against the arm-1 line):"
grep -E 'SEMA-SOURCE-CE|OPERAND-SOURCE-CE' "$Q" 2>/dev/null | cut -c1-320 | sed 's/^/      /'
echo "--- and what the address table answered for them:"
grep -E 'SEMA-TABLE:|OPERAND-TABLE:' "$Q" 2>/dev/null | cut -c1-320 | sed 's/^/      /'
echo "--- ★★★★★ THE FORWARD'S VERDICT:"
echo "      DOORBELL-REFUSED   = $(grep -c 'DOORBELL-REFUSED' "$Q" 2>/dev/null)"
echo "      PushbufferAperture = $(grep -c 'PushbufferAperture' "$Q" 2>/dev/null)   (w246: 8 with route B off, 0 with it ON)"
echo "      SERVED-LOCAL       = $(grep -c 'SERVED-LOCAL' "$Q" 2>/dev/null)   (⊘ these are the KERNEL's CeUtils channels, not ours)"
grep -E 'DOORBELL.*(REFUSED|SERVED-REMOTE|FORWARDED)' "$Q" 2>/dev/null | cut -c1-240 | tail -6 | sed 's/^/      /'
echo "      ⚠ w246: route B ENUMERATES a ring; it does not submit work (CE-SUBMIT was 0 in all"
echo "        four corners). A green pushbuffer read is NOT the first forwarded work."

# =========================================================================================
# ★★★★★ w287's OWN GRADE — the two facts this rung changed, and nothing else
# =========================================================================================
echo "=== ★★★★★ w287 — DID THE CUT LAND? ==="

echo "--- ⊘ KNOWN-POSITIVE FIRST. Every grep below is worthless if the file it reads is empty"
echo "    or the pattern never matches ANYTHING (today's decisive grep ran over zero files)."
echo "      GR-BIRTH lines present at all = [$(grep -c 'GR-BIRTH' "$Q" 2>/dev/null)]  (0 ⇒ every"
echo "        verdict below is VOID, not favourable)"
echo "      RING-PROJ lines present       = [$(grep -c 'RING-PROJ' "$Q" 2>/dev/null)]"

echo "--- ★★★★★ CHANGE 1 — THE CLIENT'S USERD. The client's channel is the 64-entry one."
echo "    ⊘ 'DECLINED' is w284's measured state and is the thing that had to move."
echo "      userd=GUEST-USERD (adopted) = [$(grep 'GR-BIRTH' "$Q" 2>/dev/null | grep -c 'userd=GUEST-USERD')]"
echo "      userd=DECLINED              = [$(grep 'GR-BIRTH' "$Q" 2>/dev/null | grep -c 'userd=DECLINED')]"
echo "    --- every CE birth verbatim (the client's has entries=64; the driver's have 1024):"
grep -E 'GR-BIRTH' "$Q" 2>/dev/null | grep 'engine=Ce' | cut -c1-320 | sed 's/^/      /'
echo "    --- ★★★ THE TWO NUMBERS THAT DECIDED IT — the ring's joined leaf and the USERD's"
echo "        placement. w284: leaf fb 0x40000 sz 0x10000 vs userd fb 0x50000 (one byte past)."
grep -E 'RING-PROJ' "$Q" 2>/dev/null | grep -oE 'userd=[^ ]+|LEAF@[^ ]+' | sed 's/^/      /' | head -20

echo "--- ★★★★★ CHANGE 2 — THE FORK IS CUT. On a Passthrough channel we must no longer read,"
echo "    decode and re-emit the guest's ring. ⊘ A DROP TO ZERO HERE IS THE POINT, not a"
echo "    regression: w283d's CE-SUBMIT came from host_token 0x7 while the guest's own"
echo "    channel was 0x6, so it was never evidence about the passthrough channel."
echo "      CE-SUBMIT total    = [$(grep -c 'CE-SUBMIT' "$Q" 2>/dev/null)]"
echo "      SEMA-SOURCE-CE     = [$(grep -c 'SEMA-SOURCE-CE' "$Q" 2>/dev/null)]   (the decode path running at all)"
echo "      OPERAND-SOURCE-CE  = [$(grep -c 'OPERAND-SOURCE-CE' "$Q" 2>/dev/null)]"
echo "      DOORBELL-XLATE     = [$(grep -c 'DOORBELL-XLATE' "$Q" 2>/dev/null)]   (⊘ must NOT drop — the doorbell still forwards)"
echo "      DOORBELL-STORE ★★★ WROTE = [$(grep -c 'DOORBELL-STORE.*WROTE' "$Q" 2>/dev/null)]"

echo "--- ★★★★★ THE ACCEPTANCE CRITERION, ANCHORED. Criterion 1 is GP_GET ADVANCING, i.e. the"
echo "    GUEST driving a HARDWARE CE. ⊘ 'bytes moved' and 'semaphore landed' are NOT it —"
echo "    w283c printed those two green while GP_GET stood at 0."
grep -E '^★ +R33 arm 1 COPY|^FAIL +R33 arm 1 COPY' "$P" 2>/dev/null | sed 's/^/      /'
echo "      GP_GET/GP_PUT as the client saw them = [$(grep -oE 'GP_GET [0-9]+ (caught )?GP_PUT [0-9]+' "$P" 2>/dev/null | tail -1)]"
echo "      R33_RC = [$(grep -oE '^R33_RC=[0-9]+' "$P" 2>/dev/null | tail -1)]"

echo "--- ★★ NO EMULATED FRAMEBUFFER IN THE PASSTHROUGH VAS (a standing owner requirement)."
echo "    ⚠ #255 is computed INSIDE the CE decode path, which change 2 deliberately stops"
echo "      running — so a ZERO here is 'the instrument did not run', NOT 'the condition is"
echo "      absent'. It needs its own input; that is named as open, not scored as a pass."
echo "      #255/ShadowsGuestMemory lines = [$(grep -cE '#255|ShadowsGuestMemory' "$Q" 2>/dev/null)]"
echo "      Fabricated/emulated-FB operands = [$(grep -cE 'Fabricated' "$Q" 2>/dev/null)]"

echo "--- named refusals this rung can produce (0x4B56 = USERD_OFFSET_MISALIGNED):"
echo "      USERD_OFFSET_MISALIGNED = [$(grep -c '4B56\|19286\|USERD_OFFSET_MISALIGNED' "$Q" 2>/dev/null)]"
echo "      USERD_NOT_OURS          = [$(grep -c 'USERD_NOT_OURS\|4B55' "$Q" 2>/dev/null)]"
echo "      RingFbNeverWritten      = [$(grep -c 'RingFbNeverWritten' "$Q" 2>/dev/null)]"

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_${PFX}_* /workspace/bench/xid_${PFX}_* 2>/dev/null
finish 0
