#!/usr/bin/env bash
# Add the guest-only module/owner client to a COPY of a staged fastguest initrd.
set -euo pipefail
base=$(realpath "${1:?base initrd}")
probe=$(realpath "${2:?built guest probe module}")
owner=$(realpath "${3:?static owner client}")
driver=$(realpath "${4:?patched pinned nvidia module}")
output=$(realpath -m "${5:?new output initrd}")
[ ! -e "$output" ] || { echo 'Output must be new' >&2; exit 2; }
script=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'find "$tmp" -mindepth 1 -delete; rmdir "$tmp"' EXIT
(cd "$tmp" && gzip -dc "$base" | cpio --quiet -id --no-absolute-filenames)
[ -f "$tmp/init" ] && [ -f "$tmp/bin/busybox" ]
original="$tmp/lib/modules/nvidia.ko"
[ "$(modinfo -F version "$original")" = 580.159.04 ]
[ "$(modinfo -F version "$driver")" = 580.159.04 ]
[ "$(modinfo -F vermagic "$original")" = "$(modinfo -F vermagic "$driver")" ]
[ "$(modinfo -F vermagic "$original")" = "$(modinfo -F vermagic "$probe")" ]
install -m 644 "$driver" "$original"
install -m 755 "$script/translated_rc_guest.sh" "$tmp/bin/rmladder"
install -m 755 "$owner" "$tmp/bin/kf_rc_owner"
install -m 644 "$probe" "$tmp/lib/modules/kf_rc_probe.ko"
(cd "$tmp" && find . -print0 | cpio --quiet --null -o -H newc | gzip -1) >"$output"
