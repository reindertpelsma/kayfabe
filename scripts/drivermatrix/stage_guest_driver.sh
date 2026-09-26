#!/usr/bin/env bash
# ★ STAGE ONE GUEST DRIVER VERSION for the thin guest — the kernel modules built from THAT
# version's open source, and THAT version's GSP firmware — without touching the host driver.
#
#   usage: stage_guest_driver.sh <version> [<stage-root>]     e.g. 580.105.08
#   result: <stage-root>/<version>/modules/{nvidia,nvidia-uvm,nvidia-modeset}.ko
#           <stage-root>/<version>/firmware/nvidia/<version>/gsp_*.bin
#           <stage-root>/<version>/NVIDIA-Linux-x86_64-<version>.run   (kept: the fat guest's)
#           <stage-root>/<version>/STAGED   (last line; written only when every check passed)
#
# ## Why the guest axis needs this (`docs/design/V3_DRIVER_MATRIX.md` §5)
#
# The thin guest's HOST MODE (`build_fast_guest.sh`, `KF_FROM_HOST=1`) borrows the kernel AND
# the nvidia modules from the host, so the guest driver it boots IS the host driver. That pins
# the two axes together — exactly the coupling `THE_ARCHITECTURE_v3.md` §0 forbids. Walking the
# guest axis with the host held fixed needs the modules of ANOTHER version built for the host's
# own kernel (so the thin guest can still boot the host's `vmlinuz`), and that version's GSP
# firmware (the guest driver refuses a firmware whose `.fwversion` is not its own).
#
# ## Where each artefact comes from, and why
#
# - modules: `NVIDIA/open-gpu-kernel-modules` at the tag, `make modules` against the running
#   kernel's build tree. ⊘ Never the `.run`'s precompiled interface: the open module is the
#   product's guest (`multi_driver_support`: the open driver is canonical) and building it from
#   the tag is the one way the modules and the ogkm tag the derivation measured are provably the
#   same source.
# - firmware: the `.run` package of the same version (`firmware/gsp_*.bin`). The open-source
#   tree does not ship firmware.
#
# ⚠ Verified on CONTENT (the lesson of `provision_host_driver.sh`): `modinfo -F version` of the
# built `nvidia.ko` must equal <version>, and the firmware directory must hold `gsp_ga10x.bin`.
# A stage that fails a check exits nonzero and never writes STAGED.
set -uo pipefail
V=${1:?usage: stage_guest_driver.sh <version> [<stage-root>]}
ROOT=${2:-/workspace/drivers}
KREL=${KREL:-$(uname -r)}
KBUILD=/lib/modules/$KREL/build
OUT=$ROOT/$V
SRC=$ROOT/src/ogkm-$V
RUN=$OUT/NVIDIA-Linux-x86_64-$V.run
JOBS=${JOBS:-$(nproc)}
say() { echo "[$(date -Is)] stage $V: $*"; }
die() { say "⊘ $*"; echo "STAGE_FAILED $V: $*" >&2; exit 1; }

[ -d "$KBUILD" ] || die "no kernel build tree at $KBUILD (install linux-headers-$KREL)"
mkdir -p "$OUT/modules" "$OUT/firmware/nvidia/$V" "$ROOT/src"
rm -f "$OUT/STAGED"

# ── 1. the modules, from the tag ──────────────────────────────────────────────────────────
if [ ! -f "$OUT/modules/nvidia.ko" ] || [ "$(modinfo -F version "$OUT/modules/nvidia.ko" 2>/dev/null)" != "$V" ]; then
    if [ ! -d "$SRC/.git" ]; then
        say "clone ogkm $V"
        git clone -q --depth 1 --branch "$V" https://github.com/NVIDIA/open-gpu-kernel-modules.git "$SRC" \
            || die "clone of tag $V failed"
    fi
    # ⊘ THE KERNEL'S OWN COMPILER, not the default `cc`. `[measured 2026-09-26]` Ubuntu 22.04's
    # HWE 6.8 kernel is built with gcc-12 while `cc` is gcc-11, and the kernel's flags include
    # `-ftrivial-auto-var-init=zero`, which gcc-11 rejects — ogkm 550.54.14 and 565.57.01 failed
    # every object on it, while 570+ (which pick the kernel's compiler themselves) built. The
    # compiler is read from the kernel's own `CONFIG_CC_VERSION_TEXT`, never guessed.
    KCC=$(sed -n 's/^CONFIG_CC_VERSION_TEXT="\([^ ]*\) .*/\1/p' "$KBUILD/.config" 2>/dev/null)
    command -v "$KCC" >/dev/null 2>&1 || KCC=cc
    say "make modules against $KREL (-j$JOBS, CC=$KCC)"
    ( cd "$SRC" && make -s modules -j"$JOBS" SYSSRC="$KBUILD" CC="$KCC" > "$OUT/build.log" 2>&1 ) \
        || { tail -25 "$OUT/build.log"; die "ogkm $V did not build against $KREL (log: $OUT/build.log)"; }
    for m in nvidia nvidia-uvm nvidia-modeset; do
        cp "$SRC/kernel-open/$m.ko" "$OUT/modules/$m.ko" || die "no $m.ko after the build"
    done
fi
got=$(modinfo -F version "$OUT/modules/nvidia.ko" 2>/dev/null)
[ "$got" = "$V" ] || die "built nvidia.ko says version '$got', not $V"
vm=$(modinfo -F vermagic "$OUT/modules/nvidia.ko" 2>/dev/null | awk '{print $1}')
[ "$vm" = "$KREL" ] || die "built nvidia.ko vermagic '$vm' is not the kernel it must load into ($KREL)"
say "modules ✔ version=$got vermagic=$vm"

# ── 2. the firmware, from the same version's .run ────────────────────────────────────────
# ⚠ Not every version has a .run on the public download paths (measured 2026-09-26: 545.23.08
# and 575.51.03 are 404 on both XFree86/ and tesla/). NVIDIA's CUDA apt repository carries them
# as debs, so that is the fallback — the firmware package is `nvidia-firmware-<branch>-<v>` or
# `nvidia-firmware-<branch>_<v>` on newer branches and `nvidia-kernel-common-<branch>_<v>` on
# older ones. Same bytes the driver's own packaging installs; the version is checked below.
CUDA_REPO=https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2204/x86_64
fetch_deb_firmware() {
    local br=${V%%.*} idx deb x
    idx=$(curl -fsSL "$CUDA_REPO/") || return 1
    for pat in "nvidia-firmware-$br-${V}_${V}-0ubuntu1_amd64.deb" "nvidia-firmware-${br}_${V}-0ubuntu1_amd64.deb" \
               "nvidia-kernel-common-${br}_${V}-0ubuntu1_amd64.deb"; do
        grep -q "$pat" <<<"$idx" || continue
        say "firmware from the CUDA repo deb $pat"
        deb=$ROOT/src/$pat
        curl -fsSL -o "$deb" "$CUDA_REPO/$pat" || continue
        x=$(mktemp -d "$ROOT/src/d-$V.XXXX")
        dpkg-deb -x "$deb" "$x" && cp "$x/lib/firmware/nvidia/$V/"gsp_*.bin "$OUT/firmware/nvidia/$V/" 2>/dev/null
        rm -rf "$x" "$deb"
        ls "$OUT/firmware/nvidia/$V/"gsp_ga10x.bin >/dev/null 2>&1 && return 0
    done
    return 1
}
if ! ls "$OUT/firmware/nvidia/$V/"gsp_ga10x.bin >/dev/null 2>&1; then
    if [ ! -s "$RUN" ]; then
        for url in https://us.download.nvidia.com/XFree86/Linux-x86_64/$V/NVIDIA-Linux-x86_64-$V.run \
                   https://us.download.nvidia.com/tesla/$V/NVIDIA-Linux-x86_64-$V.run; do
            say "download $url"
            curl -fsSL -o "$RUN.part" "$url" && { mv "$RUN.part" "$RUN"; break; }
        done
        rm -f "$RUN.part"
    fi
    if [ -s "$RUN" ]; then
        x=$(mktemp -d "$ROOT/src/x-$V.XXXX")
        sh "$RUN" -x --target "$x/pkg" > /dev/null 2>&1 || { rm -rf "$x"; die "could not extract $RUN"; }
        cp "$x/pkg/firmware/"gsp_*.bin "$OUT/firmware/nvidia/$V/" 2>/dev/null
        rm -rf "$x"
    else
        say "no .run for $V on us.download.nvidia.com (XFree86 or tesla)"
        fetch_deb_firmware || die "no firmware for $V: no .run and no CUDA-repo firmware deb"
    fi
fi
ls "$OUT/firmware/nvidia/$V/"gsp_ga10x.bin >/dev/null 2>&1 || die "no gsp_ga10x.bin in the $V package"
say "firmware ✔ $(ls "$OUT/firmware/nvidia/$V/" | tr '\n' ' ')"

# ── 3. provenance, then the marker ───────────────────────────────────────────────────────
( cd "$OUT" && sha256sum modules/*.ko firmware/nvidia/"$V"/*.bin > SHA256SUMS )
{ echo "version=$V"; echo "kernel=$KREL"; echo "ogkm_tag=$V"; echo "staged=$(date -Is)"; } > "$OUT/STAGED"
say "STAGED ✔ $OUT"
