#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# gop_standin.sh — kf3's boot-display GOP (firmware/kf-gop) on a stand-in device: local, no GPU,
# no kf3. docs/design/V3_DISPLAY.md §4.11.8 records what each arm showed and when.
#
#   usage: scripts/display/gop_standin.sh [--keep] [arm ...]        (default: every arm)
#
# The stand-in is QEMU's `ati-vga` (ATI Rage 128 Pro, 1002:5046): a VGA-class (0x0300) device that
# OVMF has NO built-in driver for — like a real NVIDIA GPU, and unlike QEMU's stdvga, which OVMF's
# QemuVideoDxe claims before any option-ROM driver runs (arm `stdvga_builtin_wins`). Its BAR0 is
# plain RAM, so the host reads the framebuffer back with the monitor's `pmemsave`. The SAME
# `kf-gop.efi` kf3 ships — built here from firmware/kf-gop, the source crates/kf-gop-image embeds
# (CI's `firmware` job checks the two builds are byte-identical) — is packed by `kf-oprom` for the
# stand-in's ids, BAR 0, at an odd mode (1152x648, pitch 4608) so nothing passes by matching a default.
#
# Arms (each boots one VM: q35, 512 MiB, 1 vCPU, KVM when usable, else TCG):
#   gop_ubuntu            Ubuntu OVMF + the test app: every GOP check, then the framebuffer read
#                         back byte-exact against the pattern the app drew, at the descriptor's pitch.
#   gop_qemu_edk2         the same on QEMU's own edk2-x86_64-code.fd (what a QEMU build ships).
#   stdvga_builtin_wins   stdvga + a kf-gop ROM: OVMF's QemuVideoDxe binds first (expected; recorded).
#   neg_wrong_id          a ROM whose descriptor names another device id: kf-gop refuses by name.
#   neg_no_descriptor     a ROM whose descriptor magic is broken: kf-gop refuses by name.
#   linux                 Ubuntu OVMF + the host kernel + a busybox initramfs: the EFI stub's
#                         framebuffer is the stand-in's BAR0 (BOOTFB nested in it, at +0, pitch·H
#                         long), simpledrm binds it, sysfb's parent is the device, boot_vga = 1, and
#                         a pattern Linux writes through /dev/fb0 lands in the BAR byte-exact.
#   linux_two_vga         a second VGA-class device without a GOP in a lower slot: boot_vga must
#                         follow the firmware framebuffer; the arm records both devices' decode bits.
#   sb_ms_unsigned_observe        Secure Boot with Microsoft's keys, unsigned ROM. ASSERTS that Secure
#                                 Boot enforces (the unsigned test app is denied); OBSERVES what OVMF
#                                 does with the unsigned ROM (verdict=OBSERVED).
#   sb_snakeoil_signed            Secure Boot with the snakeoil keys, ROM signed with the snakeoil key
#                                 (sbsign): must load and pass every check.
#   sb_snakeoil_unsigned_observe  the same keys, unsigned ROM: asserts secure_boot=1, observes the ROM.
#   sb_snakeoil_tailpad_observe   the signed PE placed at 0x200 with padding AFTER it (the layout
#                                 kf-oprom does not emit): asserts secure_boot=1, observes the ROM.
#   ⊘ CORRECTED 2026-10-03 (v3-gop, the review of v3-gop-kf3): the three `_observe` arms were named
#   sb_ms_unsigned, sb_snakeoil_unsigned and sb_snakeoil_tailpad and reported verdict=PASS whatever
#   the ROM did, so no arm could fail on the ROM outcome it was named for. Stock OVMF trusts every
#   option ROM (PcdOptionRomImageVerificationPolicy 0x00), so on it the ROM outcome can only be
#   observed; the arms now say so in their names and verdicts. The arms that CAN fail on it need a
#   firmware that verifies ROMs:
#   sb_deny_signed                OVMF built with PcdOptionRomImageVerificationPolicy=0x04 (deny on a
#                                 security violation) + snakeoil keys: the signed ROM in kf-oprom's
#                                 end-aligned layout MUST load and pass every check.
#   sb_deny_unsigned              the same firmware: the unsigned ROM MUST NOT start.
#   sb_deny_tailpad               the same firmware: the signed PE with padding after it MUST NOT
#                                 start — the premise of kf-oprom's end-aligned layout (pack.rs).
#   The sb_deny_* arms SKIP, saying so, unless OVMF_DENY_CODE (and OVMF_DENY_VARS, its VARS with the
#   snakeoil keys enrolled) name such a build; stock distribution OVMF never verifies option ROMs.
#   OVMF_DENY_CODE is booted like Ubuntu's Secure Boot build (SMM on, secure flash), so build it the
#   same way: from QEMU 10.2.4's roms/edk2, OvmfPkg/OvmfPkgX64.dsc with SECURE_BOOT_ENABLE=TRUE,
#   SMM_REQUIRE=TRUE, FD_SIZE_4MB and PcdOptionRomImageVerificationPolicy set to 0x04 (its
#   [PcdsDynamicDefault] row) — scripts/display/build_ovmf_deny.sh does exactly that; Ubuntu's
#   OVMF_VARS_4M.snakeoil.fd serves as OVMF_DENY_VARS.
#   Secure Boot arms skip, with the reason, when sbsign or the snakeoil keys are missing.
#
# env: QEMU (qemu-system-x86_64)  OVMF_DIR (/usr/share/OVMF)  OVMF_KEYS (/usr/share/ovmf)
#      QEMU_EDK2 (edk2-x86_64-code.fd[.bz2]; searched under /usr/share/qemu and $QEMU_SRC_DIRS)
#      KERNEL (/boot/vmlinuz-$(uname -r))  KERNEL_CONFIG (/boot/config-$(uname -r))
#      BUSYBOX (busybox, must be static)
#      KF_GOP_WORK (scratch dir; default a fresh mktemp -d, deleted at exit unless --keep)
#      CARGO_TARGET_DIR (default <scratch>/target)  KF_CARGO_LOCK (flock this file around cargo)
#      KF_CARGO_JOBS (2)  ACCEL (kvm|tcg)  ARM_TIMEOUT (seconds; 90 under KVM, 400 under TCG)
#      OVMF_DENY_CODE, OVMF_DENY_VARS (the sb_deny_* arms; see above)
# output: `GOP_STANDIN arm=<arm> verdict=PASS|FAIL|SKIP|OBSERVED <facts>` per arm, then
#         GOP_STANDIN_SUMMARY with each verdict counted separately. Exit 1 if any arm FAILs.
#         OBSERVED is an arm whose named outcome is a finding, not a requirement (the `_observe`
#         Secure Boot arms): it asserts what it can (that Secure Boot enforces) and FAILs when it
#         cannot, but its observation passes nothing — it is never counted as a PASS.
#         `stdvga_builtin_wins` asserts its outcome (OVMF's built-in driver binds) and can fail.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
KEEP=0
ARGS=()
for a in "$@"; do
    case "$a" in
        --keep) KEEP=1 ;;
        -h | --help) sed -n '3,/^set -uo/p' "$0" | sed '$d'; exit 0 ;;
        *) ARGS+=("$a") ;;
    esac
done
ALL_ARMS="gop_ubuntu gop_qemu_edk2 stdvga_builtin_wins neg_wrong_id neg_no_descriptor linux linux_two_vga sb_ms_unsigned_observe sb_snakeoil_signed sb_snakeoil_unsigned_observe sb_snakeoil_tailpad_observe sb_deny_signed sb_deny_unsigned sb_deny_tailpad"
ARMS="${ARGS[*]:-$ALL_ARMS}"

QEMU=${QEMU:-qemu-system-x86_64}
OVMF_DIR=${OVMF_DIR:-/usr/share/OVMF}
OVMF_KEYS=${OVMF_KEYS:-/usr/share/ovmf}
KERNEL=${KERNEL:-/boot/vmlinuz-$(uname -r)}
KERNEL_CONFIG=${KERNEL_CONFIG:-/boot/config-$(uname -r)}
BUSYBOX=${BUSYBOX:-$(command -v busybox || true)}
WORK=${KF_GOP_WORK:-$(mktemp -d "${TMPDIR:-/tmp}/kfgop-standin.XXXXXX")}
mkdir -p "$WORK"
cleanup() {
    for p in $(jobs -p); do kill "$p" 2>/dev/null; done
    if [ "$KEEP" -eq 0 ] && [ -z "${KF_GOP_WORK:-}" ]; then rm -rf "$WORK"; fi
    rm -f "${SOCKS[@]}" 2>/dev/null
}
SOCKS=()
trap cleanup EXIT
if [ -z "${ACCEL:-}" ]; then
    if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then ACCEL=kvm; else ACCEL=tcg; fi
fi
if [ "$ACCEL" = kvm ]; then CPU=host; else CPU=max; fi
ARM_TIMEOUT=${ARM_TIMEOUT:-$([ "$ACCEL" = kvm ] && echo 90 || echo 400)}

# The stand-in's identity and mode.
SI_VENDOR=0x1002 SI_DEVICE=0x5046 SI_CLASS=0x030000 SI_W=1152 SI_H=648

echo "GOP_STANDIN_START rev=$(git -C "$REPO" rev-parse --short=8 HEAD 2>/dev/null)$( [ -n "$(git -C "$REPO" status --porcelain --untracked-files=no 2>/dev/null)" ] && echo -dirty) $(date -Is)"
echo "GOP_STANDIN_HOST qemu=\"$($QEMU --version | head -1)\" accel=$ACCEL kernel=$(basename "$KERNEL") work=$WORK"
dpkg-query -W -f='GOP_STANDIN_HOST ovmf_package=${Version}\n' ovmf 2>/dev/null || true

# ---------------------------------------------------------------------------------------------
# Build: kf-oprom (the packer), the driver (release, and a debugcon test build), the test app.
# ---------------------------------------------------------------------------------------------
TGT=${CARGO_TARGET_DIR:-$WORK/target}
cargo_run() {
    if [ -n "${KF_CARGO_LOCK:-}" ]; then flock "$KF_CARGO_LOCK" cargo "$@"; else cargo "$@"; fi
}
(cd "$REPO" && cargo_run build --release -p kf-oprom -j"${KF_CARGO_JOBS:-2}" --target-dir "$TGT/ws" -q) || { echo "GOP_STANDIN_FATAL kf-oprom build failed"; exit 2; }
KFO="$TGT/ws/release/kf-oprom"
for flavour in release debugcon; do
    feat=(); [ "$flavour" = debugcon ] && feat=(--features debugcon)
    (cd "$REPO/firmware/kf-gop" && cargo_run build --release --target x86_64-unknown-uefi "${feat[@]}" \
        -j"${KF_CARGO_JOBS:-2}" --target-dir "$TGT/fw-$flavour" -q) || { echo "GOP_STANDIN_FATAL firmware ($flavour) build failed"; exit 2; }
done
EFI_REL="$TGT/fw-release/x86_64-unknown-uefi/release/kf-gop.efi"
EFI_DBG="$TGT/fw-debugcon/x86_64-unknown-uefi/release/kf-gop.efi"
EFI_APP="$TGT/fw-release/x86_64-unknown-uefi/release/kf-gop-test.efi"
"$KFO" pe "$EFI_REL" | sed 's/^/GOP_STANDIN_PE release /'
"$KFO" pe "$EFI_DBG" | sed 's/^/GOP_STANDIN_PE debugcon /'
echo "GOP_STANDIN_PE release_sha256=$(sha256sum "$EFI_REL" | cut -c1-64)"

# A 128-byte EDID with a valid header and checksum. The firmware only carries it; the test app
# checks the two EDID protocols return exactly these bytes.
python3 - "$WORK/edid.bin" <<'EOF'
import sys
b = bytes([0, 255, 255, 255, 255, 255, 255, 0]) + bytes((i * 7) & 0xFF for i in range(8, 127))
open(sys.argv[1], "wb").write(b + bytes([(-sum(b)) & 0xFF]))
EOF
pack() { # pack <efi> <out> <vendor> <device> [extra kf-oprom args]
    local efi=$1 out=$2 ven=$3 dev=$4; shift 4
    "$KFO" pack --efi "$efi" --out "$out" --vendor "$ven" --device "$dev" --class "$SI_CLASS" \
        --bar 0 --width "$SI_W" --height "$SI_H" --edid "$WORK/edid.bin" "$@" >/dev/null
}
pack "$EFI_REL" "$WORK/rel.rom" "$SI_VENDOR" "$SI_DEVICE" || { echo "GOP_STANDIN_FATAL pack failed"; exit 2; }
pack "$EFI_DBG" "$WORK/dbg.rom" "$SI_VENDOR" "$SI_DEVICE"
"$KFO" parse "$WORK/rel.rom" | sed 's/^/GOP_STANDIN_ROM /'

mkdir -p "$WORK/esp/EFI/BOOT"
cp "$EFI_APP" "$WORK/esp/EFI/BOOT/BOOTX64.EFI"

# QEMU's own edk2 build (what a QEMU installation ships as edk2-x86_64-code.fd).
find_qemu_edk2() {
    local c
    for c in ${QEMU_EDK2:-} /usr/share/qemu/edk2-x86_64-code.fd \
        ${QEMU_SRC_DIRS:-/workspace/bench/qemu-10.2.4 /workspace/qemu-next /workspace/qemu-build-src}; do
        [ -d "$c" ] && c="$c/pc-bios/edk2-x86_64-code.fd.bz2"
        if [ -f "$c" ]; then echo "$c"; return 0; fi
    done
    return 1
}
QEMU_EDK2_SRC=$(find_qemu_edk2 || true)
if [ -n "$QEMU_EDK2_SRC" ]; then
    case "$QEMU_EDK2_SRC" in
        *.bz2) bzip2 -dc "$QEMU_EDK2_SRC" > "$WORK/qemu-edk2-code.fd"
               bzip2 -dc "$(dirname "$QEMU_EDK2_SRC")/edk2-i386-vars.fd.bz2" > "$WORK/qemu-edk2-vars.fd" 2>/dev/null ;;
        *) cp "$QEMU_EDK2_SRC" "$WORK/qemu-edk2-code.fd"
           cp "$(dirname "$QEMU_EDK2_SRC")/edk2-i386-vars.fd" "$WORK/qemu-edk2-vars.fd" 2>/dev/null ;;
    esac
    echo "GOP_STANDIN_HOST qemu_edk2=$QEMU_EDK2_SRC md5=$(md5sum < "$WORK/qemu-edk2-code.fd" | cut -c1-32)"
fi

# ---------------------------------------------------------------------------------------------
# VM plumbing
# ---------------------------------------------------------------------------------------------
qmp() { # qmp <sock> <command> [json-arguments]: one QMP command; prints its reply, fails on an error
    python3 - "$@" <<'EOF'
import json, socket, sys
s = socket.socket(socket.AF_UNIX)
s.settimeout(60)
s.connect(sys.argv[1])
f = s.makefile("rw")
def reply():
    while True:
        line = f.readline()
        if not line:
            return {"error": "eof"}
        m = json.loads(line)
        if "return" in m or "error" in m:
            return m
json.loads(f.readline())  # greeting
f.write(json.dumps({"execute": "qmp_capabilities"}) + "\n"); f.flush(); reply()
cmd = {"execute": sys.argv[2]}
if len(sys.argv) > 3:
    cmd["arguments"] = json.loads(sys.argv[3])
f.write(json.dumps(cmd) + "\n"); f.flush()
r = reply() if sys.argv[2] != "quit" else {"return": {}}
print(json.dumps(r))
sys.exit(1 if "error" in r else 0)
EOF
}
pmemsave() { # pmemsave <sock> <guest-physical address> <bytes> <file>
    qmp "$1" pmemsave "{\"val\": $(($2)), \"size\": $(($3)), \"filename\": \"$4\"}" >/dev/null
}

# boot <dir> <code.fd> <vars.fd> <done-regex> <logs-to-watch,comma-separated> [qemu args...]: start
# the VM, wait for the regex in one of the logs (or the timeout, or QEMU's exit). Leaves the VM
# running; sets QPID and SOCK.
boot() {
    local d=$1 code=$2 vars=$3 done_re=$4 watch=$5; shift 5
    mkdir -p "$d"
    cp "$vars" "$d/vars.fd"
    SOCK=$(mktemp -u /tmp/kfgop.XXXXXX)
    SOCKS+=("$SOCK")
    local machine="q35,accel=$ACCEL"
    local sec=()
    # Ubuntu's Secure Boot build requires SMM, and SMM requires the secure flash.
    # shellcheck disable=SC2054  # the commas are QEMU's -global syntax, not array separators
    case "$code" in
        *secboot* | *.ms.fd | *snakeoil*) machine="$machine,smm=on"; sec=(-global driver=cfi.pflash01,property=secure,value=on) ;;
    esac
    # The deny-policy firmware (sb_deny_*) is a Secure Boot build too, whatever its file is called.
    if [ -n "${OVMF_DENY_CODE:-}" ] && [ "$code" = "$OVMF_DENY_CODE" ]; then
        machine="$machine,smm=on"
        # shellcheck disable=SC2054  # QEMU's -global syntax, as above
        sec=(-global driver=cfi.pflash01,property=secure,value=on)
    fi
    "$QEMU" -machine "$machine" "${sec[@]}" -cpu "$CPU" -m 512 -smp 1 -nodefaults -vga none -display none \
        -drive if=pflash,format=raw,unit=0,readonly=on,file="$code" \
        -drive if=pflash,format=raw,unit=1,file="$d/vars.fd" \
        -debugcon file:"$d/dbg.log" -global isa-debugcon.iobase=0x402 \
        -serial file:"$d/serial.log" -qmp unix:"$SOCK",server=on,wait=off -no-reboot \
        "$@" >"$d/qemu.log" 2>&1 &
    QPID=$!
    local t=0
    while [ "$t" -lt "$ARM_TIMEOUT" ]; do
        sleep 1; t=$((t + 1))
        for w in ${watch//,/ }; do
            tr -d '\0' < "$d/$w" 2>/dev/null | grep -aqE "$done_re" && { echo "$t" > "$d/seconds"; return 0; }
        done
        kill -0 "$QPID" 2>/dev/null || { echo "qemu exited: $(tail -3 "$d/qemu.log")" > "$d/why"; return 1; }
    done
    echo "timeout after ${ARM_TIMEOUT}s" > "$d/why"
    return 1
}
stop() {
    qmp "$SOCK" quit >/dev/null 2>&1
    for _ in 1 2 3 4 5; do kill -0 "$QPID" 2>/dev/null || break; sleep 1; done
    kill "$QPID" 2>/dev/null
    wait "$QPID" 2>/dev/null
    rm -f "$SOCK"
}
esp_drive() { ESP=(-drive "if=virtio,format=raw,readonly=on,file=fat:$1"); } # sets ESP
verdict() { # verdict <arm> PASS|FAIL|SKIP|OBSERVED <facts...>
    local arm=$1 v=$2; shift 2
    # A checker that crashed leaves no verdict word: that is a failure, never a silent pass.
    case "$v" in PASS | FAIL | SKIP | OBSERVED) ;; *) set -- "no verdict from the arm's checker ($v)" "$@"; v=FAIL ;; esac
    echo "GOP_STANDIN arm=$arm verdict=$v $*"
    RESULTS+=("$arm=$v")
}
RESULTS=()
esp_drive "$WORK/esp"

# The framebuffer the test app drew, compared byte for byte (visible pixels; the pitch padding and
# the 64 KiB tail are not drawn).
compare_pattern() { # compare_pattern <fb.bin> <width> <height> <pitch>
    python3 - "$@" <<'EOF'
import sys
fb = open(sys.argv[1], "rb").read()
w, h, pitch = int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
def px(x, y):  # kf_gop::test_pattern
    return bytes([(x * 3 + y * 5) & 0xFF, (x ^ y) & 0xFF, (x + y * 7) & 0xFF, 0])
bad, first = 0, None
for y in range(h):
    row = b"".join(px(x, y) for x in range(w))
    got = fb[y * pitch : y * pitch + w * 4]
    if got != row:
        bad += 1
        if first is None:
            i = next(i for i in range(len(row)) if i >= len(got) or got[i] != row[i])
            first = f"y={y} x={i // 4}"
print(f"pattern_rows_bad={bad}/{h}" + (f" first_bad={first}" if first else ""))
sys.exit(1 if bad else 0)
EOF
}

# ---------------------------------------------------------------------------------------------
# Arms
# ---------------------------------------------------------------------------------------------
gop_arm() { # gop_arm <arm> <code> <vars> <rom> [extra qemu args]
    local arm=$1 code=$2 vars=$3 rom=$4; shift 4
    local d="$WORK/$arm"
    if ! boot "$d" "$code" "$vars" 'KFGOP-TEST (READY|RESULT FAIL)' dbg.log \
        -device ati-vga,romfile="$rom" "${ESP[@]}" "$@"; then
        verdict "$arm" FAIL "boot: $(cat "$d/why")"; stop; return
    fi
    sleep 1
    local result fbline base pitch cmp
    result=$(grep -a 'KFGOP-TEST RESULT' "$d/dbg.log" | head -1)
    fbline=$(grep -a 'KFGOP-TEST PATTERN' "$d/dbg.log" | head -1)
    base=$(sed -n 's/.*fb_base=\(0x[0-9a-f]*\).*/\1/p' <<<"$fbline")
    pitch=$(sed -n 's/.*pitch=\([0-9]*\).*/\1/p' <<<"$fbline")
    cmp="no-pattern"
    if [ -n "$base" ]; then
        pmemsave "$SOCK" "$base" $((pitch * SI_H)) "$d/fb.bin"
        cmp=$(compare_pattern "$d/fb.bin" "$SI_W" "$SI_H" "$pitch")
    fi
    stop
    grep -a 'KFGOP-TEST check .* FAIL' "$d/dbg.log" | sed "s/^/GOP_STANDIN_DETAIL $arm /"
    local sb; sb=$(grep -ao 'secure_boot=[^ ]*' "$d/dbg.log" | head -1)
    if grep -q 'RESULT PASS' <<<"$result" && grep -q 'pattern_rows_bad=0/' <<<"$cmp"; then
        verdict "$arm" PASS "${result#KFGOP-TEST RESULT PASS } $cmp $sb fb_base=$base pitch=$pitch boot_s=$(cat "$d/seconds")"
    else
        verdict "$arm" FAIL "result=\"$result\" $cmp $sb"
    fi
}

arm_gop_ubuntu() { gop_arm gop_ubuntu "$OVMF_DIR/OVMF_CODE_4M.fd" "$OVMF_DIR/OVMF_VARS_4M.fd" "$WORK/rel.rom"; }

arm_gop_qemu_edk2() {
    if [ ! -f "$WORK/qemu-edk2-code.fd" ] || [ ! -s "$WORK/qemu-edk2-vars.fd" ]; then
        verdict gop_qemu_edk2 SKIP "no QEMU edk2-x86_64-code.fd (+ edk2-i386-vars.fd) found; set QEMU_EDK2"; return
    fi
    gop_arm gop_qemu_edk2 "$WORK/qemu-edk2-code.fd" "$WORK/qemu-edk2-vars.fd" "$WORK/rel.rom"
    # QEMU's build is a DEBUG build: it says why an option-ROM driver runs late, and whether OVMF's
    # text console drew through kf-gop's single mode.
    local d="$WORK/gop_qemu_edk2"
    grep -a 'is deferred to load before EndOfDxe\|can be loaded after EndOfDxe\|GraphicsConsole video resolution\|Graphics Console Started' \
        "$d/dbg.log" | head -4 | sed 's/^/GOP_STANDIN_DETAIL gop_qemu_edk2 edk2: /'
}

arm_stdvga_builtin_wins() {
    local d="$WORK/stdvga_builtin_wins"
    pack "$EFI_DBG" "$WORK/stdvga.rom" 0x1234 0x1111
    if ! boot "$d" "$OVMF_DIR/OVMF_CODE_4M.fd" "$OVMF_DIR/OVMF_VARS_4M.fd" 'KFGOP-TEST (READY|RESULT FAIL)' dbg.log \
        -device VGA,romfile="$WORK/stdvga.rom",vgamem_mb=16 "${ESP[@]}"; then
        verdict stdvga_builtin_wins FAIL "boot: $(cat "$d/why")"; stop; return
    fi
    stop
    local loaded started mode
    loaded=$(grep -ac 'kf-gop: loaded' "$d/dbg.log")
    started=$(grep -ac 'kf-gop: started' "$d/dbg.log")
    mode=$(grep -a 'check one_mode' "$d/dbg.log" | grep -o 'max=[0-9]*')
    if [ "$loaded" -ge 1 ] && [ "$started" -eq 0 ] && [ -n "$mode" ] && [ "${mode#max=}" -gt 1 ]; then
        verdict stdvga_builtin_wins PASS "observed=builtin-driver-bound kf_gop_loaded=$loaded kf_gop_started=0 gop_modes=${mode#max=}"
    else
        verdict stdvga_builtin_wins FAIL "observed=unexpected loaded=$loaded started=$started $mode"
    fi
}

neg_arm() { # neg_arm <arm> <rom> <expected refusal regex>
    local arm=$1 rom=$2 want=$3 d="$WORK/$1"
    if ! boot "$d" "$OVMF_DIR/OVMF_CODE_4M.fd" "$OVMF_DIR/OVMF_VARS_4M.fd" 'KFGOP-TEST (READY|RESULT FAIL)' dbg.log \
        -device ati-vga,romfile="$rom" "${ESP[@]}"; then
        verdict "$arm" FAIL "boot: $(cat "$d/why")"; stop; return
    fi
    stop
    local refusal ours
    refusal=$(grep -a 'kf-gop: Supported refuses' "$d/dbg.log" | head -1)
    ours=$(grep -ao 'ours=[0-9]*' "$d/dbg.log" | head -1)
    if grep -qE "$want" <<<"$refusal" && [ "$ours" = ours=0 ] && ! grep -aq 'kf-gop: started' "$d/dbg.log"; then
        verdict "$arm" PASS "refusal=\"${refusal#kf-gop: Supported refuses: }\" test_app_$ours"
    else
        verdict "$arm" FAIL "refusal=\"$refusal\" $ours"
    fi
}
arm_neg_wrong_id() {
    pack "$EFI_DBG" "$WORK/wrongid.rom" "$SI_VENDOR" 0x5047
    neg_arm neg_wrong_id "$WORK/wrongid.rom" 'ids 1002:5046 but the descriptor names 1002:5047'
}
arm_neg_no_descriptor() {
    cp "$WORK/dbg.rom" "$WORK/nodesc.rom"
    printf 'X' | dd of="$WORK/nodesc.rom" bs=1 seek=$((0x40)) conv=notrunc status=none
    neg_arm neg_no_descriptor "$WORK/nodesc.rom" 'KFGP: no KFGP magic'
}

# A busybox initramfs whose /init reports what Linux made of the firmware framebuffer.
build_initramfs() {
    [ -f "$WORK/initramfs.cpio.gz" ] && return 0
    local r="$WORK/initramfs"
    mkdir -p "$r/bin" "$r/proc" "$r/sys" "$r/dev"
    cp "$BUSYBOX" "$r/bin/busybox"
    # The pattern Linux writes through /dev/fb0: the test app's pattern, at the descriptor's pitch.
    python3 - "$r/pattern.bin" "$SI_W" "$SI_H" 4608 <<'EOF'
import sys
w, h, pitch = int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
def px(x, y):  # kf_gop::test_pattern
    return bytes([(x * 3 + y * 5) & 0xFF, (x ^ y) & 0xFF, (x + y * 7) & 0xFF, 0])
with open(sys.argv[1], "wb") as f:
    for y in range(h):
        f.write(b"".join(px(x, y) for x in range(w)).ljust(pitch, b"\0"))
EOF
    cat > "$r/init" <<'EOF'
#!/bin/busybox sh
B=/bin/busybox
$B mount -t proc proc /proc
$B mount -t sysfs sysfs /sys
$B mount -t devtmpfs devtmpfs /dev
$B sleep 2
echo "KFLINUX begin $($B uname -r)"
$B dmesg | $B grep -iE 'efifb|simple|sysfb|vgaarb|BOOTFB|framebuffer|fbcon|\[drm\]' | $B sed 's/^/KFLINUX dmesg /'
$B sed 's/^/KFLINUX iomem /' /proc/iomem
for d in /sys/bus/pci/devices/*; do
    c=$($B cat "$d/class")
    case "$c" in 0x03*)
        cmd=$($B od -An -tx2 -j4 -N2 "$d/config" | $B tr -d ' ')
        echo "KFLINUX pci $($B basename "$d") class=$c vendor=$($B cat "$d/vendor") device=$($B cat "$d/device") boot_vga=$($B cat "$d/boot_vga" 2>/dev/null || echo none) command=0x$cmd bar0=$($B head -1 "$d/resource" | $B cut -d' ' -f1,2 | $B tr ' ' '-')" ;;
    esac
done
for p in /sys/bus/platform/devices/simple-framebuffer.*; do
    [ -e "$p" ] && echo "KFLINUX sysfb $($B readlink -f "$p") driver=$($B basename "$($B readlink -f "$p/driver" 2>/dev/null)")"
done
for f in name virtual_size stride bits_per_pixel; do echo "KFLINUX fb0 $f=$($B cat /sys/class/graphics/fb0/$f 2>/dev/null)"; done
# Draw through Linux's own framebuffer device; the host then compares the BAR with the pattern,
# byte for byte, at the pitch. fbcon is unbound first so nothing draws over it. `[2026-10-03, local
# 7.0.0-34-generic]` a write() to simpledrm's fbdev lands in its shadow and reached the BAR only at
# the next full commit; blank/unblank is that commit (fbcon's own text, drawn through the same
# device, landed at once).
for v in /sys/class/vtconsole/vtcon*; do
    $B grep -q 'frame buffer' "$v/name" 2>/dev/null && echo 0 > "$v/bind"
done
if [ -c /dev/fb0 ] && $B dd if=/pattern.bin of=/dev/fb0 bs=4608 2>/dev/null; then
    echo 1 > /sys/class/graphics/fb0/blank
    $B sleep 1
    echo 0 > /sys/class/graphics/fb0/blank
    echo "KFLINUX fb0 drawn"
fi
$B sleep 2
echo "KFLINUX end"
while true; do $B sleep 60; done
EOF
    chmod +x "$r/init"
    (cd "$r" && find . | cpio -o -H newc 2>/dev/null | gzip) > "$WORK/initramfs.cpio.gz"
}

linux_preflight() { # prints a reason and returns 1 when the Linux arms cannot run here
    [ -r "$KERNEL" ] || { echo "kernel $KERNEL not readable"; return 1; }
    [ -n "$BUSYBOX" ] && file -L "$BUSYBOX" | grep -q 'statically linked' || { echo "no static busybox"; return 1; }
    command -v cpio >/dev/null || { echo "no cpio"; return 1; }
    # The arm asserts simpledrm on the firmware framebuffer; a kernel without it built in cannot
    # pass, and that says nothing about the GOP.
    if [ -r "$KERNEL_CONFIG" ]; then
        grep -q '^CONFIG_DRM_SIMPLEDRM=y' "$KERNEL_CONFIG" && grep -q '^CONFIG_SYSFB_SIMPLEFB=y' "$KERNEL_CONFIG" \
            || { echo "kernel $(basename "$KERNEL") lacks built-in simpledrm/sysfb ($KERNEL_CONFIG)"; return 1; }
    fi
}

# linux_boot <arm> [qemu device args]: boot the kernel, leave its report in $WORK/<arm>/serial.log.
linux_boot() {
    local arm=$1 d="$WORK/$1"; shift
    build_initramfs
    boot "$d" "$OVMF_DIR/OVMF_CODE_4M.fd" "$OVMF_DIR/OVMF_VARS_4M.fd" 'KFLINUX end' serial.log \
        "$@" -kernel "$KERNEL" -initrd "$WORK/initramfs.cpio.gz" -append "console=ttyS0 loglevel=7 panic=-1 fbcon=nodefer"
}

arm_linux() {
    local why; why=$(linux_preflight) || { verdict linux SKIP "$why"; return; }
    local d="$WORK/linux"
    if ! linux_boot linux -device ati-vga,romfile="$WORK/rel.rom"; then
        verdict linux FAIL "boot: $(cat "$d/why")"; stop; return
    fi
    sleep 1
    local pci bar0 bar_lo
    pci=$(grep -a 'KFLINUX pci .*vendor=0x1002' "$d/serial.log" | head -1)
    bar0=$(sed -n 's/.*bar0=\(0x[0-9a-f]*-0x[0-9a-f]*\).*/\1/p' <<<"$pci")
    bar_lo=${bar0%-*}
    [ -n "$bar_lo" ] && pmemsave "$SOCK" "$bar_lo" $((4608 * SI_H)) "$d/bar.bin"
    stop
    python3 - "$d/serial.log" "$d/bar.bin" "$bar0" $((4608 * SI_H)) <<'EOF' | tee "$d/facts"
import os, re, sys
log = open(sys.argv[1], errors="replace").read().splitlines()
bar = sys.argv[3]
facts, ok = [], True
def need(cond, fact):
    global ok
    facts.append(fact)
    ok = ok and bool(cond)
pci = [l for l in log if l.startswith("KFLINUX pci") and "vendor=0x1002" in l]
need(pci and "boot_vga=1" in pci[0], "boot_vga=" + (re.search(r"boot_vga=(\S+)", pci[0]).group(1) if pci else "?"))
lo, hi = (int(x, 16) for x in bar.split("-")) if bar else (0, 0)
bootfb = [l for l in log if l.startswith("KFLINUX iomem") and "BOOTFB" in l]
if bootfb:
    a, b = (int(x, 16) for x in bootfb[0].split()[2].split("-"))
    need(a == lo and b <= hi, f"bootfb={a:#x}-{b:#x}")
    need(b - a + 1 == int(sys.argv[4]), f"bootfb_bytes={b - a + 1}")
else:
    need(False, "bootfb=absent")
sysfb = [l for l in log if l.startswith("KFLINUX sysfb")]
need(sysfb and "/0000:00:" in sysfb[0] and "driver=simple-framebuffer" in sysfb[0],
     "sysfb=" + (sysfb[0].split()[2].split("/")[-2] + " " + sysfb[0].split()[3] if sysfb else "absent"))
fb = {l.split()[2].split("=")[0]: l.split("=", 1)[1] for l in log if l.startswith("KFLINUX fb0") and "=" in l}
need(fb.get("virtual_size") == "1152,648" and fb.get("stride") == "4608",
     f"fb0={fb.get('name', '?').replace(' ', '_')}:{fb.get('virtual_size')}:stride={fb.get('stride')}")
drm = [l for l in log if "simpledrm" in l and "Initialized" in l]
need(drm, "simpledrm=" + ("initialized" if drm else "absent"))
need(any(l.startswith("KFLINUX fb0 drawn") for l in log), "fb0_write=" + ("ok" if any(l.startswith("KFLINUX fb0 drawn") for l in log) else "failed"))
bad = h = 648
if os.path.exists(sys.argv[2]):
    fbb = open(sys.argv[2], "rb").read()
    def px(x, y):  # kf_gop::test_pattern
        return bytes([(x * 3 + y * 5) & 0xFF, (x ^ y) & 0xFF, (x + y * 7) & 0xFF, 0])
    bad = sum(1 for y in range(h) if fbb[y * 4608 : y * 4608 + 1152 * 4] != b"".join(px(x, y) for x in range(1152)))
need(bad == 0, f"bar_pattern_rows_bad={bad}/{h}")
print(("PASS " if ok else "FAIL ") + " ".join(facts))
EOF
    local res; res=$(cat "$d/facts")
    grep -a 'KFLINUX dmesg' "$d/serial.log" | grep -iE 'simple-framebuffer|simpledrm|vgaarb|fb0' | head -6 | sed 's/^/GOP_STANDIN_DETAIL linux /'
    verdict linux "${res%% *}" "${res#* } kernel=$(grep -ao 'KFLINUX begin [^ ]*' "$d/serial.log" | cut -d' ' -f3)"
}

arm_linux_two_vga() {
    local why; why=$(linux_preflight) || { verdict linux_two_vga SKIP "$why"; return; }
    local d="$WORK/linux_two_vga"
    # 02.0: a VGA-class device with no GOP and no ROM; 03.0: the stand-in with kf-gop.
    if ! linux_boot linux_two_vga -device ati-vga,addr=02.0,romfile= -device ati-vga,addr=03.0,romfile="$WORK/rel.rom"; then
        verdict linux_two_vga FAIL "boot: $(cat "$d/why")"; stop; return
    fi
    stop
    local res
    res=$(python3 - "$d/serial.log" <<'EOF'
import re, sys
log = open(sys.argv[1], errors="replace").read().splitlines()
devs = {}
for l in log:
    if l.startswith("KFLINUX pci"):
        f = dict(kv.split("=", 1) for kv in l.split()[3:])
        devs[l.split()[2]] = f
boot = [k for k, f in devs.items() if f.get("boot_vga") == "1"]
bootfb = [l for l in log if l.startswith("KFLINUX iomem") and "BOOTFB" in l]
owner = None
if bootfb:
    a = int(bootfb[0].split()[2].split("-")[0], 16)
    for k, f in devs.items():
        lo, hi = (int(x, 16) for x in f["bar0"].split("-"))
        if lo <= a <= hi:
            owner = k
decode = " ".join(f"{k[-4:]}:cmd={f['command']}" for k, f in sorted(devs.items()))
legacy_other = any(k != owner and (int(f["command"], 16) & 3) == 3 for k, f in devs.items())
ok = len(boot) == 1 and boot[0] == owner
print(("PASS" if ok else "FAIL") + f" boot_vga={','.join(boot) or 'none'} bootfb_owner={owner} {decode} other_has_io_and_mem={'yes' if legacy_other else 'no'}")
EOF
)
    verdict linux_two_vga "${res%% *}" "${res#* }"
}

sb_preflight() {
    command -v sbsign >/dev/null || { echo "no sbsign"; return 1; }
    command -v openssl >/dev/null || { echo "no openssl"; return 1; }
    [ -r "$OVMF_KEYS/PkKek-1-snakeoil.key" ] && [ -r "$OVMF_KEYS/PkKek-1-snakeoil.pem" ] || { echo "no snakeoil keys in $OVMF_KEYS"; return 1; }
    [ -f "$OVMF_DIR/OVMF_CODE_4M.secboot.fd" ] && [ -f "$OVMF_DIR/OVMF_VARS_4M.ms.fd" ] && [ -f "$OVMF_DIR/OVMF_VARS_4M.snakeoil.fd" ] || { echo "no Secure Boot OVMF variants in $OVMF_DIR"; return 1; }
}
sb_prepare() {
    [ -f "$WORK/sb/signed.rom" ] && return 0
    mkdir -p "$WORK/sb/esp/EFI/BOOT"
    # Debian's snakeoil key is encrypted with the passphrase "snakeoil"; sbsign wants it in clear.
    openssl pkey -in "$OVMF_KEYS/PkKek-1-snakeoil.key" -passin "pass:${SNAKEOIL_PASS:-snakeoil}" \
        -out "$WORK/sb/snakeoil.key" || { echo "GOP_STANDIN_FATAL cannot decrypt the snakeoil key"; return 1; }
    sbsign --key "$WORK/sb/snakeoil.key" --cert "$OVMF_KEYS/PkKek-1-snakeoil.pem" \
        --output "$WORK/sb/kf-gop-signed.efi" "$EFI_DBG" || return 1
    sbsign --key "$WORK/sb/snakeoil.key" --cert "$OVMF_KEYS/PkKek-1-snakeoil.pem" \
        --output "$WORK/sb/esp/EFI/BOOT/BOOTX64.EFI" "$EFI_APP" || return 1
    pack "$WORK/sb/kf-gop-signed.efi" "$WORK/sb/signed.rom" "$SI_VENDOR" "$SI_DEVICE"
    "$KFO" pe "$WORK/sb/kf-gop-signed.efi" | sed 's/^/GOP_STANDIN_PE snakeoil-signed /'
    # The layout kf-oprom does NOT emit: the signed PE at 0x200 and zero padding after it.
    python3 - "$WORK/sb/signed.rom" "$WORK/sb/kf-gop-signed.efi" "$WORK/sb/tailpad.rom" <<'EOF'
import struct, sys
rom, pe = open(sys.argv[1], "rb").read(), open(sys.argv[2], "rb").read()
at = 0x200
total = -(-(at + len(pe)) // 512) * 512
out = bytearray(rom[:at].ljust(at, b"\0")) + pe + bytes(total - at - len(pe))
blocks = total // 512
struct.pack_into("<H", out, 0x02, blocks)              # InitializationSize
struct.pack_into("<H", out, 0x16, at)                  # EfiImageHeaderOffset
struct.pack_into("<H", out, 0x1C + 0x10, blocks)       # PCIR ImageLength
open(sys.argv[3], "wb").write(out)
print(f"GOP_STANDIN_ROM tailpad pe_at={at:#x} pe_bytes={len(pe)} padding_after={total - at - len(pe)}")
EOF
}
# sb_arm <arm> <vars> <rom> <esp> [code]: boot until the test app reports or the boot manager refuses
# it, then print the facts. The unsigned app is refused on the serial console, so both logs are
# watched. `code` defaults to the distribution's Secure Boot OVMF.
sb_arm() {
    local arm=$1 vars=$2 rom=$3 esp=$4 code=${5:-$OVMF_DIR/OVMF_CODE_4M.secboot.fd} d="$WORK/$1"
    esp_drive "$esp"
    boot "$d" "$code" "$vars" 'KFGOP-TEST (READY|RESULT FAIL)|Access Denied|Security Violation' \
        dbg.log,serial.log -device ati-vga,romfile="$rom" "${ESP[@]}" || echo "boot: $(cat "$d/why")" > "$d/note"
    sleep 2
    stop
    local loaded started result sb app
    loaded=$(grep -ac 'kf-gop: loaded' "$d/dbg.log")
    started=$(grep -ac 'kf-gop: started' "$d/dbg.log")
    result=$(grep -ao 'KFGOP-TEST RESULT [A-Z]* checks=[0-9]* failed=[0-9]*' "$d/dbg.log" | head -1)
    sb=$(grep -ao 'secure_boot=[^ ]*' "$d/dbg.log" | head -1)
    app=$(tr -d '\0' < "$d/serial.log" 2>/dev/null | grep -aoE 'Access Denied|Security Violation' | head -1)
    echo "rom_loaded=$loaded rom_started=$started ${sb:-secure_boot=not-reported} app_result=\"${result:-none}\" app_denied=\"${app:-no}\" $(cat "$d/note" 2>/dev/null)"
}
arm_sb_ms_unsigned_observe() {
    local arm=sb_ms_unsigned_observe
    local why; why=$(sb_preflight) || { verdict "$arm" SKIP "$why"; return; }
    local f; f=$(sb_arm "$arm" "$OVMF_DIR/OVMF_VARS_4M.ms.fd" "$WORK/dbg.rom" "$WORK/esp")
    # ASSERTED: the unsigned test app is refused (Secure Boot is enforcing). OBSERVED: what happens
    # to the unsigned ROM — stock OVMF trusts option ROMs, so there is no outcome to require here.
    if grep -q 'app_denied="Access Denied"\|app_denied="Security Violation"' <<<"$f"; then
        local obs=denied; grep -q 'rom_loaded=[1-9]' <<<"$f" && obs=loaded
        verdict "$arm" OBSERVED "observed=unsigned_rom_$obs $f"
    else
        verdict "$arm" FAIL "could not confirm enforcement: $f"
    fi
}
sb_snakeoil() { # sb_snakeoil <arm> <rom> <must_load: yes|observe>
    local arm=$1 rom=$2 must=$3
    local why; why=$(sb_preflight) || { verdict "$arm" SKIP "$why"; return; }
    sb_prepare || { verdict "$arm" FAIL "signing failed"; return; }
    local f; f=$(sb_arm "$arm" "$OVMF_DIR/OVMF_VARS_4M.snakeoil.fd" "$rom" "$WORK/sb/esp")
    if ! grep -q 'secure_boot=1' <<<"$f"; then verdict "$arm" FAIL "Secure Boot not confirmed on: $f"; return; fi
    local obs=denied; grep -q 'rom_started=[1-9]' <<<"$f" && obs=loaded
    if [ "$must" = yes ]; then
        if [ "$obs" = loaded ] && grep -q 'app_result="KFGOP-TEST RESULT PASS' <<<"$f"; then
            verdict "$arm" PASS "observed=signed_rom_loaded $f"
        else
            verdict "$arm" FAIL "$f"
        fi
    else
        verdict "$arm" OBSERVED "observed=$obs $f"
    fi
}
arm_sb_snakeoil_signed() { sb_snakeoil sb_snakeoil_signed "$WORK/sb/signed.rom" yes; }
arm_sb_snakeoil_unsigned_observe() { sb_snakeoil sb_snakeoil_unsigned_observe "$WORK/dbg.rom" observe; }
arm_sb_snakeoil_tailpad_observe() { sb_snakeoil sb_snakeoil_tailpad_observe "$WORK/sb/tailpad.rom" observe; }

# ★ The arms that CAN fail on the ROM outcome: a firmware that verifies option ROMs
# (PcdOptionRomImageVerificationPolicy = 0x04, DENY_EXECUTE_ON_SECURITY_VIOLATION). Stock OVMF sets it
# only under AMD SEV, so these need a build named by OVMF_DENY_CODE / OVMF_DENY_VARS.
sb_deny() { # sb_deny <arm> <rom> <expect: load|deny>
    local arm=$1 rom=$2 expect=$3
    if [ -z "${OVMF_DENY_CODE:-}" ] || [ -z "${OVMF_DENY_VARS:-}" ]; then
        verdict "$arm" SKIP "no OVMF that verifies option ROMs: set OVMF_DENY_CODE and OVMF_DENY_VARS to a Secure Boot OVMF built with PcdOptionRomImageVerificationPolicy=0x04 and its snakeoil-enrolled VARS"
        return
    fi
    local why; why=$(sb_preflight) || { verdict "$arm" SKIP "$why"; return; }
    sb_prepare || { verdict "$arm" FAIL "signing failed"; return; }
    local f; f=$(sb_arm "$arm" "$OVMF_DENY_VARS" "$rom" "$WORK/sb/esp" "$OVMF_DENY_CODE")
    if ! grep -q 'secure_boot=1' <<<"$f"; then verdict "$arm" FAIL "Secure Boot not confirmed on: $f"; return; fi
    local started=0; grep -q 'rom_started=[1-9]' <<<"$f" && started=1
    case "$expect" in
        load)
            if [ "$started" -eq 1 ] && grep -q 'app_result="KFGOP-TEST RESULT PASS' <<<"$f"; then
                verdict "$arm" PASS "observed=rom_verified_and_started $f"
            else
                verdict "$arm" FAIL "the signed ROM did not start under a deny policy: $f"
            fi ;;
        deny)
            if [ "$started" -eq 0 ]; then
                verdict "$arm" PASS "observed=rom_denied $f"
            else
                verdict "$arm" FAIL "the ROM started under a deny policy: $f"
            fi ;;
    esac
}
arm_sb_deny_signed() { sb_deny sb_deny_signed "$WORK/sb/signed.rom" load; }
arm_sb_deny_unsigned() { sb_deny sb_deny_unsigned "$WORK/dbg.rom" deny; }
arm_sb_deny_tailpad() { sb_deny sb_deny_tailpad "$WORK/sb/tailpad.rom" deny; }

for arm in $ARMS; do
    if declare -F "arm_$arm" >/dev/null; then "arm_$arm"; else verdict "$arm" FAIL "unknown arm"; fi
done

fails=0 passes=0 observed=0 skipped=0
for r in "${RESULTS[@]}"; do
    case "$r" in
        *=FAIL) fails=$((fails + 1)) ;;
        *=PASS) passes=$((passes + 1)) ;;
        *=OBSERVED) observed=$((observed + 1)) ;;
        *=SKIP) skipped=$((skipped + 1)) ;;
    esac
done
echo "GOP_STANDIN_SUMMARY arms=${#RESULTS[@]} passed=$passes observed=$observed skipped=$skipped failed=$fails ${RESULTS[*]} $(date -Is)"
[ "$KEEP" -eq 1 ] && echo "GOP_STANDIN_KEPT $WORK"
[ "$fails" -eq 0 ]
