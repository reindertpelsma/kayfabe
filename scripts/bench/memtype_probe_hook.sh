#!/usr/bin/env bash
# ★★★★★ w312 — MEASURE THE GUEST'S **EFFECTIVE** MEMORY TYPE, INSIDE THE GUEST.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/memtype_probe_hook.sh scripts/bench/boot_capture.sh <tag>
#   needs: GQ_TIMEOUT >= 120. Nothing else — the probe has no dependencies and no GPU
#          requirement; it degrades to refusals rather than to assumptions.
#
# ## What this answers that nothing else in the tree can
#
# `crates/kayfabe-linux-raw/src/memtype.rs` reads back the **host** userspace PTE and says, in
# its own header, that it cannot see past it: on Intel, EPT sets `IPAT` for a normal-RAM
# backing and the guest PTE is ignored; on AMD NPT there is no `IPAT` and the guest PTE is
# honoured. *"Nothing in userspace can read a guest's effective type. A consumer that needs
# that answer must measure it in the guest."* This hook is that consumer.
#
# ⊘ **It measures. It changes nothing.** No `CachePolicy` may move on the strength of one
# run; a reading that says something is wrong is the input to a separate rung with its own
# falsifier, not a licence to edit an attribute.
#
# ## ★★★ The three arms, and what each reading MEANS — pre-registered, so the grading is
# ## not chosen after the numbers arrive
#
# **Arm 1 — always. Guest RAM + the known-positive + the instrument's own failure mode.**
#   Runs with the NVIDIA module loaded, disturbs nothing, and is safe beside any other test.
#   Expected on the GA106 bench guest:
#     anon-wb           `System RAM`, untracked        -> `cached`,        ratio ~1x
#     devmem-nonram     `uncached-minus`               -> `uncached-class`, ratio >= 10x
#     control-mismatch  `System RAM`, untracked        -> NOT `cached`
#   ⇒ **A run in which `known_positive` is not `FIRED` is VOID** (probe exit 2). It is not a
#     green run with a caveat; it is a run that measured nothing, because a probe that only
#     ever says "write-back" has not been shown able to say anything else.
#   ⇒ ★★ **`anon-wb` coming back anything but `cached` is the loud result.** Guest RAM is what
#     every ring, pushbuffer and semaphore this project places lives in. `cached` there is the
#     precondition for the whole data plane; `uncached-class` there would mean every one of
#     those accesses is a bus transaction and would explain a bandwidth cliff with byte-exact
#     results — the C artifact measured 0.12 GB/s against 14 GB/s on exactly that shape.
#
# **Arm 2 — the vendor question, and it is the reason this exists.**
#   Look for a region whose guest record says *uncached* and whose timed verdict says
#   *cached*. That pair is `IPAT` (or an equivalent host-side override) **discarding** the
#   guest's choice, measured rather than inferred from the host's CPU vendor.
#   ⇒ On the AMD bench we expect **no such row**: the guest's choice should be honoured, so
#     record and verdict should agree everywhere.
#   ⇒ **If such a row appears on AMD**, `docs/reference/memory_cacheability.md` §1.1 is wrong
#     about this fleet and the whole "let the guest decide" family of arguments is void here.
#     That is a large result and it is why the row is pre-registered.
#
# **Arm 3 — the NVIDIA BARs. `KAYFABE_MEMTYPE_NVIDIA=1`, and read the caveat first.**
#   ⊘ `mmap` of `/sys/bus/pci/devices/<BDF>/resourceN` is **refused by the kernel while a
#     driver holds the BAR**, so with the NVIDIA module loaded these arms report `EBUSY` —
#     which is a refusal to answer, never an answer. `e2_doorbell_witness.sh` unloads the
#     module first and this hook does NOT, deliberately: unloading it mid-capture changes what
#     every other arm is measuring.
#   ★ The *categorical* half still answers with the module loaded and needs no mmap at all,
#     because the guest kernel's PAT list records what the NVIDIA driver reserved. That is the
#     arm worth reading, and it bears on a live contradiction in this tree:
#     `rm.rs:1520` (`open_usermode`, the BAR0 window the doorbell store lands in) and
#     `rm.rs:1731` (`map_object_uncached`) both pass **`CachePolicy::WriteBack`** while their
#     own rustdoc cites `nv_encode_caching(..., NV_MEMORY_UNCACHED, NV_MEMORY_TYPE_REGISTERS)`.
#     ⇒ **If the guest's PAT list records `uncached`/`uncached-minus` for the NVIDIA BAR0
#       range, the declared requirement at those two call sites is false.** It is inert today
#       (`Backing::DeviceFile`'s attainable policy is `None`, so `require_attainable` cannot
#       refuse it and `mmap` cannot install it either way) — but any future
#       `memtype::require_effective` check over that mapping would report `Downgraded` for a
#       CORRECT mapping, i.e. the oracle would be inverted. Report it; do not fix it here.
#
# ## ⊘ Traps this hook is written against, all measured in this repo
#
#  - **Zero bytes is not "not yet".** Every guest command writes a start marker and an
#    explicit `MEMTYPE_RC=` terminator, so *"file exists, has no terminator"* is detectable.
#    `143` (the job killed) and `124` (the launcher's timeout expired while the job ran on)
#    mean opposite things and arrive as the same word — the `timeout` is INSIDE the guest so
#    the recorded status is the work's own.
#  - **Grade by identity, anchored.** The verdict grep is `^★ MEMTYPE PROBE`; an unanchored
#    match on `MEMTYPE` also hits this hook's own banner and every `MEMTYPE-GATE:` line.
#  - **The probe's refusals go to stderr** under the `MEMTYPE-GATE:` convention that
#    `tests/effective_memtype.rs` adopted after finding its own skip messages had been
#    swallowed on every passing run. `2>&1` here is load-bearing, not tidiness.
set -uo pipefail
SELFDIR=$(cd "$(dirname "$0")" && pwd)
G="$SELFDIR/gssh_nv"
SRC="$SELFDIR/memtype_probe.c"
ARGS=${KAYFABE_MEMTYPE_ARGS:-}
[ "${KAYFABE_MEMTYPE_NVIDIA:-0}" = "1" ] && ARGS="$ARGS --nvidia"

die() { echo "★★★ memtype hook FAILED: $*"; exit 2; }

echo "=== w312 — the guest's EFFECTIVE memory type ==="
[ -x "$G" ]   || die "no gssh_nv at $G"
[ -f "$SRC" ] || die "no probe source at $SRC"
printf '    %-64s %7s bytes  md5 %s\n' "$SRC" "$(stat -c %s "$SRC")" \
       "$(md5sum < "$SRC" | cut -d' ' -f1)"

echo "=== guest preconditions (⊘ each is a DIFFERENT failure from 'the probe said write-back') ==="
$G 'echo "GUEST_UNAME=$(uname -r)"
    echo "GUEST_ARCH=$(uname -m)"
    echo "GUEST_CPU_VENDOR=$(grep -m1 vendor_id /proc/cpuinfo | cut -d: -f2 | tr -d " ")"
    echo "GUEST_HYPERVISOR_FLAG=$(grep -c hypervisor /proc/cpuinfo)"
    echo "GUEST_NVRM_LOADED=$(lsmod | grep -c "^nvidia ")"
    echo "GUEST_DEBUGFS=$(test -r /sys/kernel/debug/x86/pat_memtype_list && echo yes || echo no)"
    echo "GUEST_HAS_GCC=$(command -v gcc >/dev/null && echo yes || echo no)"'

# ⚠ THE HOST CPU VENDOR IS THE ONE THE ASYMMETRY IS ABOUT, and the guest cannot read it —
#   `/proc/cpuinfo` in a guest reports whatever the VMM models. Record the real one from the
#   host side, beside the guest's, so the pair is in the artefact rather than in someone's head.
echo "    HOST_CPU_VENDOR=$(grep -m1 vendor_id /proc/cpuinfo | cut -d: -f2 | tr -d ' ')"
echo "    HOST_UNAME=$(uname -r)"

echo "=== build the probe IN the guest (no toolchain assumptions, no cross-libc) ==="
$G 'cat > /tmp/memtype_probe.c' < "$SRC" || die "could not push the source"
$G 'gcc -O2 -Wall -o /tmp/memtype_probe /tmp/memtype_probe.c 2>&1 | head -20;
    test -x /tmp/memtype_probe && echo GUEST_BUILD=ok || echo GUEST_BUILD=FAILED'
$G 'test -x /tmp/memtype_probe' || die "the probe did not build in the guest"

echo "=== run it as root, under its OWN deadline, with a START marker and an RC terminator ==="
$G "echo STARTED \$(date -Is) > /tmp/memtype.started
    sudo timeout 120 /tmp/memtype_probe $ARGS > /tmp/memtype.out 2>&1
    echo MEMTYPE_RC=\$? >> /tmp/memtype.out"
echo "--- the probe's own output, verbatim ---"
$G 'cat /tmp/memtype.out'
echo "--- end of the probe's output ---"

echo "=== ★★★★★ THE GRADE (anchored) ==="
$G 'grep -E "^★ MEMTYPE PROBE" /tmp/memtype.out | sed "s/^/    /"'
$G 'grep -oE "^MEMTYPE_RC=[0-9]+" /tmp/memtype.out | tail -1 | sed "s/^/    /"'
echo "    --- every refusal the probe made, by name (these are NOT failures of the run):"
$G 'grep -E "^MEMTYPE-GATE:" /tmp/memtype.out | sed "s/^/      /"'
echo "    --- ★★★ ARM 2: any row where the guest asked for uncached and got cached:"
$G 'grep -E "THE GUEST ASKED FOR UNCACHED AND GOT CACHED" /tmp/memtype.out | sed "s/^/      /" ;
    grep -qE "THE GUEST ASKED FOR UNCACHED AND GOT CACHED" /tmp/memtype.out \
      || echo "      (none — the guest'"'"'s choice was honoured everywhere it was measured)"'
echo "    --- ★★★ the other direction: guest records cached, CPU is not:"
$G 'grep -E "THE GUEST RECORDS CACHED AND THE CPU IS NOT" /tmp/memtype.out | sed "s/^/      /" ;
    grep -qE "THE GUEST RECORDS CACHED AND THE CPU IS NOT" /tmp/memtype.out \
      || echo "      (none)"'

echo "=== ⊘ VOID CHECK — the one that decides whether this run measured anything ==="
if $G 'grep -q "known_positive=FIRED" /tmp/memtype.out'; then
  echo "    ★ KNOWN-POSITIVE FIRED — the probe demonstrated it can report something other"
  echo "      than write-back, so its write-back readings are readings."
else
  echo "    ⊘⊘ VOID — the known-positive did NOT fire. Every 'cached' above is UNSUPPORTED:"
  echo "       nothing in this run showed the instrument able to report anything else."
fi

echo "=== the guest driver's own word across the run (a probe must not have disturbed it) ==="
$G 'dmesg 2>/dev/null | grep -iE "nvrm|xid" | tail -8 | sed "s/^/    /"'
echo "=== w312 memtype hook done ==="
