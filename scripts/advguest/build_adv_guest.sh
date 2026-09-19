#!/usr/bin/env bash
# ★★★★★ THE ADVERSARIAL GUEST — build the hostile module + an initrd that runs it.
#
# A sibling of build_fast_guest.sh, deliberately SEPARATE so the thin-guest
# iteration lane keeps working untouched. This one builds a tiny initrd whose
# /init does exactly one thing: (optionally) load the real nvidia stack so the
# GPU reaches a real post-init state (owner addendum #2 — "the boot sequence can
# be imported or executed"), then `insmod advguest.ko`, which runs the whole
# adversarial suite in its module_init and reports over the serial console.
#
# The module attacks the emulated GPU's MMIO surface DIRECTLY (BAR0/BAR1/BAR2),
# not /dev/nvidia*, so in the default COLD mode it needs no nvidia driver at all.
#
# usage: build_adv_guest.sh [outdir]
#   ADV_POST_INIT=1   load the real nvidia stack first, attack a post-init GPU
#                     (needs KF_FROM_HOST-style modules; see below)
#   KDIR=...          kernel build dir for the module (default: running kernel)
set -uo pipefail

OUT=${1:-/workspace/bench/advguest}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
KREL=$(uname -r)
KDIR=${KDIR:-/lib/modules/$KREL/build}
POST_INIT=${ADV_POST_INIT:-0}

die() { echo "build_adv_guest: $*" >&2; exit 1; }

command -v busybox >/dev/null || die "busybox is not installed"
command -v cpio    >/dev/null || die "cpio is not installed"
[ -d "$KDIR" ] || die "no kernel build dir at $KDIR (install linux-headers-$KREL or set KDIR=)"
[ -f "/boot/vmlinuz-$KREL" ] || die "no /boot/vmlinuz-$KREL — a container may not ship the kernel image"

# ── 1. build the module ────────────────────────────────────────────────────────
echo "== building advguest.ko against $KDIR"
make -C "$HERE" clean >/dev/null 2>&1
make -C "$HERE" KDIR="$KDIR" || die "module build failed"
[ -f "$HERE/advguest.ko" ] || die "advguest.ko was not produced"

# ── 2. lay out the initrd ──────────────────────────────────────────────────────
mkdir -p "$OUT" || die "cannot create $OUT"
ROOT=$(mktemp -d) || die "mktemp"
trap 'rm -rf "$ROOT"' EXIT
mkdir -p "$ROOT/ird"/{bin,dev,proc,sys,tmp,lib/modules}
cp "$(command -v busybox)" "$ROOT/ird/bin/busybox"
cp "$HERE/advguest.ko" "$ROOT/ird/lib/modules/advguest.ko"
cp "/boot/vmlinuz-$KREL" "$OUT/vmlinuz" || die "cannot copy kernel image"

# ── 2b. (optional) the real nvidia stack, so phase 0 attacks a POST-INIT GPU ────
# Taken from the host, by the same rule build_fast_guest.sh uses in KF_FROM_HOST
# mode: the host runs the driver version this tree pins, so its modules match its
# own kernel by construction. If they are absent we still build a COLD initrd and
# say so — a missing driver must not silently disable the post-init lane without a
# word in the log.
if [ "$POST_INIT" = "1" ]; then
    MODROOT="/lib/modules/$KREL"
    DEP="$MODROOT/modules.dep"
    [ -f "$DEP" ] || die "ADV_POST_INIT=1 but no $DEP on the host"
    : > "$ROOT/ird/lib/modules/loadorder"
    found=0
    for ko in nvidia nvidia-uvm nvidia-modeset; do
        line=$(grep -E "(^|/)$ko\.ko(\.[a-z]+)?:" "$DEP" | head -1)
        [ -n "$line" ] || continue
        self=${line%%:*}; deps=${line#*:}
        order=""; for d in $deps; do order="$d $order"; done
        for rel in $order "$self"; do
            src="$MODROOT/$rel"; [ -f "$src" ] || continue
            base=$(basename "$rel"); base=${base%.zst}; base=${base%.xz}; base=${base%.ko}.ko
            dst="$ROOT/ird/lib/modules/$base"; [ -f "$dst" ] && continue
            case "$src" in *.zst) zstd -dq -o "$dst" "$src" ;;
                           *.xz)  xz -dc "$src" > "$dst" ;;
                           *)     cp "$src" "$dst" ;; esac
            echo "$base" >> "$ROOT/ird/lib/modules/loadorder"; found=$((found+1))
        done
    done
    [ "$found" -gt 0 ] || die "ADV_POST_INIT=1 but no nvidia modules under $MODROOT"
    echo "== post-init: $found nvidia module(s) staged (closure)"
    FW="/lib/firmware/nvidia"
    if [ -d "$FW" ]; then
        mkdir -p "$ROOT/ird/lib/firmware"; cp -a "$FW" "$ROOT/ird/lib/firmware/" 2>/dev/null
        find "$ROOT/ird/lib/firmware/nvidia" -name '*.zst' 2>/dev/null | while read -r z; do
            zstd -dq -o "${z%.zst}" "$z" && rm -f "$z"; done
        echo "== post-init: GSP firmware staged"
    else
        echo "== post-init: ⊘ NO firmware at $FW — RmInitAdapter will fail, phase 0 will report COLD"
    fi
fi

# ── 3. /init — load driver (optional), insmod advguest, report, die ─────────────
cat > "$ROOT/ird/init" <<INIT
#!/bin/busybox sh
/bin/busybox --install -s /bin
mount -t proc  none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev 2>/dev/null
echo "ADVGUEST: up \$(cut -d' ' -f1 /proc/uptime)s"

POST=$POST_INIT

# Read arms/params from the kernel command line (ADV_* tokens, whitespace-free).
for tok in \$(cat /proc/cmdline); do
    case "\$tok" in
        ADV_STORM=*)   ADV_STORM=\${tok#ADV_STORM=} ;;
        ADV_THREADS=*) ADV_THREADS=\${tok#ADV_THREADS=} ;;
        ADV_POST=*)    POST=\${tok#ADV_POST=} ;;
    esac
done

if [ "\$POST" = "1" ] && [ -f /lib/modules/loadorder ]; then
    echo "ADVGUEST: phase-0 bring-up — loading the real nvidia stack (ogkm boot sequence)"
    while read -r ko; do
        [ -f "/lib/modules/\$ko" ] || continue
        if insmod "/lib/modules/\$ko" 2>&1; then
            echo "ADVGUEST: insmod \$ko ok"
        else
            echo "ADVGUEST: insmod \$ko FAILED"
            dmesg | grep -i -E "unknown symbol|version magic|NVRM|RmInitAdapter" | tail -6 \
                | sed 's/^/ADVGUEST:   /'
        fi
    done < /lib/modules/loadorder
    maj=\$(awk '/nvidia-frontend|nvidiactl|^ *[0-9]+ nvidia\$/ {print \$1; exit}' /proc/devices)
    if [ -n "\$maj" ]; then
        mknod /dev/nvidiactl c "\$maj" 255 2>/dev/null
        mknod /dev/nvidia0   c "\$maj" 0   2>/dev/null
    fi
    echo "ADVGUEST: nvidia nodes \$(ls /dev/nvidia* 2>/dev/null | tr '\n' ' ')"
fi

echo "ADVGUEST: arming suite post_init=\$POST storm=\${ADV_STORM:-4096} threads=\${ADV_THREADS:-4}"
# ⊘ advguest_init returns -ENODEV on purpose (one-shot suite), so insmod prints a
# harmless "No such device" AFTER the suite has already reported. That is expected.
insmod /lib/modules/advguest.ko \
    post_init=\${POST:-0} storm=\${ADV_STORM:-4096} threads=\${ADV_THREADS:-4} 2>&1 \
    | sed 's/^/ADVGUEST: insmod: /'

echo "ADVGUEST: DONE \$(cut -d' ' -f1 /proc/uptime)s"
poweroff -f
INIT
chmod +x "$ROOT/ird/init"

( cd "$ROOT/ird" && find . | cpio -o -H newc --quiet | gzip -9 ) > "$OUT/initrd.cpio.gz" \
    || die "cpio failed"

echo "== built: $OUT/vmlinuz        $(du -h "$OUT/vmlinuz" | cut -f1)"
echo "== built: $OUT/initrd.cpio.gz $(du -h "$OUT/initrd.cpio.gz" | cut -f1)"
echo "== kernel release: $KREL   post_init: $POST_INIT"
