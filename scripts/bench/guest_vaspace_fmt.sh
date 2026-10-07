#!/usr/bin/env bash
# POST_CAPTURE_HOOK — ★★★ MEASURE THE ONE NUMBER `:3094` TURNS ON, and scout the fallback.
#
# ## The wall, and why nothing but a measurement can move it
#
# Boot `s28_933a709_spd` stopped on `gpu_vaspace.c:3094`
# (`vaLimitNew <= pGVAS->vaLimitMax`), returning `NV_ERR_INVALID_ARGUMENT` = `0x1f`, the
# value guest userspace independently reported as `UVM_REGISTER_GPU rmStatus`.
#
# Transcribed (not paraphrased) from `ogkm-580.159.04`:
#
#   gpu_vaspace.c:3091  vaLimitNew = mmuFmtEntryIndexVirtAddrHi(pGpuState->pFmt->pRoot, 0,
#                                                               pParams->numEntries - 1);
#   gpu_vaspace.c:3094  NV_ASSERT_OR_RETURN(vaLimitNew <= pGVAS->vaLimitMax, NV_ERR_INVALID_ARGUMENT);
#   gpu_vaspace.c:1121  vaLimitMax = NVBIT64(pFmt->pRoot->virtAddrBitHi + 1) - 1;
#   mmu_fmt.h:178-193   mmuFmtEntryIndexVirtAddrHi(L,0,i) = (i << L->virtAddrBitLo)
#                                                          + (NVBIT64(L->virtAddrBitLo) - 1)
#
# ⇒ the assert is EXACTLY  `numEntries <= 2^(virtAddrBitHi - virtAddrBitLo + 1)`  — the
# published root may not have more entries than the root level HAS. `vaStart`, the requested
# `vaSize`/`vaLimit` (which clamp `vasLimit`, never `vaLimitMax`), and everything this port
# puts on the wire are all absent from it.
#
# ⊘ And `virtAddrBitLo`/`virtAddrBitHi` have never appeared in any log any boot has produced.
#
# ## What this hook does NOT do
#
# It does not patch the guest driver. A guest patch is an instrument, and one that has to be
# fenced back out before any claim about stock-driver behaviour. RM already exports both
# numbers to unprivileged userspace, and both are served entirely inside guest CPU-RM with no
# RPC to GSP — so they answer even while the GSP plane is refusing things:
#
#   0x801806  NV0080_CTRL_CMD_DMA_ADV_SCHED_GET_VA_CAPS   (dma.c:734 -> vaspaceGetVasInfo)
#             gpu_vaspace.c:2367  pParams->vaBitCount = pFmt->pRoot->virtAddrBitHi + 1;
#   0x90f10102 NV90F1_CTRL_CMD_VASPACE_GET_PAGE_LEVEL_INFO
#             gpu_vaspace.c:4003  copies the whole MMU_FMT_LEVEL per level, verbatim
#
# ⚠ Neither exists anywhere in this port's tree (measured: grep for `GET_PAGE_LEVEL_INFO`,
# `0x90f10102`, `0x90f10101` returns nothing), so this reading cannot be an echo of our own
# belief. That is the point of choosing them.
#
# ## ⊘ The fallback is SCOUTED, not assumed
#
# If the reading comes back `vaBitCount=49` the transcription above says :3094 cannot fire,
# and only a printk at the failing site can go further. Whether that is even possible depends
# on the guest carrying driver source, which nobody has checked. So this hook reports what is
# there — and reports it as a SCOUTING RESULT, never as a plan that has been validated.
set -uo pipefail
REPO=${KAYFABE_REPO:-/workspace/bench/kayfabe}
G="$REPO/scripts/bench/gssh_nv"
SRC="$REPO/scripts/bench/vasfmt_probe.c"

die() { echo "★ guest_vaspace_fmt hook FAILED: $*"; exit 2; }
[ -x "$G" ] || die "no gssh_nv at $G"
[ -r "$SRC" ] || die "no probe source at $SRC"

echo "=== source that will run (md5, so a run cannot silently be the other copy) ==="
printf '    %-58s %5s lines  md5 %s\n' "$SRC" "$(wc -l < "$SRC")" \
       "$(md5sum < "$SRC" | cut -d' ' -f1)"

echo "=== push + build inside the guest ==="
$G 'cat > /tmp/vasfmt_probe.c' < "$SRC" || die "could not push the probe"
$G 'gcc -O0 -Wall -o /tmp/vasfmt_probe /tmp/vasfmt_probe.c 2>&1; echo GCC_RC=$?'
$G 'test -x /tmp/vasfmt_probe' || die "the probe did not build; nothing below is a reading"

# ⚠ `timeout` and the rc reported DIRECTLY. A grep verdict on a log has already reported
# success on a red run in this campaign.
echo "=== ★★★ THE READING ==="
$G 'timeout 60 /tmp/vasfmt_probe 2>&1; echo PROBE_RC=$?'

# ---- the fallback, SCOUTED ------------------------------------------------------------
echo "=== ⊘ SCOUTING ONLY — is a guest driver patch even possible on this image? ==="
$G 'echo "--- /usr/src ---";        ls -d /usr/src/nvidia* 2>&1 | head
    echo "--- dkms ---";            (dkms status 2>&1 || echo "no dkms") | head
    echo "--- kernel headers ---";  ls -d /lib/modules/$(uname -r)/build 2>&1
    echo "--- module origin ---";   modinfo -n nvidia 2>&1
    echo "--- gpu_vaspace.c present? ---"; find /usr/src -name gpu_vaspace.c 2>/dev/null | head'

echo "=== hook complete (this line means the OBSERVATION was made, not that it was green) ==="
