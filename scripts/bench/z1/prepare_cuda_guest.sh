#!/usr/bin/env bash
# Stage a CUDA variant of a fresh host-mode fast image. Administrator setup only.
# Usage: prepare_cuda_guest.sh <base-image-directory> <output-directory> <bin-directory>
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../../.." && pwd)
base=${1:?}; output=${2:?}; bins=${3:?}
mkdir -p "$output" "$bins"
libcuda=/usr/lib/x86_64-linux-gnu/libcuda.so.1
gcc -O0 -I"$REPO/scripts/bench/cuda_min" "$REPO/scripts/bench/cup3.c" \
    "$libcuda" -o "$bins/cup3"
gcc -O2 "$REPO/scripts/bench/cup8bench.c" "$libcuda" -lm -o "$bins/cup8bench"
root=$(mktemp -d "$output/stage.XXXXXX")
trap 'rm -rf "$root"' EXIT
( cd "$root"; gzip -dc "$base/initrd.cpio.gz" | cpio -idm --quiet )
cp "$REPO/scripts/bench/z1/cuda_guest.sh" "$root/bin/kftool"
cp "$bins/cup3" "$bins/cup8bench" "$root/bin/"
chmod +x "$root/bin/kftool" "$root/bin/cup3" "$root/bin/cup8bench"
for program in "$bins/cup3" "$bins/cup8bench"; do
    while read -r library; do
        mkdir -p "$root$(dirname "$library")"
        cp -L "$library" "$root$library"
    done < <(ldd "$program" | awk '/=>/ {print $3} /ld-linux/ {print $1}' | grep '^/')
done
for pattern in libcuda.so libnvidia-ptxjitcompiler.so libnvidia-nvvm.so libnvidia-gpucomp.so; do
    for library in /usr/lib/x86_64-linux-gnu/"$pattern"*; do
        [ -f "$library" ] || continue
        mkdir -p "$root/usr/lib/x86_64-linux-gnu"
        cp -a --remove-destination "$library" "$root/usr/lib/x86_64-linux-gnu/"
    done
done
cp "$base/vmlinuz" "$output/vmlinuz"
( cd "$root"; find . -print0 | cpio --null -o -H newc --quiet | gzip -1 ) \
    > "$output/initrd.cpio.gz"
sha256sum "$output/vmlinuz" "$output/initrd.cpio.gz" "$bins/cup3" "$bins/cup8bench"
