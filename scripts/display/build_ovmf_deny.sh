#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
#
# build_ovmf_deny.sh — an OVMF that VERIFIES option ROMs, for gop_standin.sh's sb_deny_* arms
# (test F1, docs/design/V3_DISPLAY.md §4.11.9). Local, no GPU.
#
#   usage: scripts/display/build_ovmf_deny.sh <edk2-src> <out-dir>
#     <edk2-src>  an EDK2 tree WITH its submodules — QEMU 10.2.4's roms/edk2 (edk2-stable202408).
#                 It is copied to a scratch directory and never modified.
#     <out-dir>   receives OVMF_CODE_4M.deny.fd; the script prints the gop_standin.sh line.
#
# Why it is needed: stock OVMF trusts every option ROM — PcdOptionRomImageVerificationPolicy is 0x00
# in OvmfPkg/OvmfPkgX64.dsc's [PcdsDynamicDefault], set to 0x04 only under AMD SEV
# (OvmfPkg/PlatformPei/AmdSev.c) — so the stand-in's Secure Boot arms can only OBSERVE what happens
# to a ROM. This build sets the default to 0x04 (DENY_EXECUTE_ON_SECURITY_VIOLATION), the policy of
# firmware that checks ROM signatures, so an arm can REQUIRE an outcome: kf-oprom's end-aligned
# signed ROM must start; an unsigned or tail-padded one must not.
#
# Shape: OvmfPkgX64.dsc, RELEASE, GCC5, SECURE_BOOT_ENABLE + SMM_REQUIRE + FD_SIZE_4MB — Ubuntu's
# Secure Boot build, so Ubuntu's OVMF_VARS_4M.snakeoil.fd (snakeoil keys enrolled) serves as its VARS
# and gop_standin.sh boots it with SMM and the secure flash.
#
# Needs: gcc, make, python3, nasm, iasl (acpica-tools), uuid-dev. Two workarounds for a 2026 host
# toolchain building edk2-stable202408, both applied to the scratch copy only:
#   - gcc 15 defaults to C23, under which BaseTools' Pccts (`()` prototypes) and EDK2's C do not
#     build: both are built with -std=gnu17.
#   - NASM 3 refuses `push strict dword imm` in 64-bit code
#     (UefiCpuPkg/Library/CpuExceptionHandlerLib/X64/ExceptionHandlerAsm.nasm); `push strict qword`
#     assembles to the same bytes (68 imm32), which the vector-patching code depends on.
set -euo pipefail
die() { echo "build_ovmf_deny: $*" >&2; exit 1; }
SRC=${1:?usage: build_ovmf_deny.sh <edk2-src> <out-dir>}
OUT=${2:?usage: build_ovmf_deny.sh <edk2-src> <out-dir>}
J=${JOBS:-$(nproc)}
for t in gcc make python3 nasm iasl; do command -v "$t" >/dev/null || die "missing $t"; done
[ -f "$SRC/OvmfPkg/OvmfPkgX64.dsc" ] || die "$SRC is not an EDK2 tree"
[ -n "$(ls -A "$SRC/CryptoPkg/Library/OpensslLib/openssl" 2>/dev/null)" ] || die "$SRC lacks its submodules (openssl)"
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/ovmf-deny.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
cp -a "$SRC" "$WORK/edk2"
cd "$WORK/edk2"

dsc=OvmfPkg/OvmfPkgX64.dsc
grep -q 'PcdOptionRomImageVerificationPolicy|0x00' "$dsc" || die "$dsc: no PcdOptionRomImageVerificationPolicy|0x00 row to change"
sed -i 's/PcdOptionRomImageVerificationPolicy|0x00/PcdOptionRomImageVerificationPolicy|0x04/' "$dsc"
grep -q 'PcdOptionRomImageVerificationPolicy|0x04' "$dsc" || die "the policy edit did not take"
sed -i 's/push    strict dword/push    strict qword/' UefiCpuPkg/Library/CpuExceptionHandlerLib/X64/ExceptionHandlerAsm.nasm

export PYTHON_COMMAND=python3
# edksetup.sh parses the positional parameters of whoever sources it, so source it with none.
set --
set +u
# shellcheck disable=SC1091  # EDK2's own environment script, in the scratch copy
. ./edksetup.sh > "$OUT/edksetup.log" 2>&1 || die "edksetup.sh failed (see $OUT/edksetup.log)"
set -u
make -C BaseTools -j"$J" CC="gcc -std=gnu17" BUILD_CC="gcc -std=gnu17" > "$OUT/basetools.log" 2>&1 \
    || die "BaseTools failed (see $OUT/basetools.log)"
sed -i 's/^\(DEFINE GCC_ALL_CC_FLAGS *= \)/\1-std=gnu17 /' Conf/tools_def.txt
build -p "$dsc" -a X64 -t GCC5 -b RELEASE -n "$J" \
    -D SECURE_BOOT_ENABLE=TRUE -D SMM_REQUIRE=TRUE -D FD_SIZE_4MB -D BUILD_SHELL=FALSE \
    > "$OUT/build.log" 2>&1 || die "the OVMF build failed (see $OUT/build.log)"
cp Build/OvmfX64/RELEASE_GCC5/FV/OVMF_CODE.fd "$OUT/OVMF_CODE_4M.deny.fd"
tag=$(sed -n 's/^EDK2_STABLE *= *//p' "$SRC/../edk2-version" 2>/dev/null || true)
echo "OVMF_DENY_BUILT edk2=${tag:-$SRC} code=$OUT/OVMF_CODE_4M.deny.fd sha256=$(sha256sum "$OUT/OVMF_CODE_4M.deny.fd" | cut -c1-64)"
echo "run: OVMF_DENY_CODE=$OUT/OVMF_CODE_4M.deny.fd OVMF_DENY_VARS=/usr/share/OVMF/OVMF_VARS_4M.snakeoil.fd bash scripts/display/gop_standin.sh sb_deny_signed sb_deny_unsigned sb_deny_tailpad"
