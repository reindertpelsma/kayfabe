#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Run build-linux.sh first to populate its pinned official WDK/SDK cache.
set -euo pipefail
cd "$(dirname "$0")/.."
cache=${KFGT_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/kayfabe-gsp-wdk}
output=${KFGT_OUTPUT:-$PWD/build/load-probe}
kit=10.0.28000.0
mkdir -p "$output"
"${CLANG_CL:-clang-cl}" /nologo /c /kernel /W4 /WX /O2 /GS /D_AMD64_ /DAMD64 /D_WIN64 \
    /D_WIN32_WINNT=0x0A00 /DNTDDI_VERSION=0x0A000008 /D_KERNEL_MODE \
    "/imsvc$cache/wdk/c/Include/$kit/km" "/imsvc$cache/wdk/c/Include/$kit/km/crt" \
    "/imsvc$cache/sdk/c/Include/$kit/shared" "/imsvc$cache/sdk/c/Include/$kit/ucrt" \
    "/Fo$output/load_probe.obj" tests/load_probe.c
"${LLD_LINK:-lld-link-19}" /driver /subsystem:native,10.0 /osversion:10.0 \
    /entry:GsDriverEntry /machine:x64 /nodefaultlib /dynamicbase /nxcompat \
    /integritycheck /release /Brepro \
    "/out:$output/load-probe.sys" "$output/load_probe.obj" \
    "/libpath:$cache/wdk/c/Lib/$kit/km/x64" ntoskrnl.lib BufferOverflowK.lib
sha256sum "$output/load-probe.sys"
echo 'Unsigned diagnostic-only image. This is NOT the GSP recorder.'
