#!/usr/bin/env bash
# ★★★★★ w279 — THE SAME 53-IOCTL CLIENT, over a JOIN-AWARE residency question.
#
# Pre-registration: `traces/boots/w279/PREREGISTRATION.md` — committed BEFORE this runs.
#
# ⊘⊘ **THE WORKLOAD AND THE ARMING ARE w278b's, BYTE FOR BYTE.** This file is `w278_run.sh`
#    with exactly two defaults changed (the tag prefix and the repo path) and one flipped:
#    `KAYFABE_RING_VIDMEM` defaults **on**, because `w278b` — the arm that produced
#    `RingFbNeverWritten` — is the arm being re-run. The variable is the DEVICE's residency
#    question, and it is a source change, not an env var. ⇒ Diff this against `w278_run.sh`
#    before reading any number out of it.
#
# ⊘⊘ **THE DEVICE IS UNCHANGED.** The arming is `w271_pin`'s, carried byte for byte from
#    `w277_run.sh`. The only new thing inside the guest is a **userspace program**: a
#    statically linked `kayfabe-rm-ladder --ce-client`, the SAME binary (same md5) that
#    produced the native reference. ⇒ There is no "arm" to assert here beyond the carried
#    six; the variable is the WORKLOAD, and `cup2` is deliberately NOT run.
#
# ONE boot. A second arm would vary the device, and the device is not what is being varied.
set -uo pipefail
PFX=${W279_TAG_PREFIX:-w279}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W279 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W279 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w279}
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
# ★★★★★ w279 MEASURED — THE PATH WAS WRONG AND `RC=0` HID IT, FOR A WHOLE BOOT.
#
# `[measured 2026-08-12, /workspace/w279_run.log, first attempt]` `cargo build` returned 0,
# `CLIENT BUILD RC=0` printed, and the very next line was `CLIENT md5=` — EMPTY. The binary
# is written under $CARGO_TARGET_DIR (exported above), not $REPO/target, so `$CLIENT` named
# a file that does not exist. The native arm died `rc=127` and `KAYFABE_R33_BIN` pointed at
# nothing, so the guest ran NO CLIENT AT ALL — and the boot still reported
# `RingFbNeverWritten = 0`, which is this rung's PREDICTED SUCCESS. ⊘ An absent artefact
# reads as favourable; only the `fbRING rows = [0] ⇒ VOID` known-positive separated them.
#
# ⇒ Resolve against CARGO_TARGET_DIR, and ASSERT THE FILE, not the exit status. A build
# system's `0` means "I had nothing to do", which is also what it says when it built
# somewhere else.
CLIENT=${CARGO_TARGET_DIR:-$REPO/target}/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
if [ ! -s "$CLIENT" ]; then
  echo "=== ★★★ CLIENT MISSING OR EMPTY at $CLIENT — the build's RC=0 said nothing about it ==="
  ls -l "${CARGO_TARGET_DIR:-$REPO/target}/x86_64-unknown-linux-musl/release/" 2>&1 | head -20
  finish 97
fi
file "$CLIENT"
CLIENT_MD5=$(md5sum < "$CLIENT" | cut -d' ' -f1)
echo "=== CLIENT md5=$CLIENT_MD5 ==="
[ -n "$CLIENT_MD5" ] || { echo "=== ★★★ EMPTY md5 — refusing to boot over an unidentified binary ==="; finish 98; }

# ★★★★★ THE NATIVE ARM RUNS FROM THIS EXACT BINARY, HERE, IMMEDIATELY BEFORE THE BOOT.
#     ⊘ Not carried from an earlier session: a differential whose reference is a number in a
#     document is a differential against a document. Both halves of this rung's diff are
#     produced by one script, from one file, within minutes of each other.
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
[ "${W279_RING_VIDMEM:-on}" = "on" ] && export KAYFABE_RING_VIDMEM=on
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
echo "      NATIVE md5 = $CLIENT_MD5"

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
# ★★★★★ w279's OWN GRADE — the join-aware residency question, read from the device's lines
# =========================================================================================
echo "=== ★★★★★ w279: DID THE REFUSAL MOVE? ==="
echo "--- H1/H3 the named refusals, counted (w278b: RingFbNeverWritten fired, aperture 0):"
echo "      RingFbNeverWritten = $(grep -c 'RingFbNeverWritten' "$Q" 2>/dev/null)   (H1 predicts 0; H3 is it still firing)"
echo "      PushbufferAperture = $(grep -c 'PushbufferAperture' "$Q" 2>/dev/null)   (H4)"
echo "      DOORBELL-REFUSED   = $(grep -c 'DOORBELL-REFUSED' "$Q" 2>/dev/null)"
echo "--- ⊘ EVERY distinct FwdFault named in this boot, so a NEW refusal cannot read as none:"
grep -oE 'FwdFault::[A-Za-z]+' "$Q" 2>/dev/null | sort | uniq -c | sed 's/^/      /'
echo "--- ★★★ H5 THE INSTRUMENT ITSELF — the standing token on the client's ring page."
echo "    ⊘ KNOWN-POSITIVE FOR THIS GREP: the pattern must match SOMETHING first. If the"
echo "      count below is 0 the grep is void, not the finding (I ran a decisive grep over"
echo "      zero files once already this campaign)."
echo "      fbRING rows in this log = [$(grep -c 'fbRING' "$Q" 2>/dev/null)]  ⊘ 0 ⇒ VOID, not 'no join'"
echo "      JOINED-one-memory       = [$(grep -c 'JOINED-one-memory' "$Q" 2>/dev/null)]  (H5 predicts >0)"
echo "      resN-NEVER-WRITTEN      = [$(grep -c 'resN-NEVER-WRITTEN' "$Q" 2>/dev/null)]"
grep -oE 'fbRING\[p0\]@0x[0-9a-f]+=[0-9a-f]+ nz[0-9]+/[0-9]+ [A-Za-z?-]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'
echo "--- ★★ H8 THE REGRESSION ARM — a ring SERVED whose page is nz0 is forbidden #2 back:"
echo "      SERVED-LOCAL = $(grep -c 'SERVED-LOCAL' "$Q" 2>/dev/null)"
grep -oE 'fbRING\[p0\]@0x[0-9a-f]+=0{16,} nz0/[0-9]+ [A-Za-z?-]+' "$Q" 2>/dev/null | sed 's/^/      ⚠ ZERO-RING: /'
echo "--- H10 the ring's own bytes, as the device read them (must be the CLIENT'S entry):"
grep -oE 'gp\[0\]@0x[0-9a-f]+=0x[0-9a-f]+\+0x[0-9a-f]+' "$Q" 2>/dev/null | sort -u | sed 's/^/      /'

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_${PFX}_* /workspace/bench/xid_${PFX}_* 2>/dev/null
finish 0
