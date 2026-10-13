#!/usr/bin/env bash
# Build the fn-1 negotiation oracle from the REAL ogkm tree.
#
#   build_fn1_oracle.sh <ogkm-root> <out-dir>
#
# exit 0   built: <out-dir>/fn1_negotiation_oracle
# exit 77  SKIPPED: the ogkm tree is absent (the reason is on stdout, one line, "SKIP: ...")
# exit 1   the tree is present and the oracle does not build or an anchor moved: a rotted oracle is
#          an error, never a skip.
#
# The oracle includes two byte-for-byte slices of src/nvidia/src/kernel/vgpu/rpc.c; this script cuts
# them out by the file's own text and refuses if an anchor is missing, duplicated, or the slice lost a
# marker it must contain.
set -euo pipefail
root=${1:?ogkm root}
out=${2:?out dir}
here=$(cd "$(dirname "$0")" && pwd)
rpc="$root/src/nvidia/src/kernel/vgpu/rpc.c"
if [ ! -f "$rpc" ]; then
  echo "SKIP: ogkm tree not found at $root (no src/nvidia/src/kernel/vgpu/rpc.c); set KAYFABE_OGKM_595"
  exit 77
fi
mkdir -p "$out"

# count lines matching a fixed anchor, require exactly one
one() { # file pattern(ERE)
  local n; n=$(grep -cE "$2" "$1" || true)
  [ "$n" = 1 ] || { echo "ERROR: anchor '$2' matches $n lines in $1 (want 1)" >&2; exit 1; }
}
A_BEG='^NV_STATUS RmRpcSetGuestSystemInfo\(OBJGPU \*pGpu, OBJRPC \*pRpc\)$'
A_END='^NV_STATUS rpcSetGuestSystemInfoExt_v15_02\('
B_FN='^static NV_STATUS _issueRpcAndWait\(OBJGPU \*pGpu, OBJRPC \*pRpc\)$'
B_BEG='^    // Now check if RPC really succeeded$'
B_END='^static NV_STATUS _issueRpcAsync\('
for a in "$A_BEG" "$A_END" "$B_FN" "$B_END"; do one "$rpc" "$a"; done

# slice A: [A_BEG, A_END)   slice B: [B_BEG, B_END)
A_BEG=$A_BEG A_END=$A_END awk 'BEGIN{b=ENVIRON["A_BEG"]; e=ENVIRON["A_END"]} $0 ~ b {on=1} $0 ~ e {on=0} on' "$rpc" > "$out/fn1_slice_set_guest_system_info.inc"
# (the "Now check" comment also appears in the large-RPC variant, so start at the first one inside
# _issueRpcAndWait itself)
B_FN=$B_FN B_BEG=$B_BEG B_END=$B_END awk 'BEGIN{f=ENVIRON["B_FN"]; b=ENVIRON["B_BEG"]; e=ENVIRON["B_END"]} $0 ~ f {infn=1} infn && $0 ~ b {on=1} $0 ~ e {on=0; infn=0} on' "$rpc" > "$out/fn1_slice_issue_tail.inc"

need() { # file marker
  grep -qF -- "$2" "$1" || { echo "ERROR: slice $(basename "$1") lost marker: $2" >&2; exit 1; }
}
sa="$out/fn1_slice_set_guest_system_info.inc"
for m in 'rpc_result_private != NV_OK' 'NV_VGPU_GRIDSW_VERSION_MIN_SUPPORTED_INTERNAL_MAJOR' \
         'Retrying with the VGX version requested' 'rpcSetIpVersion' 'NV_RM_RPC_SET_GUEST_SYSTEM_INFO_EXT' \
         'return NV_ERR_INVALID_STATE' 'status = _issueRpcAndWait(pGpu, pRpc);'; do need "$sa" "$m"; done
sb="$out/fn1_slice_issue_tail.inc"
for m in 'rpc_result != NV_VGPU_MSG_RESULT_SUCCESS' 'return NV_ERR_GENERIC;' 'return NV_OK;'; do need "$sb" "$m"; done
[ "$(wc -l < "$sa")" -gt 100 ] || { echo "ERROR: slice A is suspiciously short" >&2; exit 1; }
tail -n 3 "$sb" | grep -q '^}' || { echo "ERROR: slice B does not end at the function's closing brace" >&2; exit 1; }

inc=()
for d in src/nvidia/generated src/nvidia/inc src/nvidia/inc/kernel src/common/sdk/nvidia/inc \
         src/common/sdk/nvidia/inc/ctrl src/common/inc src/nvidia/kernel/inc src/nvidia/interface \
         src/common/shared/inc src/nvidia/inc/libraries src/common/inc/swref src/common/inc/swref/published \
         src/nvidia/arch/nvalloc/common/inc src/nvidia/inc/os; do
  inc+=("-I$root/$d")
done
cc=${CC:-cc}
"$cc" -std=gnu11 -O1 -Wall -Wno-unused-function -Wno-unused-variable -Wno-unused-but-set-variable \
      "${inc[@]}" -I"$out" -o "$out/fn1_negotiation_oracle" "$here/fn1_negotiation_oracle.c"
echo "BUILT: $out/fn1_negotiation_oracle"
