#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Cross-build with official Microsoft WDK headers/libraries, clang-cl and lld.
set -euo pipefail
cd "$(dirname "$0")"
cache=${KFGT_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/kayfabe-gsp-wdk}
output=${KFGT_OUTPUT:-$PWD/build}
version=10.0.28000.2526
kit=10.0.28000.0
mkdir -p "$cache" "$output"
fetch() {
    local name=$1 sha=$2 key=$3 archive="$cache/$3.nupkg"
    if [[ ! -f $archive ]] || ! echo "$sha  $archive" | sha256sum -c --status; then
        curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 \
            "https://api.nuget.org/v3-flatcontainer/$name/$version/$name.$version.nupkg" -o "$archive.tmp"
        echo "$sha  $archive.tmp" | sha256sum -c --status
        mv "$archive.tmp" "$archive"
    fi
    python3 - "$archive" "$cache/$key" <<'PYZIP'
import pathlib, sys, zipfile
archive, destination = sys.argv[1:]
marker = pathlib.Path(destination) / '.extracted-v2'
if not marker.exists():
    with zipfile.ZipFile(archive) as source:
        for name in source.namelist():
            if name.startswith(('c/Include/', 'c/Lib/', 'c/bin/10.0.28000.0/x64/')):
                if '..' in pathlib.PurePosixPath(name).parts:
                    raise SystemExit('unsafe archive member')
                source.extract(name, destination)
    marker.write_text('complete\n')
PYZIP
}
fetch microsoft.windows.wdk.x64 63c939fb5a79295bf40e941db592681272219b04edff095fe2f3d123e5579a90 wdk
fetch microsoft.windows.sdk.cpp be1b419491607eae6f7c57844ebab39face9643c51e2af1d9176a3ba0d0b23fc sdk
compiler=${CLANG_CL:-clang-cl}
linker=${LLD_LINK:-lld-link-19}
args=(/nologo /c /kernel /W4 /WX /O2 /GS /D_AMD64_ /DAMD64 /D_WIN64
      /D_WIN32_WINNT=0x0A00 /DNTDDI_VERSION=0x0A000008 /D_KERNEL_MODE
      "/imsvc$cache/wdk/c/Include/$kit/km" "/imsvc$cache/wdk/c/Include/$kit/km/crt"
      "/imsvc$cache/sdk/c/Include/$kit/shared" "/imsvc$cache/sdk/c/Include/$kit/ucrt")
if [[ ${KFGT_INIT_DIAGNOSTICS:-0} == 1 ]]; then args+=(/DKFGT_INIT_DIAGNOSTICS); fi
for source in gsptrace queue; do
    "$compiler" "${args[@]}" "/Fo$output/$source.obj" "$source.c"
done
"$linker" /driver /subsystem:native,10.0 /osversion:10.0 /entry:GsDriverEntry /machine:x64 \
    /nodefaultlib /dynamicbase /nxcompat /integritycheck /release /Brepro \
    "/out:$output/gsptrace.sys" "$output/gsptrace.obj" "$output/queue.obj" \
    "/libpath:$cache/wdk/c/Lib/$kit/km/x64" ntoskrnl.lib hal.lib wdmsec.lib BufferOverflowK.lib
x86_64-w64-mingw32-gcc -std=c11 -O2 -Wall -Wextra -Werror collect.c -ladvapi32 -Wl,--no-insert-timestamp -o "$output/gsptrace.exe"
x86_64-w64-mingw32-gcc -std=c11 -O2 -Wall -Wextra -Werror tests/windows_api_test.c -ladvapi32 -Wl,--no-insert-timestamp -o "$output/windows_api_test.exe"
mkdir -p "$output/signing"
for name in signtool.exe signtool.exe.manifest mssign32.dll wintrust.dll wintrust.dll.ini appxsip.dll appxpackaging.dll opcservices.dll; do
    cp "$cache/sdk/c/bin/$kit/x64/$name" "$output/signing/"
done
cp "$cache/sdk/c/bin/$kit/x64/"Microsoft.Windows.Build.{Signing,Appx}.*.manifest "$output/signing/"
sha256sum "$output/gsptrace.sys" "$output/gsptrace.exe"
echo 'Unsigned driver built. Use a disposable Windows target with test signing enabled; see README.'
