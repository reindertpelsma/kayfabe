#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Cross-build kfwc.sys (kernel driver) and kfwc_collect.exe (companion) on a Linux box.
#
#   build-linux.sh [OUTDIR]            (default OUTDIR: /tmp/kfwc-out)
#
# Needs: clang-cl, lld-link (19 or newer), x86_64-w64-mingw32-gcc, python3, and the Windows
# kit headers/libs. The kit comes from Microsoft's NuGet packages Microsoft.Windows.WDK.x64 and
# Microsoft.Windows.SDK.CPP 10.0.28000.2526 (SHA-256 pinned below), fetched into KFWC_CACHE
# (default ~/.cache/kfwc-wdk, about 275 MB downloaded, ~120 MB extracted) and NEVER into the
# repository. Their licence is Microsoft's pre-release WDK/SDK terms: you may use them to develop
# and test drivers for Windows; you may not redistribute them. Accept those terms yourself
# before running this script. To reuse an already extracted kit set KFWC_KIT=DIR where
# DIR/wdk/c/Include/10.0.28000.0/km and DIR/sdk/c/Include/10.0.28000.0/shared exist.
#
# Produces: kfwc.sys (UNSIGNED), kfwc_collect.exe, driver-pe.json, SHA256SUMS, build-info.txt.
# A successful build is not a load test: see docs/design/V3_WINDOWS_CAPTURE_MODULE.md.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
out=${1:-/tmp/kfwc-out}
cache=${KFWC_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/kfwc-wdk}
version=10.0.28000.2526
kit=10.0.28000.0
mkdir -p "$out"

fetch() { # name sha256 dir
    local name=$1 sha=$2 key=$3 archive="$cache/$3.nupkg"
    [[ -d $cache/$key/c/Include ]] && return 0
    mkdir -p "$cache"
    if [[ ! -f $archive ]] || ! echo "$sha  $archive" | sha256sum -c --status; then
        curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 \
            "https://api.nuget.org/v3-flatcontainer/$name/$version/$name.$version.nupkg" -o "$archive.tmp"
        echo "$sha  $archive.tmp" | sha256sum -c --status || { echo "hash mismatch for $name" >&2; exit 1; }
        mv "$archive.tmp" "$archive"
    fi
    python3 -I - "$archive" "$cache/$key" <<'PYZIP'
import pathlib, sys, zipfile
archive, dest = sys.argv[1:]
with zipfile.ZipFile(archive) as z:
    for n in z.namelist():
        if n.startswith(('c/Include/', 'c/Lib/10.0.28000.0/km/x64/')):
            if '..' in pathlib.PurePosixPath(n).parts:
                raise SystemExit('unsafe archive member')
            z.extract(n, dest)
PYZIP
}

if [[ -n ${KFWC_KIT:-} ]]; then
    kitdir=$KFWC_KIT
else
    fetch microsoft.windows.wdk.x64 63c939fb5a79295bf40e941db592681272219b04edff095fe2f3d123e5579a90 wdk
    fetch microsoft.windows.sdk.cpp be1b419491607eae6f7c57844ebab39face9643c51e2af1d9176a3ba0d0b23fc sdk
    kitdir=$cache
fi
wdk=$kitdir/wdk/c
sdk=$kitdir/sdk/c
cc=${CLANG_CL:-clang-cl}
lld=${LLD_LINK:-lld-link-19}
command -v "$lld" >/dev/null || lld=lld-link

# -Wcast-qual is load-bearing: the mapped BAR0 pointer is "pointer to volatile const" and the
# read-only guarantee depends on the compiler refusing to cast that away (tests/guard_readonly.py
# fails the build if this flag disappears).
cflags=(/nologo /c /kernel /W4 /WX /O2 /GS /D_AMD64_ /DAMD64 /D_WIN64 /D_WIN32_WINNT=0x0A00 /DNTDDI_VERSION=0x0A000008
        /D_KERNEL_MODE /clang:-Wcast-qual
        "/imsvc$wdk/Include/$kit/km" "/imsvc$wdk/Include/$kit/km/crt"
        "/imsvc$sdk/Include/$kit/shared" "/imsvc$sdk/Include/$kit/ucrt")
objs=()
for src in driver/kfwc_drv lib/kfwc_record lib/kfwc_elem lib/kfwc_discover lib/kfwc_engine lib/kfwc_ring; do
    o=$out/$(basename "$src").obj
    (cd "$root" && "$cc" "${cflags[@]}" "/Fo$o" "$src.c")
    objs+=("$o")
done
# The link line is the one that survived the earlier tracer's load investigation
# (origin/codex/windows-gsp-tracer-2026-10-04 LOAD_TESTS.md): /tsaware:no, readable .retplne,
# nonpageable code and data sections. Without them the image is rejected by the loader.
"$lld" /driver /subsystem:native,10.0 /osversion:10.0 /entry:GsDriverEntry /machine:x64 \
    /nodefaultlib /dynamicbase /nxcompat /integritycheck /release /Brepro /tsaware:no \
    /section:.retplne,R /section:.text,ERP /section:.rdata,RP /section:.data,RWP /section:.pdata,RP \
    "/out:$out/kfwc.sys" "${objs[@]}" "/libpath:$wdk/Lib/$kit/km/x64" \
    ntoskrnl.lib hal.lib wdmsec.lib BufferOverflowK.lib
python3 -I "$root/tests/check_driver_pe.py" "$out/kfwc.sys" > "$out/driver-pe.json"
python3 -I "$root/tests/guard_readonly.py" --root "$root" --sys "$out/kfwc.sys"

x86_64-w64-mingw32-gcc -std=gnu11 -O2 -Wall -Wextra -Werror -Wshadow -Wl,--no-insert-timestamp \
    "$root/user/kfwc_collect.c" "$root/lib/kfwc_record.c" "$root/lib/kfwc_meta.c" -ladvapi32 \
    -o "$out/kfwc_collect.exe"

(cd "$out" && sha256sum kfwc.sys kfwc_collect.exe > SHA256SUMS)
{
    echo "source revision: $(git -C "$root" rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "kit: Microsoft.Windows.WDK.x64 + SDK.CPP $version"
    echo "clang-cl: $("$cc" --version | head -1)"
    echo "lld-link: $("$lld" --version | head -1)"
} > "$out/build-info.txt"
cat "$out/SHA256SUMS"
echo "UNSIGNED. Sign on the target (install/install.ps1 does it) and load only on a machine you own."
