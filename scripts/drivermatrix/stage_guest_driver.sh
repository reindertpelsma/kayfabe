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
    say "make modules against $KREL (-j$JOBS)"
    ( cd "$SRC" && make -s modules -j"$JOBS" SYSSRC="$KBUILD" > "$OUT/build.log" 2>&1 ) \
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
if ! ls "$OUT/firmware/nvidia/$V/"gsp_ga10x.bin >/dev/null 2>&1; then
    if [ ! -s "$RUN" ]; then
        url=https://us.download.nvidia.com/XFree86/Linux-x86_64/$V/NVIDIA-Linux-x86_64-$V.run
        say "download $url"
        curl -fsSL -o "$RUN.part" "$url" \
            || { url=https://us.download.nvidia.com/tesla/$V/NVIDIA-Linux-x86_64-$V.run
                 say "not on XFree86; trying $url"
                 curl -fsSL -o "$RUN.part" "$url"; } \
            || die "no .run for $V on us.download.nvidia.com (XFree86 or tesla)"
        mv "$RUN.part" "$RUN"
    fi
    x=$(mktemp -d "$ROOT/src/x-$V.XXXX")
    sh "$RUN" -x --target "$x/pkg" > /dev/null 2>&1 || { rm -rf "$x"; die "could not extract $RUN"; }
    cp "$x/pkg/firmware/"gsp_*.bin "$OUT/firmware/nvidia/$V/" 2>/dev/null
    rm -rf "$x"
fi
ls "$OUT/firmware/nvidia/$V/"gsp_ga10x.bin >/dev/null 2>&1 || die "no gsp_ga10x.bin in the $V package"
say "firmware ✔ $(ls "$OUT/firmware/nvidia/$V/" | tr '\n' ' ')"

# ── 3. provenance, then the marker ───────────────────────────────────────────────────────
( cd "$OUT" && sha256sum modules/*.ko firmware/nvidia/"$V"/*.bin > SHA256SUMS )
{ echo "version=$V"; echo "kernel=$KREL"; echo "ogkm_tag=$V"; echo "staged=$(date -Is)"; } > "$OUT/STAGED"
say "STAGED ✔ $OUT"
