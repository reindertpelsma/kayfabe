#!/usr/bin/env bash
# Prepare a pinned Microsoft bundle on the trusted controller; no Windows code executes here.
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
out=${1:?usage: fetch.sh OUTPUT_DIRECTORY}
mkdir -p "$out"
curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 \
  --connect-timeout 15 --max-time 120 \
  https://download.sysinternals.com/files/DebugView.zip -o "$out/DebugView.zip"
printf '%s  %s\n' a8454253756af10667b82faf2323de536f0b7084d732acba63803df01ce4c316 "$out/DebugView.zip" | sha256sum -c -
python3 - "$out" <<'PY'
import hashlib, pathlib, sys, zipfile
p = pathlib.Path(sys.argv[1])
with zipfile.ZipFile(p/'DebugView.zip') as z:
    for name in ('dbgviewcli64.exe', 'Eula.txt'):
        (p/name).write_bytes(z.read(name))
assert hashlib.sha256((p/'dbgviewcli64.exe').read_bytes()).hexdigest() == '7954a8bbeb1f650bb5d2b8c4c6b642fd4585319d2092433ca841c7d672a4634b'
PY
cp "$here/debugview.ps1" "$here/README.md" "$out/"
sha256sum "$out/dbgviewcli64.exe" "$out/debugview.ps1"
