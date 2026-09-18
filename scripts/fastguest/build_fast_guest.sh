#!/usr/bin/env bash
# ★★★★★ THE FAST GUEST — an initrd that boots, insmods ogkm, runs the raw client, and dies.
#
# > Owner, 2026-09-18: *"a small os thats just initrd containing the ogkm kernel module insmod
# > and executes raw client. Whole thing is seconds if kayfabe is mature. Now we wait half an
# > hour per test and is bash time much higher than claude time."*
#
# ## Why this exists — measured, not asserted
#
# `[measured w760, 2026-09-18]` one iteration of the full guest costs **~28 s to `guest is up`**
# plus module load plus an ssh wait, before a single arm runs. A dozen boots were spent that
# day; several produced NO information at all — two died on `kex_exchange_identification` races
# while the guest was already up, four measured a plane that was compiled but not armed, one
# burned an hour to `EXIT=124`. Wall-clock was hours; the information was minutes.
#
# ⊘ And the second-order cost is worse than the first: a 30-minute cycle makes an agent GUESS.
# Five hypotheses were refuted that day, nearly all of the form *"I will reason from these log
# lines rather than spend another half hour"*.
#
# ## The three properties that make it fast, in order of what they buy
#
# 1. **No disk and no services.** `-kernel` + `-initrd`, `console=ttyS0`, no systemd, no
#    cloud-init, no sshd. The kernel reaches `/init` in about a second.
# 2. **Results over the SERIAL CONSOLE, never ssh.** ⊘ This is not a simplification, it is a
#    correctness fix: `[measured w760]` two boots were scored as failures because the harness
#    probed before sshd accepted, while the guest sat at a login prompt. There is no sshd here
#    to race.
# 3. **A TIMEOUT IS A CRASH.** The whole run gets one budget. ⊘ That single rule dissolves the
#    category that cost the most on 2026-09-18: an arm that timed out was SIGKILLed mid-operation
#    and left the device unusable, so the NEXT arm failed — and hours went into telling "this arm
#    is broken" apart from "the arm before it poisoned this one". Nothing here runs long enough
#    to poison anything.
#
# ⇒ Perf and correctness stop being separate lanes. `[measured w760]` a 120x-slow sweep was what
# blacked out 18 correctness verdicts; under a budget that is simply a red test.
#
# ## ⚠ What it is NOT
#
# ⊘ It does not replace the full guest. Some arms want a real environment (multiple processes,
# `nvidia_uvm`, a package manager's driver install) and those keep the fat guest as a slower
# CONFIRMATION lane. The fast guest is the ITERATION lane. A green fast run is not a release
# claim; a red one is always real.
#
# usage: build_fast_guest.sh [guest.qcow2] [outdir]
set -uo pipefail

IMG=${1:-/workspace/bench/guest.qcow2}
OUT=${2:-/workspace/bench/fastguest}
CLIENT=${CLIENT:-/root/kayfabe/target/release/kayfabe-rm-ladder}

die() { echo "build_fast_guest: $*" >&2; exit 1; }

[ -f "$IMG" ]    || die "no guest image at $IMG"
[ -x "$CLIENT" ] || die "no raw client at $CLIENT (cargo build --release -p kayfabe-isolate-host --bin kayfabe-rm-ladder)"
command -v busybox >/dev/null || die "busybox is not installed"
command -v cpio    >/dev/null || die "cpio is not installed"

# ⚠ The image must not be in use. A qcow2 read while QEMU writes it yields a torn read, and the
# failure lands later as a module that will not load — attributed to the module.
if pgrep -x qemu-system-x86 >/dev/null 2>&1; then
    die "a QEMU is running and may be writing $IMG. GPU/bench runs are STRICTLY SERIAL."
fi

mkdir -p "$OUT" || die "cannot create $OUT"
ROOT=$(mktemp -d) || die "mktemp"
trap 'qemu-nbd --disconnect /dev/nbd0 >/dev/null 2>&1; umount "$ROOT/mnt" 2>/dev/null; rm -rf "$ROOT"' EXIT

# ── 1. borrow the kernel and the modules from the fat guest ───────────────────────────────
# ⊘ Taken FROM THE IMAGE rather than built here: the modules must match the kernel they will be
# insmod'ed into, and the fat guest is where that pairing is already known-good. Building ogkm
# against a different kernel is how a vermagic mismatch becomes a mystery at boot.
modprobe nbd max_part=8 2>/dev/null
mkdir -p "$ROOT/mnt"
qemu-nbd --read-only --connect=/dev/nbd0 -f qcow2 "$IMG" || die "qemu-nbd could not attach $IMG"
sleep 1
partprobe /dev/nbd0 2>/dev/null
mount -o ro /dev/nbd0p1 "$ROOT/mnt" 2>/dev/null || mount -o ro /dev/nbd0 "$ROOT/mnt" || die "cannot mount the guest root"

KREL=$(ls "$ROOT/mnt/lib/modules" | head -1)
[ -n "$KREL" ] || die "no /lib/modules in the guest image"
echo "== guest kernel: $KREL"

mkdir -p "$ROOT/ird/lib/modules"
# ⊘⊘⊘ **/boot IS ITS OWN PARTITION, AND THE ROOT ONE IS AN EMPTY DIRECTORY.** Measured on this
# bench image: `nbd0p1` is the 32.5 GiB root and its `/boot` holds NOTHING; the kernel lives on
# `nbd0p16` (913 MiB, Ubuntu's separate boot partition). ⚠ The failure that taught this reads
# as a MISSING KERNEL -- `no vmlinuz for 6.8.0-139-generic in the image` -- while the kernel is
# right there on a partition nobody mounted. An empty directory where a file is expected is
# indistinguishable from an image that never had one, so SEARCH every partition rather than
# concluding absence from the first.
if ! cp "$ROOT/mnt/boot/vmlinuz-$KREL" "$OUT/vmlinuz" 2>/dev/null; then
    mkdir -p "$ROOT/boot"
    got=0
    for part in /dev/nbd0p16 /dev/nbd0p15 /dev/nbd0p14; do
        [ -b "$part" ] || continue
        mount -o ro "$part" "$ROOT/boot" 2>/dev/null || continue
        if cp "$ROOT/boot/vmlinuz-$KREL" "$OUT/vmlinuz" 2>/dev/null; then
            echo "== kernel taken from $part"
            got=1
        fi
        umount "$ROOT/boot" 2>/dev/null
        [ "$got" = 1 ] && break
    done
    [ "$got" = 1 ] || die "no vmlinuz-$KREL on the root or on any boot partition of $IMG"
fi

# ⊘⊘⊘ **TAKE THE DEPENDENCY CLOSURE, NOT THE FOUR NAMES WE HAPPEN TO KNOW.** `[measured w763]`
# copying only `nvidia*.ko` produced `insmod: unknown symbol in module` for all of them, and
# the kernel named why: `nvidia.ko` needs `crypto_ecdh_shared_secret`, `ecc_make_pub_key`,
# `ecc_get_curve`, `ecc_gen_privkey`, `ecc_is_pubkey_valid_full` -- the kernel's own
# `crypto/ecc.ko` -- and `nvidia-modeset.ko` additionally needs `acpi/video.ko` and
# `platform/x86/wmi.ko`. ⚠ None of those is an NVIDIA module and no amount of guessing at
# NVIDIA's file names finds them.
#
# ★ `modules.dep` in the image already states the closure, computed by `depmod` FROM THE
# SYMBOLS, so it cannot drift from what the modules actually import -- and it is flattened and
# ordered, deps last, so loading it right-to-left is a valid order.
DEP="$ROOT/mnt/lib/modules/$KREL/modules.dep"
[ -f "$DEP" ] || die "no modules.dep under $KREL -- cannot resolve the dependency closure"

# one `modname<TAB>space-separated relative dep paths` line per nvidia module we want
: > "$ROOT/ird/lib/modules/loadorder"
found=0
for ko in nvidia nvidia-uvm nvidia-modeset; do
    line=$(grep -E "(^|/)$ko\.ko(\.[a-z]+)?:" "$DEP" | head -1)
    [ -n "$line" ] || continue
    self=${line%%:*}
    deps=${line#*:}
    # ⊘ deps first (right to left), then the module itself
    order=""
    for d in $deps; do order="$d $order"; done
    for rel in $order "$self"; do
        src="$ROOT/mnt/lib/modules/$KREL/$rel"
        [ -f "$src" ] || continue
        base=$(basename "$rel"); base=${base%.zst}; base=${base%.xz}; base=${base%.ko}.ko
        dst="$ROOT/ird/lib/modules/$base"
        [ -f "$dst" ] && continue
        case "$src" in *.zst) zstd -dq -o "$dst" "$src" ;;
                       *.xz)  xz -dc "$src" > "$dst" ;;
                       *)     cp "$src" "$dst" ;; esac
        echo "$base" >> "$ROOT/ird/lib/modules/loadorder"
        found=$((found+1))
    done
done
[ "$found" -gt 0 ] || die "no nvidia modules found under $KREL — is the driver installed in the image?"
echo "== modules taken (with closure): $found"
sed 's/^/==   /' "$ROOT/ird/lib/modules/loadorder"

# ⊘⊘⊘ **THE GSP FIRMWARE IS NOT A MODULE AND `modules.dep` DOES NOT MENTION IT.**
# `[measured w763]` with all six modules loading clean, `RmInitAdapter` still failed:
#   Direct firmware load for nvidia/580.159.04/gsp_ga10x.bin failed with error -2
#   NVRM: RmFetchGspRmImages: No firmware image found
#   NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x61:0x56:1927)
# and the client saw it only as `openat(nvidia<gpu>) errno 5`. ★ The dependency closure is a
# statement about SYMBOLS; a blob the driver opens by PATH at runtime is invisible to it.
# ⇒ Take the whole `nvidia/` firmware tree from the image, by the same rule as the modules:
# from where the working pairing already is, never rebuilt or guessed here.
FW="$ROOT/mnt/lib/firmware/nvidia"
if [ -d "$FW" ]; then
    mkdir -p "$ROOT/ird/lib/firmware"
    cp -a "$FW" "$ROOT/ird/lib/firmware/" 2>/dev/null
    # ⊘ Decompress in place: the kernel's direct-load path opens the bare name, and a
    # `.bin.zst` next to a missing `.bin` reads to the driver as "no firmware image found".
    find "$ROOT/ird/lib/firmware/nvidia" -name '*.zst' 2>/dev/null | while read -r z; do
        zstd -dq -o "${z%.zst}" "$z" && rm -f "$z"
    done
    find "$ROOT/ird/lib/firmware/nvidia" -name '*.xz' 2>/dev/null | while read -r x; do
        xz -dc "$x" > "${x%.xz}" && rm -f "$x"
    done
    echo "== firmware taken: $(find "$ROOT/ird/lib/firmware/nvidia" -type f | wc -l) file(s), $(du -sh "$ROOT/ird/lib/firmware/nvidia" | cut -f1)"
else
    echo "== firmware: ⊘ NONE at /lib/firmware/nvidia in the image - RmInitAdapter will fail 0x61"
fi

umount "$ROOT/mnt"; qemu-nbd --disconnect /dev/nbd0 >/dev/null 2>&1

# ── 2. the initrd ─────────────────────────────────────────────────────────────────────────
mkdir -p "$ROOT/ird"/{bin,dev,proc,sys,tmp}
cp "$(command -v busybox)" "$ROOT/ird/bin/busybox"
cp "$CLIENT" "$ROOT/ird/bin/rmladder"
chmod +x "$ROOT/ird/bin/rmladder"

# ⊘ The client is dynamically linked against glibc unless built for musl; carry what it needs.
if ldd "$CLIENT" >/dev/null 2>&1 && ! ldd "$CLIENT" | grep -q 'not a dynamic'; then
    mkdir -p "$ROOT/ird/lib" "$ROOT/ird/lib64"
    ldd "$CLIENT" | awk '/=>/ {print $3} /ld-linux/ {print $1}' | grep '^/' | sort -u | while read -r so; do
        d="$ROOT/ird$(dirname "$so")"; mkdir -p "$d"; cp -L "$so" "$d/" 2>/dev/null
    done
fi

cat > "$ROOT/ird/init" <<'INIT'
#!/bin/busybox sh
# ★ /init — the whole guest. Mount, load, run, report, die.
/bin/busybox --install -s /bin
mount -t proc  none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev 2>/dev/null

echo "FASTGUEST: up $(cut -d' ' -f1 /proc/uptime)s"

# ⊘⊘⊘ **`insmod` NAMES A CLASS; THE KERNEL NAMES THE SYMBOL.** busybox prints the same
# "unknown symbol in module, or unknown parameter" for a missing dependency, a vermagic
# mismatch and a bad parameter -- three different fixes behind one string. The kernel logs
# which symbol, and `quiet` on our own command line is what hid it. ⇒ dump the ring on
# failure, bounded, so a failed load says WHAT is missing on the first boot rather than the
# third.
# ★ The build wrote `loadorder` in dependency order, deps first. Nothing here knows the
# names; they came from the image's own `modules.dep`.
while read -r ko; do
    [ -f "/lib/modules/$ko" ] || continue
    if insmod "/lib/modules/$ko" 2>&1; then
        echo "FASTGUEST: insmod $ko ok"
    else
        echo "FASTGUEST: insmod $ko FAILED"
        dmesg | grep -i -E "unknown symbol|version magic|disagrees about" | tail -8 \
            | sed 's/^/FASTGUEST:   /'
    fi
done < /lib/modules/loadorder
# ⊘ The device nodes are created by the driver's own open path on a real system; without
# nvidia-modprobe we make them ourselves from /proc/devices.
maj=$(awk '/nvidia-frontend|nvidiactl|^ *[0-9]+ nvidia$/ {print $1; exit}' /proc/devices)
if [ -n "$maj" ]; then
    mknod /dev/nvidiactl c "$maj" 255 2>/dev/null
    mknod /dev/nvidia0   c "$maj" 0   2>/dev/null
fi
# ⊘⊘⊘ **nvidia-uvm IS ITS OWN MAJOR, AND FORGETTING IT READS AS A KAYFABE DEFECT.**
# `[measured w763]` `--uvm-invalidate` failed `W392C open = /dev/nvidia-uvm: No such file or
# directory` -- a MISSING DEVICE NODE in this initrd, reported in the scoreboard as an arm
# failure. ★ On a real system `nvidia-modprobe` makes these; there is none here, so every
# node this suite can ask for is made from `/proc/devices` by name.
uvmmaj=$(awk '/nvidia-uvm$/ {print $1; exit}' /proc/devices)
if [ -n "$uvmmaj" ]; then
    mknod /dev/nvidia-uvm       c "$uvmmaj" 0 2>/dev/null
    mknod /dev/nvidia-uvm-tools c "$uvmmaj" 1 2>/dev/null
fi
mkdir -p /dev/nvidia-caps
capmaj=$(awk '/nvidia-caps/ {print $1; exit}' /proc/devices)
[ -n "$capmaj" ] && mknod /dev/nvidia-caps/nvidia-cap1 c "$capmaj" 1 2>/dev/null
echo "FASTGUEST: nodes $(ls /dev/nvidia* /dev/nvidia-caps/* 2>/dev/null | tr '\n' ' ')"

echo "FASTGUEST: ready $(cut -d' ' -f1 /proc/uptime)s"
# ⊘⊘⊘ **ARMS ARRIVE COMMA-SEPARATED, AND THAT IS NOT A STYLE CHOICE.** The kernel splits
# its command line on WHITESPACE and honours no shell quoting whatsoever, so the obvious
# `KF_ARMS="--timer --engines"` reaches init as the single env value `"--timer` with a literal
# quote, and `--engines"` is dropped on the floor as an unrecognised kernel arg. Written that
# way first; caught by reading the kernel's own parser, not by a run. ⇒ one token, commas.
ARMS=$(echo "${KF_ARMS:-}" | tr -d '"' | tr ',' ' ')
[ -n "$ARMS" ] || ARMS="--timer --engines --doorbell-census"
# ⊘⊘⊘ **THE DEADLINE IS COMPUTED HERE, BECAUSE ONLY HERE IS THE BOOT ALREADY SPENT.**
# `[measured w763]` the runner set `KF_SELF_DEADLINE_MS = (budget - 4) * 1000` -- but that is
# measured from the CLIENT's start, and the boot costs ~9 s of the budget before the client
# exists. So a 25 s budget armed a 21 s client deadline that would have fired at ~30 s wall,
# four seconds AFTER the outer `timeout` killed QEMU. ⇒ The dump the whole lane exists to
# produce could not fire, and the evidence was a serial log truncated mid-word.
# ★ `/init` knows the uptime. The deadline is what is LEFT, minus a margin to speak in.
BUDGET_S=${KF_BUDGET_S:-20}
UP=$(cut -d' ' -f1 /proc/uptime | cut -d. -f1)
LEFT=$(( BUDGET_S - UP - 3 ))
[ "$LEFT" -lt 2 ] && LEFT=2
export KF_SELF_DEADLINE_MS=$(( LEFT * 1000 ))
echo "FASTGUEST: arms $ARMS  deadline ${LEFT}s (budget ${BUDGET_S}s, ${UP}s already spent booting)"
/bin/rmladder --gpu 0 $ARMS 2>&1
echo "FASTGUEST: client rc=$? at $(cut -d' ' -f1 /proc/uptime)s"
echo "FASTGUEST: DONE"
poweroff -f
INIT
chmod +x "$ROOT/ird/init"

( cd "$ROOT/ird" && find . | cpio -o -H newc --quiet | gzip -9 ) > "$OUT/initrd.cpio.gz" \
    || die "cpio failed"

echo "== built: $OUT/vmlinuz  $(du -h "$OUT/vmlinuz" | cut -f1)"
echo "== built: $OUT/initrd.cpio.gz  $(du -h "$OUT/initrd.cpio.gz" | cut -f1)"
echo "== kernel release: $KREL"
