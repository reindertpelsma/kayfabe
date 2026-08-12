#!/usr/bin/env bash
# ★★★★★ w278 / R33 MILESTONE 2 — the RAW CE CLIENT, inside the guest.
#
# Pre-registration: `traces/boots/w278/PREREGISTRATION.md` — committed BEFORE this runs.
#
# ⊘⊘ **THE DEVICE IS UNCHANGED.** The arming is `w271_pin`'s, carried byte for byte from
#    `w277_run.sh`. The only new thing inside the guest is a **userspace program**: a
#    statically linked `kayfabe-rm-ladder --ce-client`, the SAME binary (same md5) that
#    produced the native reference. ⇒ There is no "arm" to assert here beyond the carried
#    six; the variable is the WORKLOAD, and `cup2` is deliberately NOT run.
#
# ONE boot. A second arm would vary the device, and the device is not what is being varied.
set -uo pipefail
PFX=${W278_TAG_PREFIX:-w278}
OUT=/workspace/${PFX}_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W278 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W278 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=${KAYFABE_REPO:-/workspace/kayfabe_w278}
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
CLIENT=$REPO/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
file "$CLIENT"
echo "=== CLIENT md5=$(md5sum < "$CLIENT" | cut -d' ' -f1) ==="

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

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_${PFX}_* /workspace/bench/xid_${PFX}_* 2>/dev/null
finish 0
