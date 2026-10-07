#!/usr/bin/env bash
# ★★★★★ R33 MILESTONE 2 — RUN THE RAW CE CLIENT **INSIDE THE GUEST**, against our emulated GPU.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/r33_hook_ce_client.sh scripts/bench/boot_capture.sh <tag>
#   needs: GQ_TIMEOUT >= 180
#          KAYFABE_R33_BIN — a **statically linked** kayfabe-rm-ladder (musl). Default:
#            $REPO/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
#
# ## Why this hook exists and `cup2_hook_*` does not answer it
#
# Every guest-side measurement this campaign has is `cup2`, i.e. **libcuda**: 578 records to
# reproduce a hang, with the CUDA runtime, UVM, the context path and the RM control plane all
# in the picture at once. This runs the SAME program that produced the native reference —
# byte for byte, the same binary — with **no libcuda in the process at all**. One black-box
# layer removed, and ~53 ioctls instead of hundreds.
#
# ⇒ Both outcomes are large, and they are pre-registered in `traces/boots/<tag>/`:
#   * it WORKS in the guest  ⇒ the ring/pushbuffer/USERD/doorbell/semaphore data plane is
#     sound under our device model, and the `cup2` wall is specific to libcuda's context path;
#   * it FAILS               ⇒ we have a ~53-ioctl reproduction of a wall we currently
#     reproduce in 578 records, and the census says exactly which ioctl it died on.
#
# ## ⚠⚠ THE CAVEAT THAT DECIDES WHAT THIS MEASURES — stated here, not discovered later
#
# The ladder builds **its own** `FERMI_VASPACE_A` inside the guest and probes **that** VAS.
# It therefore says NOTHING about the VA the GR engine faults on during `cuCtxCreate`: that
# channel belongs to the guest driver's own client, with its own PDB. A probe in the wrong
# address space is this campaign's recorded failure shape, and R33 prints its own range
# handles precisely so the scope is on the artefact rather than in someone's head.
#
# ## ⊘ Three traps this hook is written against, all measured in this repo
#
#  - **A statically linked binary, always.** The guest is a different userland; a dynamic
#    binary that fails to load reports as *"the ladder found no GPU"*, which is a completely
#    different finding. The hook ASSERTS `file`-static and refuses otherwise.
#  - **Zero bytes is not "not yet".** Every guest command writes a start marker and an
#    explicit `R33_RC=` terminator, so *"file exists, has no terminator"* is detectable.
#  - **Grade by identity, anchored.** The verdict grep is `^★ *R33 raw CE client` — an
#    unanchored match on `R33` also hits the info banner that CONTAINS the words of the
#    success line.
set -uo pipefail
SELFDIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SELFDIR/../.." && pwd)
G="$SELFDIR/gssh_nv"
TAG=${1:-r33}
BIN=${KAYFABE_R33_BIN:-$REPO/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder}
ARGS=${KAYFABE_R33_ARGS:---ce-client}

die() { echo "★★★ R33 hook FAILED: $*"; exit 2; }

echo "=== R33 milestone 2 — the raw CE client, in the guest ==="
[ -x "$G" ]   || die "no gssh_nv at $G"
[ -f "$BIN" ] || die "no binary at $BIN (build: cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder)"

# ⊘ WHICH COPY RAN. A run must never be silently a different binary — and the md5 is what
#   lets the guest arm be joined against the native arm as THE SAME PROGRAM.
echo "--- the binary under test (⊘ the native arm must carry the SAME md5):"
printf '    %-72s %9s bytes  md5 %s\n' "$BIN" "$(stat -c %s "$BIN")" "$(md5sum < "$BIN" | cut -d' ' -f1)"
file "$BIN" | sed 's/^/    /'
case "$(file -b "$BIN")" in
  *static*) echo "    ★ STATIC — it will run in the guest's userland" ;;
  *) die "the binary is NOT statically linked. A dynamic binary that fails to load in the
     guest reports as 'no GPU', which is a different finding wearing this one's clothes" ;;
esac

echo "=== guest preconditions (⊘ each is a DIFFERENT failure from 'the client did not work') ==="
$G 'echo "GUEST_UNAME=$(uname -r)"; echo "GUEST_NVIDIA_DEVS=[$(ls /dev/nvidia* 2>/dev/null | tr "\n" " ")]"; echo "GUEST_NVRM_LOADED=$(lsmod | grep -c "^nvidia ")"; echo "GUEST_EUID=$(id -u)"'

echo "=== push the binary into the guest ==="
$G 'cat > /tmp/kayfabe-rm-ladder && chmod +x /tmp/kayfabe-rm-ladder' < "$BIN" || die "could not push the binary"
$G 'echo "GUEST_MD5=$(md5sum < /tmp/kayfabe-rm-ladder | cut -d" " -f1)"; test -x /tmp/kayfabe-rm-ladder && echo GUEST_EXECUTABLE=yes || echo GUEST_EXECUTABLE=no'

echo "=== run it, under its own deadline, with a START marker and an RC terminator ==="
# ★ `timeout 120` inside the guest, NOT only on the ssh: a hung client must produce a
#   TERMINATOR (`R33_RC=124`) rather than an absent file. `143` vs `124` mean opposite
#   things and arrive as the same word — this way the guest's own exit status is the one
#   recorded, and the launcher's timeout cannot be mistaken for the work's.
$G "echo STARTED \$(date -Is) > /tmp/r33.started; timeout 120 /tmp/kayfabe-rm-ladder $ARGS > /tmp/r33.out 2>&1; echo R33_RC=\$? >> /tmp/r33.out"
echo "--- the client's own output, verbatim ---"
$G 'cat /tmp/r33.out'
echo "--- end of the client's output ---"

echo "=== ★★★★★ THE GRADE (anchored — an unanchored 'R33' matches the info banner) ==="
$G 'grep -c "^★     R33 raw CE client" /tmp/r33.out | sed "s/^/    R33_VERDICT_LINES=/"'
$G 'grep -oE "^R33_RC=[0-9]+" /tmp/r33.out | tail -1 | sed "s/^/    /"'
$G 'grep -oE "^  total=[0-9]+ failed=[0-9]+" /tmp/r33.out | sed "s/^/    IOCTL_CENSUS_/"'
echo "    --- the first non-★ arm, if any (this is the minimal repro's head):"
$G 'grep -E "^(FAIL|\?\?) " /tmp/r33.out | head -5 | sed "s/^/      /"'
echo "    --- the LAST ioctl the client issued (where it died, if it did):"
$G 'grep -E "^ +[0-9]+: nr " /tmp/r33.out | tail -3 | sed "s/^/      /"'

echo "=== the guest driver's own word across the run ==="
$G 'dmesg 2>/dev/null | grep -iE "nvrm|xid" | tail -12 | sed "s/^/    /"'
echo "=== R33 milestone 2 hook done ==="
