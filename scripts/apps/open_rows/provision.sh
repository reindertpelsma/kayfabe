#!/usr/bin/env bash
# provision.sh [ROOT] ["steps"]: fetch and build what the open FFmpeg/DCGM rows need, on THIS machine, from the repo only (needs root, network).
#   nvcodec_probe   built with gcc against nv-codec-headers at the pinned upstream commit      -> ROOT/bin/nvcodec_probe
#   ffmpeg          the distribution's package (signed by the distribution), nothing downloaded by us
#   DCGM            NVIDIA's apt repository, key from NVIDIA's official URL, exact version      -> installed, and ROOT/dcgm_root.tar
# Prints one OPENROWS_<step>=ok|FAIL reason=... line per step; a failed step never stops the others and never makes a gate bundle fail
# (build_bundle.sh ignores the exit status). Exit status: 0 when every step is ok. Trust anchors: docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=pins.env
. "$HERE/pins.env"
# shellcheck source=../pin_lib.sh
. "$HERE/../pin_lib.sh"
ROOT=${1:-/opt/open_rows}; FAILS=0
mkdir -p "$ROOT/bin" "$ROOT/src"
step() { local n=$1; shift; if "$@" >"$ROOT/$n.log" 2>&1; then echo "OPENROWS_$n=ok"; else echo "OPENROWS_$n=FAIL reason=$(tail -n 1 "$ROOT/$n.log" | tr ' =' '__' | cut -c1-120)"; FAILS=$((FAILS+1)); fi; }
# a fresh Ubuntu machine runs unattended-upgrades for a while, which holds the dpkg lock (a native run lost the DCGM step to it): wait, at most 20 minutes
wait_apt() { local i=0; while pgrep -x 'apt|apt-get|dpkg|unattended-upgr|apt.systemd.dai' >/dev/null && [ "$i" -lt 240 ]; do sleep 5; i=$((i+1)); done; }
apt_install() {
  wait_apt
  # a fresh machine has no package lists; under the box's cache-only shim apt-get is a function that does its own lookup, so do not update there
  if [ "$(type -t apt-get)" != function ] && [ ! -e "$ROOT/.apt_updated" ]; then apt-get update -qq && touch "$ROOT/.apt_updated" || return 1; fi
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends "$@"
}

probe() {
  { command -v gcc >/dev/null && command -v git >/dev/null; } || apt_install build-essential git || return 1
  pin_git_checkout "$ROOT/src/nv-codec-headers" "$OPENROWS_NVCH_URL" "$OPENROWS_NVCH_TAG" "$OPENROWS_NVCH_COMMIT" || return 1
  gcc -O2 -Wall -I"$ROOT/src/nv-codec-headers/include" "$HERE/nvcodec_probe.c" -o "$ROOT/bin/nvcodec_probe" -ldl
}
ffmpeg_pkg() { command -v ffmpeg >/dev/null || apt_install ffmpeg || return 1; ffmpeg -version | head -n 1; }
dcgm() {
  local rel base keyring list cuda
  cuda=$(nvidia-smi 2>/dev/null | sed -n 's/.*CUDA Version: *\([0-9][0-9]*\)\..*/\1/p' | head -n 1); cuda=${cuda:-12}   # the flavour the driver wants
  rel=$(. /etc/os-release && echo "${ID}${VERSION_ID//./}"); base=$OPENROWS_NVIDIA_REPO_BASE/$rel/x86_64
  keyring=/usr/share/keyrings/kayfabe-nvidia-cuda.gpg; list=/etc/apt/sources.list.d/kayfabe-nvidia-cuda.list
  if ! grep -rqs "developer.download.nvidia.com/compute/cuda/repos/$rel" /etc/apt/sources.list /etc/apt/sources.list.d; then
    command -v gpg >/dev/null || apt_install gnupg ca-certificates curl || return 1
    curl -fsSL --connect-timeout 20 -o "$ROOT/nvidia.key" "$base/$OPENROWS_NVIDIA_KEY_FILE" || return 1
    gpg --dearmor --yes -o "$keyring" "$ROOT/nvidia.key" || return 1
    echo "deb [signed-by=$keyring] $base/ /" > "$list" || return 1
  fi
  # Under the Vast payload apt-get is a cache-only shell function that holds no NVIDIA package. The one sanctioned bypass is in pin_lib.sh
  # (test_apps_pins.py pins it to that one place), so this step does NOT add another: it names the reason and stops (owner decision pending,
  # docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md section 3). On a standalone machine apt-get is the real one and the install runs.
  [ "$(type -t apt-get)" != function ] || { echo "dcgm: apt_shim_cache_only (the box's apt-get holds no NVIDIA packages)"; return 1; }
  wait_apt; apt-get -o DPkg::Lock::Timeout=600 -o Acquire::Retries=3 update -qq || return 1
  # shellcheck disable=SC2046
  wait_apt; apt-get -o DPkg::Lock::Timeout=600 -o Acquire::Retries=3 install -y -qq --no-install-recommends $(for p in $OPENROWS_DCGM_PACKAGE_STEMS; do echo "$p$cuda=$OPENROWS_DCGM_VERSION"; done) || return 1
  local pkgs; pkgs=$(dpkg-query -W -f='${Package}\n' 'datacenter-gpu-manager-4*' dcgmi libdcgm nv-hostengine 2>/dev/null | sort -u)
  # the installed files, for a guest that cannot reach the network: the bundle carries them and the row unpacks them
  dpkg -L $pkgs | sort -u | while read -r f; do if [ -f "$f" ] || [ -L "$f" ]; then echo "${f#/}"; fi; done > "$ROOT/dcgm_files.txt"
  tar -C / -cf "$ROOT/dcgm_root.tar" -T "$ROOT/dcgm_files.txt" || return 1
  command -v dcgmi nv-hostengine
}
STEPS=${2:-nvcodec_probe ffmpeg dcgm}   # provision.sh ROOT "nvcodec_probe ffmpeg" runs a subset
want() { case " $STEPS " in *" $1 "*) return 0;; esac; return 1; }
want nvcodec_probe && step nvcodec_probe probe
want ffmpeg && step ffmpeg ffmpeg_pkg
want dcgm && step dcgm dcgm
[ "$FAILS" -eq 0 ]
