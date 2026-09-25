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
# ⊘ Derived from THIS SCRIPT's location, never a hardcoded home. `[measured w777]` it read
# `/root/kayfabe/...` and a box whose checkout is `/workspace/kayfabe` failed step 4 of a
# five-step pipeline with "no raw client at ..." — a path assumption, reported as a missing
# build.
KF_ROOT=${KF_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
CLIENT=${CLIENT:-$KF_ROOT/target/release/kayfabe-rm-ladder}

die() { echo "build_fast_guest: $*" >&2; exit 1; }

[ "${KF_FROM_HOST:-0}" = 1 ] || [ -f "$IMG" ] || die "no guest image at $IMG"
[ -x "$CLIENT" ] || die "no raw client at $CLIENT (cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder)"
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
# ★★★★★ **HOST MODE — THE THIN GUEST NEEDS NO FAT GUEST IMAGE.**
#
# > Owner, 2026-09-19: *"don't do the fat guest"*
#
# The kernel and the `nvidia*.ko` only have to MATCH EACH OTHER. Borrowing them from
# `guest.qcow2` was one way to guarantee that; running them from the HOST is another, and it
# is free: the host already runs the driver version this tree pins, so its modules match its
# own kernel by construction. ⊘ That removes the fat guest from the thin lane's critical
# path entirely — no qcow2, no `qemu-nbd`, no 6 GiB image to build first.
#
# ⚠ The kernel must still be one the guest can boot. A vast container shares the host
# kernel, so `/boot/vmlinuz-$(uname -r)` is what the host is RUNNING, and the modules under
# `/lib/modules/$(uname -r)` were built against exactly it.
if [ "${KF_FROM_HOST:-0}" = "1" ]; then
    KREL=$(uname -r)
    echo "== HOST MODE: kernel $KREL, modules from /lib/modules/$KREL"
    cp "/boot/vmlinuz-$KREL" "$OUT/vmlinuz" 2>/dev/null \
      || die "no /boot/vmlinuz-$KREL on the host — a container may not ship the kernel image"
    mkdir -p "$ROOT/ird/lib/modules"
    MODROOT="/lib/modules/$KREL"
    DEP="$MODROOT/modules.dep"
    [ -f "$DEP" ] || die "no $DEP on the host"
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
    [ "$found" -gt 0 ] || die "no nvidia modules under $MODROOT"
    echo "== modules taken from the host (with closure): $found"
    sed 's/^/==   /' "$ROOT/ird/lib/modules/loadorder"
    FW="/lib/firmware/nvidia"
    if [ -d "$FW" ]; then
        mkdir -p "$ROOT/ird/lib/firmware"; cp -a "$FW" "$ROOT/ird/lib/firmware/" 2>/dev/null
        find "$ROOT/ird/lib/firmware/nvidia" -name '*.zst' 2>/dev/null | while read -r z; do
            zstd -dq -o "${z%.zst}" "$z" && rm -f "$z"; done
        echo "== firmware taken from the host: $(find "$ROOT/ird/lib/firmware/nvidia" -type f | wc -l) file(s)"
    else
        echo "== firmware: ⊘ NONE at $FW - RmInitAdapter will fail 0x61"
    fi
    SKIP_NBD=1
fi

if [ "${SKIP_NBD:-0}" != "1" ]; then
modprobe nbd max_part=8 2>/dev/null
mkdir -p "$ROOT/mnt"
qemu-nbd --read-only --connect=/dev/nbd0 -f qcow2 "$IMG" || die "qemu-nbd could not attach $IMG"

# ⊘⊘⊘ RACE, MEASURED w824. `qemu-nbd --connect` RETURNS BEFORE THE KERNEL HAS ENUMERATED THE
# PARTITIONS, and `sleep 1` + one `partprobe` was not enough on this box. Clean A/B on
# `/workspace/bench/guest.qcow2`:
#     connect; partprobe immediately  ->  /dev/nbd0p1 ABSENT   ⇒ mount fails
#     connect; settle; partprobe      ->  /dev/nbd0p1 PRESENT  ⇒ mount OK, real rootfs
#
# ⚠ AND THE FAILURE READ AS SOMETHING ELSE ENTIRELY. The mount fell through to the bare
# `/dev/nbd0` arm, failed too, and died with "cannot mount the guest root" — which reads as a
# CORRUPT OR MISSING IMAGE. The image was fine; only the partition nodes were late. An error
# message naming the wrong layer sends the next person to check the wrong thing.
#
# ⇒ POLL FOR THE ARTEFACT instead of sleeping a guess: a fixed sleep is either too short on a
# slow box or wasted on a fast one, and it encodes no evidence about what it is waiting for.
_p1_ready() { [ -b /dev/nbd0p1 ]; }
for _try in $(seq 1 25); do
    _p1_ready && break
    partprobe /dev/nbd0 2>/dev/null || true
    sleep 0.2
done
if _p1_ready; then
    # ⊘ [measured w825g] "special device /dev/nbd0p1 does not exist" RIGHT AFTER the poll saw
    # it: the poll's own `partprobe` rescans by deleting and re-creating the partition nodes,
    # so the node can be observed mid-rescan. Settle, then retry the MOUNT itself.
    udevadm settle 2>/dev/null || true
    _mounted=0
    for _try in $(seq 1 25); do
        if mount -o ro /dev/nbd0p1 "$ROOT/mnt" 2>/dev/null; then _mounted=1; break; fi
        sleep 0.2
    done
    [ "$_mounted" = 1 ] || die "/dev/nbd0p1 appeared but would not mount (25 tries over 5s)"
else
    # ⊘ No partition table at all is a DIFFERENT thing from one that was late — say which.
    echo "build_fast_guest: /dev/nbd0p1 never appeared after 5s; trying the whole-device arm" >&2
    mount -o ro /dev/nbd0 "$ROOT/mnt" || die "cannot mount the guest root (no nbd0p1, and nbd0 is not a filesystem either — is $IMG a partitioned image?)"
fi

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
fi   # ⊘ end of the qcow2 path — skipped entirely under KF_FROM_HOST=1

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

# ⊘ busybox `date` has no `%N` and /proc/uptime is 10 ms — too coarse for a ms-scale client.
# coreutils `date` as /bin/gdate gives `/init` ns stamps (client-only wall, boot excluded).
if GDATE=$(command -v date) && ldd "$GDATE" >/dev/null 2>&1; then
    cp -L "$GDATE" "$ROOT/ird/bin/gdate"
    ldd "$GDATE" | awk '/=>/ {print $3} /ld-linux/ {print $1}' | grep '^/' | sort -u | while read -r so; do
        [ -f "$ROOT/ird$so" ] && continue
        mkdir -p "$ROOT/ird$(dirname "$so")"; cp -L "$so" "$ROOT/ird$so"
    done
fi

# ★★★★★ **THE CUDA LADDER PAYLOAD (w827, owner: "Test the CUDA ladder").** Opt-in: with
# `KF_CUDA_BINS=<dir>` the initrd also carries the executables in that dir (the host-built cup
# ladder — `scripts/fastguest/cuda_ladder.sh build`) plus the driver's own CUDA userspace:
# `libcuda.so.1` and the `libnvidia-ptxjitcompiler.so.1` it dlopens to JIT the rungs' PTX, each
# with its ldd closure. `/init` then runs `KF_CUDA=<name>` instead of the raw client.
#
# ⊘ Taken from the HOST, by the same rule as the modules in HOST MODE: the userspace must match
# the kernel module's version exactly (libcuda refuses a mismatched RM with
# `CUDA_ERROR_SYSTEM_DRIVER_MISMATCH`), and the host already runs that exact pairing. A
# mismatch is REFUSED here by name rather than discovered as a cuInit failure in the guest.
# ⚠ The binaries are the SAME files the host arm runs — one build, two platforms — so the
# guest/host ratio compares platforms, not compilers.
if [ -n "${KF_CUDA_BINS:-}" ]; then
    [ -d "$KF_CUDA_BINS" ] || die "KF_CUDA_BINS=$KF_CUDA_BINS is not a directory"
    LIBDIR=/usr/lib/x86_64-linux-gnu
    CUDA_LIB=$(readlink -f "$LIBDIR/libcuda.so.1" 2>/dev/null)
    [ -f "$CUDA_LIB" ] || die "no $LIBDIR/libcuda.so.1 on the host — the CUDA userspace is not installed"
    CUDA_VER=${CUDA_LIB##*libcuda.so.}
    KO_VER=$(modinfo -F version "$ROOT/ird/lib/modules/nvidia.ko" 2>/dev/null)
    [ -n "$KO_VER" ] || KO_VER=$(cat /sys/module/nvidia/version 2>/dev/null)
    if [ -n "$KO_VER" ] && [ "$KO_VER" != "$CUDA_VER" ]; then
        die "libcuda is $CUDA_VER but the guest's nvidia.ko is $KO_VER — libcuda would refuse the RM (DRIVER_MISMATCH)"
    fi
    echo "== CUDA userspace: libcuda $CUDA_VER (guest nvidia.ko ${KO_VER:-?})"
    mkdir -p "$ROOT/ird$LIBDIR" "$ROOT/ird/bin/cuda"
    _carry() {  # a file and its ldd closure, symlinks resolved, at the host's own path
        local f=$1
        mkdir -p "$ROOT/ird$(dirname "$f")"; cp -L "$f" "$ROOT/ird$f" || die "cannot carry $f"
        ldd "$f" 2>/dev/null | awk '/=>/ {print $3} /ld-linux/ {print $1}' | grep '^/' | sort -u \
          | while read -r so; do
                [ -f "$ROOT/ird$so" ] && continue
                mkdir -p "$ROOT/ird$(dirname "$so")"; cp -L "$so" "$ROOT/ird$so"
            done
    }
    for so in libcuda.so.1 libnvidia-ptxjitcompiler.so.1 "libnvidia-gpucomp.so.$CUDA_VER"; do
        [ -e "$LIBDIR/$so" ] || { echo "== CUDA: ⊘ $so absent on the host (skipped)"; continue; }
        _carry "$LIBDIR/$so"
    done
    for b in "$KF_CUDA_BINS"/*; do
        [ -f "$b" ] && [ -x "$b" ] || continue
        cp "$b" "$ROOT/ird/bin/cuda/"; _carry "$b"; rm -f "$ROOT/ird$b"
        rmdir -p "$ROOT/ird$(dirname "$b")" 2>/dev/null
    done
    echo "== CUDA ladder carried: $(ls "$ROOT/ird/bin/cuda" | tr '\n' ' ')($(du -sh "$ROOT/ird$LIBDIR" | cut -f1) of libs)"
fi

cat > "$ROOT/ird/init" <<'INIT'
#!/bin/busybox sh
# ★ /init — the whole guest. Mount, load, run, report, die.
/bin/busybox --install -s /bin
mount -t proc  none /proc
mount -t sysfs none /sys
mount -t devtmpfs none /dev 2>/dev/null

echo "FASTGUEST: up $(cut -d' ' -f1 /proc/uptime)s"

# ★★★★★ **w794 — LEAVE THE INITRAMFS, OR THE ISOLATE SANDBOX CANNOT BE BUILT.**
#
# ⊘⊘⊘ `[measured w788]` `--engines` and `--concurrency` both fail their R10 rung with
#   `FAIL  R10 isolate = it did not start: kind=spawn-failed -- isolate refused to start:
#    Other(19270)`
# which is `pivot_root failed (errno 22)`. `EINVAL` there is not a bug in the sandbox: the
# kernel REFUSES `pivot_root` out of the initial rootfs, by design, on every kernel. An
# initramfs-only guest therefore cannot host a sandboxed isolate at all.
#
# ⚠ **The fix belongs HERE and not in the sandbox.** Falling back to `chroot` when
# `pivot_root` is unavailable would silently weaken containment on exactly the path whose
# containment IS the product (`hostile_guest_isolation_is_the_value_proposition`), and it
# would do it in the harness that most boots run. A guest image limitation must not become a
# security relaxation -- `THE_CONSTRAINTS` w729: *a pass bought by relaxing a constraint is
# not a pass*.
#
# ⇒ Move to a real tmpfs root and `switch_root` into it. After this, `/` is an ordinary mount
# and `pivot_root` is legal. ⊘ The copy is the cost: the image carries modules and GSP
# firmware, so this is ~150 MB of page-cache-to-tmpfs, measured in the boot line below rather
# than assumed.
#
# ⊘ Fail SOFT. A guest that cannot switch_root should still run every rung that does not need
# an isolate, and say which state it is in -- an image that silently refused to boot would
# cost more than the two rungs this unblocks.
# ⊘ **The re-entry guard.** `switch_root` re-execs THIS script as the new root's `/init`, so
# without a marker the second pass tries to switch again, forever. The marker is written into
# the new root before the switch and its presence is what says "already switched".
t0=$(cut -d' ' -f1 /proc/uptime)
if [ -f /.kf_switched ]; then
    echo "FASTGUEST: root is a real mount (switch_root done) — the isolate sandbox can pivot_root"
elif mkdir -p /newroot && mount -t tmpfs -o size=90% tmpfs /newroot 2>/dev/null; then
    # ⊘ `cp -a` per top-level entry, not `tar --exclude`: busybox's tar does not take the
    # GNU `--exclude` spelling, and `[measured w795]` the pipe failed silently and the guest
    # correctly reported `switch_root copy FAILED` — the fail-soft path earning itself.
    copied=1
    for e in /*; do
        case "$e" in
            /newroot|/proc|/sys|/dev) continue ;;
        esac
        cp -a "$e" /newroot/ 2>/dev/null || copied=0
    done
    if [ "$copied" = 1 ]; then
        mkdir -p /newroot/proc /newroot/sys /newroot/dev /newroot/oldroot
        : > /newroot/.kf_switched
        t1=$(cut -d' ' -f1 /proc/uptime)
        echo "FASTGUEST: switch_root prepared in $(echo "$t1 $t0" | awk '{printf "%.1f", $1-$2}')s"
        umount /dev 2>/dev/null; umount /sys 2>/dev/null; umount /proc 2>/dev/null
        exec /bin/busybox switch_root /newroot /init
    fi
    echo "FASTGUEST: ⊘ switch_root copy FAILED — staying on initramfs; R10 (isolate) will refuse"
else
    echo "FASTGUEST: ⊘ no tmpfs for switch_root — staying on initramfs; R10 (isolate) will refuse"
fi

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
# ★ ns stamps when the image carries coreutils `date` (the CUDA payload does); the raw client's
# own client-only wall is then `client wall_ms`, boot excluded.
_ns() { if [ -x /bin/gdate ]; then /bin/gdate +%s%N; else echo 0; fi; }
if [ -n "${KF_CUDA:-}" ]; then
    # ★★★ THE CUDA LADDER — one rung per boot, so the device's per-token DOORBELL-LEDGER
    # (printed as each channel is freed) belongs to exactly this program. The graded lines are
    # the workload's OWN values, lifted to anchored `^CUPn_*=` keys here.
    export LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:/lib/x86_64-linux-gnu
    rc=0
    for prog in $(echo "$KF_CUDA" | tr ',' ' '); do
        [ -x "/bin/cuda/$prog" ] || { echo "FASTGUEST: CUDA $prog ABSENT from the image"; rc=127; continue; }
        echo "FASTGUEST: CUDA $prog start $(cut -d' ' -f1 /proc/uptime)s"
        t0=$(_ns)
        ( cd /tmp && "/bin/cuda/$prog" ) > /tmp/$prog.out 2>&1
        prc=$?
        t1=$(_ns)
        cat /tmp/$prog.out
        echo "FASTGUEST: CUDA $prog rc=$prc wall_ms=$(( (t1 - t0) / 1000000 ))"
        case "$prog" in
            cup2) v=$(sed -n 's/^CE rv=\(0x[0-9a-f]*\) .*/\1/p' /tmp/$prog.out | tail -1)
                  echo "CUP2_VAL=${v:-NO_RESULT_LINE}" ;;
            cup3) v=$(sed -n 's/^KERNEL rv=\([0-9]*\) .*/\1/p' /tmp/$prog.out | tail -1)
                  echo "CUP3_VAL=${v:-NO_RESULT_LINE}" ;;
            cup8) l=$(grep '^CUP8 RESULT ' /tmp/$prog.out | tail -1)
                  echo "CUP8_BAD=$(echo "$l" | sed -n 's/.* bad=\([0-9]*\) .*/\1/p')"
                  echo "CUP8_MAXERR=$(echo "$l" | sed -n 's/.* maxerr=\([^ ]*\) .*/\1/p')" ;;
        esac
        [ "$prc" = 0 ] || rc=$prc
    done
    echo "FASTGUEST: client rc=$rc at $(cut -d' ' -f1 /proc/uptime)s"
else
t0=$(_ns)
/bin/rmladder --gpu 0 $ARMS 2>&1
crc=$?
t1=$(_ns)
[ "$t0" != 0 ] && echo "FASTGUEST: client wall_ms=$(( (t1 - t0) / 1000000 ))"
echo "FASTGUEST: client rc=$crc at $(cut -d' ' -f1 /proc/uptime)s"
fi
echo "FASTGUEST: DONE"
poweroff -f
INIT
chmod +x "$ROOT/ird/init"

( cd "$ROOT/ird" && find . | cpio -o -H newc --quiet | gzip -9 ) > "$OUT/initrd.cpio.gz" \
    || die "cpio failed"

echo "== built: $OUT/vmlinuz  $(du -h "$OUT/vmlinuz" | cut -f1)"
echo "== built: $OUT/initrd.cpio.gz  $(du -h "$OUT/initrd.cpio.gz" | cut -f1)"
echo "== kernel release: $KREL"
