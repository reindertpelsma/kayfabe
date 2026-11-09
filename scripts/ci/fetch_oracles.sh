#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# STATUS: LIVE, 2026-11-09.
# fetch_oracles.sh [ROOT] -- check out the oracle trees the GPU-free tests need, shallow, at the PINNED
# commit, and verify the commit id. Today that is third_party/ogkm-580 (open-gpu-kernel-modules
# 580.159.04), read by crates/kf-chip/tests/hwref_is_current.rs through tools/derive_hwref.sh.
#
# Why a script and not `git submodule update`: the stripped tree (scripts/ci/ignore_ladder.py strip)
# keeps .gitmodules but not a submodule checkout, and `submodule update` trusts whatever URL the
# commit under test names. This script has the URL and the pin in ITS OWN text:
#   - the gitlink recorded in the commit (git ls-tree HEAD third_party/ogkm-580) must equal PIN
#     below, so moving the pin is a visible edit of this file and of the gitlink together;
#   - the fetched HEAD must equal PIN, so a different URL or a moved tag cannot slip other bytes in
#     (git checks object ids on fetch; the script checks the one id it was given).
# Any mismatch, fetch failure or missing tool exits non-zero: an oracle that cannot be fetched is a
# failed job, never a skipped test (KF_REQUIRE_ORACLES=1 makes the test itself fail the same way).
# There is no environment override of the pin or the URL: the tests run a COPY of this file with
# those two lines rewritten (scripts/ci/test_fetch_oracles.py).
set -euo pipefail
PIN=b81d58ee0224d1d290bef1c080592b619e184042
URL=https://github.com/NVIDIA/open-gpu-kernel-modules
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "${1:-$here/../..}" && pwd)
dest=$root/third_party/ogkm-580

[[ "$PIN" =~ ^[0-9a-f]{40}$ ]] || { echo "ORACLE_FETCH_FAIL bad pin" >&2; exit 2; }
command -v git >/dev/null || { echo "ORACLE_FETCH_FAIL git is missing" >&2; exit 2; }
linked=$(git -C "$root" ls-tree HEAD third_party/ogkm-580 2>/dev/null | awk '$1 == "160000" { print $3 }')
if [ "$linked" != "$PIN" ]; then
    echo "ORACLE_FETCH_FAIL the commit pins third_party/ogkm-580 at '${linked:-nothing}', this script at $PIN: move both together" >&2
    exit 1
fi

start=$(date +%s)
if [ "$(git -C "$dest" rev-parse HEAD 2>/dev/null || true)" != "$PIN" ]; then
    mkdir -p "$dest"
    # An empty submodule directory is the expected start; anything else is refused, not overwritten.
    if [ -n "$(ls -A "$dest")" ] && [ ! -d "$dest/.git" ]; then
        echo "ORACLE_FETCH_FAIL $dest exists, is not empty and is not a checkout" >&2
        exit 1
    fi
    [ -d "$dest/.git" ] || git -C "$dest" init -q
    ok=0
    for attempt in 1 2 3; do
        if timeout 900 git -C "$dest" fetch -q --depth 1 --no-tags "$URL" "$PIN"; then
            ok=1
            break
        fi
        echo "ORACLE_FETCH_RETRY attempt=$attempt" >&2
        sleep $((attempt * ${KF_ORACLE_RETRY_SLEEP:-5}))
    done
    [ "$ok" = 1 ] || { echo "ORACLE_FETCH_FAIL fetch of $PIN from $URL failed" >&2; exit 1; }
    git -C "$dest" checkout -q --detach "$PIN"
fi
got=$(git -C "$dest" rev-parse HEAD)
[ "$got" = "$PIN" ] || { echo "ORACLE_FETCH_FAIL checked out $got, wanted $PIN" >&2; exit 1; }
[ -s "$dest/src/common/inc/swref/published/nv_ref.h" ] || {
    echo "ORACLE_FETCH_FAIL the checkout has no src/common/inc/swref/published/nv_ref.h" >&2
    exit 1
}
files=$(git -C "$dest" ls-files | wc -l)
bytes=$(du -sb --exclude=.git "$dest" | cut -f1)
gitbytes=$(du -sb "$dest/.git" | cut -f1)
echo "ORACLE_FETCH_OK ogkm-580 sha=$got files=$files worktree_bytes=$bytes git_bytes=$gitbytes seconds=$(($(date +%s) - start))"
