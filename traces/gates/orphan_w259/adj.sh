#!/usr/bin/env bash
# adjudicate <file> <line> <label>  — the gate's own verdict, 4 axes, conjunction.
set -uo pipefail
cd /workspace/wt-w259 || exit 2
export CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0
f="$1"; ln="$2"; label="${3:-}"
AXES=( "" "--features kayfabe-device/test-lock-probe" "--features kayfabe-linux-raw/force-host-page-size" "--features kayfabe-qemu-raw/host-isolates" )
cp "$f" "$f.adjbak" || exit 2
sed -i "${ln}s/^\([[:space:]]*\)pub /\1pub(crate) /" "$f"
if ! git diff --quiet -- "$f"; then muta=1; else muta=0; fi
verdict=ORPHAN; failax=""
if [ "$muta" -eq 0 ]; then verdict=NO_MUTATION; else
for ax in "${AXES[@]}"; do
  out=$(cargo check --workspace --quiet $ax 2>&1); rc=$?
  if grep -q 'No space left on device\|LLVM ERROR' <<<"$out"; then verdict=ENOSPC; failax="$ax"; break; fi
  if [ $rc -ne 0 ]; then verdict=LIVE; failax="${ax:-default}"; break; fi
done
fi
mv -f "$f.adjbak" "$f"; touch "$f"
echo "ADJ|$f:$ln|$label|$verdict|failing_axis=${failax}"
