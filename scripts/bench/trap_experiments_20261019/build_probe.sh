#!/usr/bin/env bash
# Build the static probe and an initrd with it as /bin/kftool (run on the host: gcc/musl-gcc, cpio, gzip).
set -euo pipefail
SRC=$(cd "$(dirname "$0")" && pwd); OUT=${1:?out dir}; BASE=${2:-/data/claude-tsgmirror-20261018/bench/fastguest}
mkdir -p "$OUT/fg"; (command -v musl-gcc >/dev/null && musl-gcc -O2 -static -o "$OUT/kftool" "$SRC/trapprobe.c") || gcc -O2 -static -o "$OUT/kftool" "$SRC/trapprobe.c"
cp "$BASE/vmlinuz" "$OUT/vmlinuz"
rm -rf "$OUT/root"; mkdir "$OUT/root"; zcat "$BASE/initrd.cpio.gz" | (cd "$OUT/root" && cpio -idm 2>/dev/null)
install -m 0755 "$OUT/kftool" "$OUT/root/bin/kftool"
(cd "$OUT/root" && find . | cpio -o -H newc 2>/dev/null | gzip -1 > "$OUT/initrd.cpio.gz")
ls -la "$OUT/kftool" "$OUT/initrd.cpio.gz"
