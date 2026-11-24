#!/bin/bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Build two QEMU 10.2.4 binaries from the pinned tarball for arms B+P and B0 of PREREGISTRATION.md: one with the
# optional hyperv-wake patch, one without. No kf3 source. Usage: build_qemu_pair.sh TARBALL PATCH OUTDIR
set -euo pipefail
tarball=$1 patch=$2 out=$3
want=821b545b92f165e57dddccac5077d76d4d436a226595b8813ad59306bbfd0746   # scripts/bench/qemu.pin
echo "$want  $tarball" | sha256sum -c -
mkdir -p "$out"
for v in stock patched; do
  rm -rf "$out/src-$v" "$out/build-$v"
  mkdir -p "$out/src-$v" "$out/build-$v"
  tar -xJf "$tarball" -C "$out/src-$v" --strip-components=1
  if [ "$v" = patched ]; then patch -p1 --forward -d "$out/src-$v" < "$patch"; fi
  (cd "$out/build-$v" && "$out/src-$v/configure" --target-list=x86_64-softmmu --disable-docs --disable-tools \
    --disable-guest-agent --disable-werror --enable-slirp --enable-vnc --enable-pixman --enable-zstd --disable-gtk \
    --disable-sdl --disable-curses --disable-libssh --disable-vde --enable-tpm --without-default-features --enable-kvm \
    --enable-system > configure.log 2>&1)
  nice -n 10 ninja -C "$out/build-$v" -j 16 qemu-system-x86_64 > "$out/build-$v/ninja.log" 2>&1
  sha256sum "$out/build-$v/qemu-system-x86_64"
done
grep -c "msr_hv_hypercall" "$out/src-patched/hw/i386/pc.c" "$out/src-stock/hw/i386/pc.c" || true
echo BUILD_PAIR_DONE
