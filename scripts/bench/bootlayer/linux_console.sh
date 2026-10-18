#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Run INSIDE the disposable Linux guest, after stopping its graphical/RM clients.
# Uses no forced unload and kills no process. A busy module is a named failed phase.
set -euo pipefail
phase=${1:?usage: linux_console.sh unload|redraw|reload}
mark(){ printf 'BOOTLAYER_LINUX %s uptime=%s\n' "$1" "$(cut -d' ' -f1 /proc/uptime)"; }
case "$phase" in
  unload)
    mark UNLOAD_BEGIN
    for module in nvidia_uvm nvidia_drm nvidia_modeset nvidia; do
      if [ -d "/sys/module/$module" ]; then
        rmmod "$module" || { mark "UNLOAD_REFUSED_$module"; exit 1; }
      fi
    done
    mark UNLOAD_COMPLETE
    ;;
  redraw)
    chvt 1
    mark REDRAW_BEGIN
    for n in $(seq 1 40); do
      printf 'BOOTLAYER CONSOLE RETURN %02d %s\n' "$n" "$(date -u +%FT%TZ)" > /dev/tty1
    done
    mark REDRAW_COMPLETE
    ;;
  reload)
    mark RELOAD_BEGIN
    modprobe nvidia
    nvidia-smi
    mark RELOAD_COMPLETE
    ;;
  *) echo 'unknown guest phase' >&2; exit 2 ;;
esac
