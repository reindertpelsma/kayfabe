#!/usr/bin/env bash
# ★ STAGE AN OLDER GUEST KERNEL for the thin guest — for guest driver versions that do not build
# on the host's own kernel (`docs/design/V3_DRIVER_MATRIX.md` §5).
#
#   usage: stage_guest_kernel.sh <kernel-release> [<stage-root>]     e.g. 6.5.0-45-generic
#   result: <stage-root>/kernels/<krel>/{boot/vmlinuz-<krel>, lib/modules/<krel>/..., usr/src/...}
#           <stage-root>/kernels/<krel>/STAGED   (last; only when every check passed)
#
# ## Why
#
# The thin guest's host mode boots the HOST's kernel, and every staged guest driver is built for
# it. `[measured 2026-09-26]` 545.23.08 and 545.29.06 do not build on Linux 6.8
# (`kernel-open/nvidia/libspdm_shash.c:90`: `crypto_tfm_ctx_aligned`, whose conftest arrived only
# in 550), and both of our guest paths run 6.8 (thin: the host's 6.8.0-59; fat: the image's
# 6.8.0-139). So a 545 guest needs a guest kernel the 545 tag supports — Ubuntu 22.04's 6.5 HWE.
#
# ## How
#
# ⊘ Never installed on the box: a second kernel installed through apt would move the box's own
# boot default (a vast VM reboots into what grub picked). The debs are EXTRACTED into the stage
# root instead — the image, its modules (depmod'ed there) and its build headers — and the build
# symlink is pointed at the extracted headers. `stage_guest_driver.sh` then builds against it
# (`KREL=<krel> KBUILD=<root>/lib/modules/<krel>/build`) and `build_fast_guest.sh` boots it
# (`KF_GUEST_KROOT=<root>`).
set -uo pipefail
KREL=${1:?usage: stage_guest_kernel.sh <kernel-release> [<stage-root>]}
ROOT=${2:-/workspace/drivers}/kernels/$KREL
say() { echo "[$(date -Is)] kernel $KREL: $*"; }
die() { say "⊘ $*"; echo "KERNEL_STAGE_FAILED $KREL: $*" >&2; exit 1; }
ABI=${KREL%-generic}                     # 6.5.0-45
SERIES=$(echo "$KREL" | cut -d. -f1-2)   # 6.5
mkdir -p "$ROOT" && rm -f "$ROOT/STAGED"
D=$(mktemp -d "$ROOT/../debs-$KREL.XXXX")
cd "$D" || die "no temp dir"
apt-get update -qq >/dev/null 2>&1
PKGS="linux-image-unsigned-$KREL linux-modules-$KREL linux-headers-$KREL linux-hwe-$SERIES-headers-$ABI"
say "download $PKGS"
apt-get download $PKGS > apt.log 2>&1 || { tail -5 apt.log; die "apt-get download failed (is $KREL a jammy HWE kernel?)"; }
for d in *.deb; do dpkg-deb -x "$d" "$ROOT" || die "extract $d"; done
cd / && rm -rf "$D"
[ -f "$ROOT/boot/vmlinuz-$KREL" ] || die "no boot/vmlinuz-$KREL in the packages"
[ -d "$ROOT/usr/src/linux-headers-$KREL" ] || die "no usr/src/linux-headers-$KREL in the packages"
ln -sfn "$ROOT/usr/src/linux-headers-$KREL" "$ROOT/lib/modules/$KREL/build"
depmod -b "$ROOT" "$KREL" || die "depmod over the extracted tree failed"
[ -s "$ROOT/lib/modules/$KREL/modules.dep" ] || die "no modules.dep after depmod"
[ -f "$ROOT/lib/modules/$KREL/build/.config" ] || die "the extracted headers carry no .config"
KCC=$(sed -n 's/^CONFIG_CC_VERSION_TEXT="\([^ ]*\) .*/\1/p' "$ROOT/lib/modules/$KREL/build/.config")
command -v "$KCC" >/dev/null 2>&1 || die "the kernel was built with $KCC, which is not installed (apt install ${KCC##*-linux-gnu-})"
{ echo "krel=$KREL"; echo "cc=$KCC"; echo "staged=$(date -Is)"; } > "$ROOT/STAGED"
say "STAGED ✔ $ROOT (cc=$KCC)"
