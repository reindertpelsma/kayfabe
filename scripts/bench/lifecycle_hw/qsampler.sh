#!/bin/bash
# qsampler.sh <out> <stopfile> : record every kayfabe QEMU of this task once, at first sight (binary path, sha256,
# KF3_*/KAYFABE_*/KF_* environment from /proc/<pid>/environ, -name, KF_ARMS from the kernel command line, device options).
# Only QEMUs named kf-fastguest or kayfabe-interactive are touched (read-only /proc access). Poll 0.2 s.
OUT=$1; STOP=$2; declare -A seen shacache
echo "SAMPLER_START $(date -Is)" > "$OUT"
while [ ! -e "$STOP" ]; do
  for p in $(pgrep -x qemu-system-x86 2>/dev/null); do
    [ -n "${seen[$p]:-}" ] && continue
    cmd=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null) || continue
    case "$cmd" in *"-name kf-fastguest"*|*"-name kayfabe-interactive"*) ;; *) seen[$p]=skip; continue ;; esac
    exe=$(readlink /proc/$p/exe 2>/dev/null); [ -n "$exe" ] || continue
    sha=${shacache[$exe]:-}; [ -n "$sha" ] || { sha=$(sha256sum "$exe" | cut -c1-64); shacache[$exe]=$sha; }
    seen[$p]=1
    { echo "QEMU pid=$p t=$(date +%s.%N) uid=$(awk '/^Uid:/{print $2}' /proc/$p/status) exe=$exe sha256=$sha"
      echo "  name=$(echo "$cmd" | grep -o -- '-name [^ ]*')"
      echo "  arms=$(echo "$cmd" | grep -o 'KF_ARMS=[^ ]*')"
      echo "  device=$(echo "$cmd" | grep -o -- '-device kf3-gpu[^ ]*')"
      echo "  env: $(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -E '^(KF3_|KAYFABE_|KF_)' | tr '\n' ' ')"
      echo "  env_nonkf_count=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -vcE '^(KF3_|KAYFABE_|KF_)')"
    } >> "$OUT"
  done
  sleep 0.2
done
echo "SAMPLER_STOP $(date -Is)" >> "$OUT"
