#!/usr/bin/env bash
# build_instrumented.sh — produce an `nvidia.ko` carrying the GSP-RM RPC recorder.
#
# Task #178, `replay-conformance`. Spec: docs/design/rpc_trace_capture.md.
#
# ────────────────────────────────────────────────────────────────────────────────
# ⚠ WHY THIS BUILDS FROM THE FULL SOURCE TREE AND NOT FROM /usr/src.
#
# The installed DKMS tree (`/usr/src/nvidia-580.159.04/`) ships `nv-kernel.o_binary`
# — the entire OS-agnostic RM, PRECOMPILED. `message_queue_cpu.c` is inside that
# blob. Nothing in the DKMS tree can be patched to reach it. So the instrumented
# module has to come from a checkout of `open-gpu-kernel-modules` at exactly the
# installed version, which is also what makes the userspace/kernel version match
# hold: we are rebuilding 580.159.04, not upgrading to it.
#
# ★ THE STOCK MODULE ON DISK IS NEVER TOUCHED. This script writes only into its
# own build tree and its own output directory. `capture.sh` `insmod`s the result
# BY PATH and restores the stock module with a plain `modprobe`. There is no step
# anywhere in this pipeline that writes to /lib/modules.
# ────────────────────────────────────────────────────────────────────────────────
#
# Run this ON the GPU box. From a dev box:
#
#   rsync -a scripts/rpctrace/ vb:~/rpctrace/
#   ssh vb 'sudo ~/rpctrace/build_instrumented.sh'
#
# Requires: a pristine ogkm checkout (default ~/ogkm-580.159.04), kernel headers
# for the running kernel, and gcc.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD="${BUILD:-$HOME/ogkm-rpctrace-build}"
OUT="${OUT:-$HOME/rpctrace-out}"
JOBS="${JOBS:-$(nproc)}"

# ────────────────────────────────────────────────────────────────────────────────
# ⚠ THE DRIVER VERSION IS DISCOVERED, NOT ASSUMED.
#
# This was `WANT_VERSION="580.159.04"`, a constant, because the only bench was on
# 580.159.04. `ogkm is VERSIONED, not the spec` — the second and third boxes came
# up on 575.51.03, and a hardcoded 580 here fails LOUDLY (good) only because the
# version.mk check below happens to exist. The default is now the RUNNING driver,
# read from /proc/driver/nvidia/version, which is the version the built module
# must match anyway; --version overrides it for an offline build.
# ────────────────────────────────────────────────────────────────────────────────
#
# ⊘ ONE parser, used by both the default and the mismatch guard below. The two
# wordings this must survive, both seen on real benches:
#   580: NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  580.159.04 ...
#   575: NVRM version: NVIDIA UNIX x86_64 Kernel Module  575.51.03  Wed Apr 16 ...
# So anchor on "Kernel Module", skip anything that is not a digit, and take the
# first dotted number. Emits nothing when it cannot parse — callers check.
#
detect_running_version() {
  [ -r /proc/driver/nvidia/version ] || return 0
  sed -n 's/.*Kernel Module[^0-9]*\([0-9][0-9.]*[0-9]\).*/\1/p' /proc/driver/nvidia/version | head -1
}

WANT_VERSION="${WANT_VERSION:-}"
[ -n "$WANT_VERSION" ] || WANT_VERSION="$(detect_running_version)"
PRISTINE="${PRISTINE:-}"
PATCHFILE="${PATCHFILE:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --pristine) PRISTINE="$2"; shift 2;;
    --build)    BUILD="$2";    shift 2;;
    --out)      OUT="$2";      shift 2;;
    --jobs)     JOBS="$2";     shift 2;;
    --version)  WANT_VERSION="$2"; shift 2;;
    --patch)    PATCHFILE="$2";    shift 2;;
    *) echo "unknown argument: $1" >&2; exit 2;;
  esac
done

die() { echo "‼ $*" >&2; exit 1; }
say() { echo "── $*"; }

[ -n "$WANT_VERSION" ] \
  || die "could not determine the driver version and --version was not given"
PRISTINE="${PRISTINE:-$HOME/ogkm-$WANT_VERSION}"

# ★ ONE RECORDER, VERSION-ANCHORED HOOKS. `nv_rpctrace.{c,h}` are shared verbatim
# across every version; only the patch's ANCHORS are version-specific, because the
# NVIDIA source they attach to genuinely differs (575's GspMsgQueueReceiveStatus
# has no `exit:` label; 580's does). A per-version patch is picked up automatically
# if it exists, and the generic one is used otherwise — ⊘ never a forced apply,
# because `patch --forward` dropping a hunk yields a module that builds, loads, and
# records HALF the RPCs with no error anywhere.
if [ -z "$PATCHFILE" ]; then
  if [ -f "$HERE/rpctrace-$WANT_VERSION.patch" ]; then
    PATCHFILE="$HERE/rpctrace-$WANT_VERSION.patch"
  else
    PATCHFILE="$HERE/rpctrace.patch"
  fi
fi
[ -f "$PATCHFILE" ] || die "no patch file at $PATCHFILE"
say "version=$WANT_VERSION  pristine=$PRISTINE  patch=$(basename "$PATCHFILE")"

# ── 0. Preconditions, each one a thing that has cost somebody a day somewhere ──
[ -d "$PRISTINE" ] || die "no pristine checkout at $PRISTINE"
grep -q "^NVIDIA_VERSION = $WANT_VERSION\$" "$PRISTINE/version.mk" \
  || die "$PRISTINE is not $WANT_VERSION (version.mk says: $(grep '^NVIDIA_VERSION' "$PRISTINE/version.mk"))"

KVER="$(uname -r)"
[ -d "/lib/modules/$KVER/build" ] || die "no kernel headers for $KVER"

# ⚠ The userspace/kernel-module version trap. We are rebuilding the SAME version,
# so this should hold trivially — which is exactly why it is worth asserting: a
# silent mismatch here surfaces as an inscrutable nvidia-smi failure much later.
#
# ⚠⚠ MEASURED, 2026-08-03, on the GA102 box: this check FAILED A CORRECT TREE.
# It read the version with `s/.*Kernel Module for [^ ]* *\([0-9.]*\).*/\1/p`,
# which is coupled to 580's /proc wording. 575.51.03 prints
#   `NVRM version: NVIDIA UNIX x86_64 Kernel Module  575.51.03  Wed Apr 16 ...`
# — no "for" — so the sed matched NOTHING and `running` was the EMPTY STRING. The
# guard then reported `running driver is , source tree is 575.51.03`, i.e. it
# refused the build on evidence it had failed to gather. It is a false red rather
# than a false green only by luck: the empty string can never equal a version, so
# a parse failure and a genuine mismatch are INDISTINGUISHABLE in its output.
# ⇒ Both readers now go through `detect_running_version`, and an unparseable
# /proc is now its own named error instead of masquerading as a mismatch.
if [ -r /proc/driver/nvidia/version ]; then
  running="$(detect_running_version)"
  [ -n "$running" ] \
    || die "/proc/driver/nvidia/version exists but no version could be parsed from it: $(cat /proc/driver/nvidia/version)"
  [ "$running" = "$WANT_VERSION" ] \
    || die "running driver is $running, source tree is $WANT_VERSION — refusing to build a mismatch"
  say "running driver $running matches the source tree"
fi

if command -v mokutil >/dev/null 2>&1; then
  if mokutil --sb-state 2>/dev/null | grep -qi "SecureBoot enabled"; then
    die "Secure Boot is ON: an unsigned instrumented module will not load. Stop here."
  fi
fi

# ── 1. A FRESH tree every time ────────────────────────────────────────────────
# ⊘ Not "patch if not already patched". An idempotence check on a patch is a
# check on the patch's own idea of what it did; a re-derived tree cannot be half
# applied. The copy costs ~10s and removes the entire question.
say "re-deriving build tree $BUILD from $PRISTINE"
rm -rf "$BUILD"
mkdir -p "$BUILD"
tar -C "$PRISTINE" --exclude=.git -cf - . | tar -C "$BUILD" -xf -

# ── 2. Drop the recorder in, twice, from ONE source file ──────────────────────
# The header goes to both the OS layer (which implements it) and the RM (which
# calls it): two include paths that cannot be shared, one file that cannot drift.
say "installing recorder sources"
install -m 0644 "$HERE/nv_rpctrace.c" "$BUILD/kernel-open/nvidia/nv_rpctrace.c"
install -m 0644 "$HERE/nv_rpctrace.h" "$BUILD/kernel-open/nvidia/nv_rpctrace.h"
install -m 0644 "$HERE/nv_rpctrace.h" "$BUILD/src/nvidia/inc/kernel/gpu/gsp/nv_rpctrace.h"
cmp -s "$HERE/nv_rpctrace.h" "$BUILD/kernel-open/nvidia/nv_rpctrace.h" \
  && cmp -s "$HERE/nv_rpctrace.h" "$BUILD/src/nvidia/inc/kernel/gpu/gsp/nv_rpctrace.h" \
  || die "the two header copies differ from the source — refusing to build"

# ── 3. Apply the hooks ────────────────────────────────────────────────────────
say "applying $(basename "$PATCHFILE")"
( cd "$BUILD" && patch -p1 --forward --no-backup-if-mismatch < "$PATCHFILE" ) \
  || die "patch did not apply cleanly against $WANT_VERSION"

# ⊘ `patch --forward` returns 1 when hunks are rejected but leaves the tree half
# patched, and `set -e` would catch that here — but only because of the `||` above.
# Assert the absence of rejects DIRECTLY: a .rej left behind means some hooks are
# in and some are not, which is the one failure that still builds and still loads.
if find "$BUILD" -name '*.rej' | grep -q .; then
  die "patch left rejects: $(find "$BUILD" -name '*.rej')"
fi

# ★ Prove the hooks are where the constraint says they are, on the real post-patch
# text, rather than trusting that the patch header describes the patch. The send
# hook must appear BEFORE the encrypt call and the receive hook AFTER the decrypt
# call, in file order.
#
# ⚠ MEASURED, 2026-08-03: the first version of this check anchored on the bare
# names `ccslEncryptWithRotationChecks` / `NV_RPCTRACE_DIR_CPU_TO_GSP`, and it
# FAILED THE BUILD on a correctly-placed hook — because the hook's own comment
# names the encrypt function, so the "encrypt call" it found was three lines of
# prose above the hook. The gate was measuring its own documentation. Anchoring
# on the ASSIGNMENT (`= ccsl…`) and on the CALL (`nv_rpctrace_record(`) matches
# code and only code. Worth keeping the story: the check fired, and the thing it
# caught was itself.
mqc="$BUILD/src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c"
l_send=$(grep -n "nv_rpctrace_record(NV_RPCTRACE_DIR_CPU_TO_GSP" "$mqc" | head -1 | cut -d: -f1)
l_enc=$(grep -n "= ccslEncryptWithRotationChecks" "$mqc" | head -1 | cut -d: -f1)
l_dec=$(grep -n "= ccslDecryptWithRotationChecks" "$mqc" | head -1 | cut -d: -f1)
l_recv=$(grep -n "nv_rpctrace_record(NV_RPCTRACE_DIR_GSP_TO_CPU" "$mqc" | head -1 | cut -d: -f1)
[ -n "$l_send" ] && [ -n "$l_enc" ] && [ -n "$l_dec" ] && [ -n "$l_recv" ] \
  || die "could not locate all four anchors in message_queue_cpu.c"
[ "$l_send" -lt "$l_enc" ] \
  || die "SEND HOOK IS AFTER THE ENCRYPT CALL ($l_send >= $l_enc) — it would record CIPHERTEXT"
[ "$l_recv" -gt "$l_dec" ] \
  || die "RECEIVE HOOK IS BEFORE THE DECRYPT CALL ($l_recv <= $l_dec) — it would record CIPHERTEXT"
say "hook placement verified: send@$l_send < encrypt@$l_enc ; decrypt@$l_dec < receive@$l_recv"

# ── 4. Build ──────────────────────────────────────────────────────────────────
say "building with -j$JOBS for $KVER (this takes a few minutes)"
make -C "$BUILD" -j"$JOBS" modules SYSSRC="/lib/modules/$KVER/build" >"$BUILD/build.log" 2>&1 \
  || { tail -40 "$BUILD/build.log"; die "build failed — full log at $BUILD/build.log"; }

ko="$BUILD/kernel-open/nvidia.ko"
[ -f "$ko" ] || die "no nvidia.ko produced"

# ── 5. Assert the thing we built is the thing we meant to build ───────────────
got_ver="$(modinfo -F version "$ko")"
[ "$got_ver" = "$WANT_VERSION" ] || die "built module reports version $got_ver, expected $WANT_VERSION"
got_kver="$(modinfo -F vermagic "$ko" | awk '{print $1}')"
[ "$got_kver" = "$KVER" ] || die "built module vermagic is $got_kver, running kernel is $KVER"

# The recorder must be PRESENT and REFERENCED. A build that silently dropped
# nv_rpctrace.c from NVIDIA_SOURCES would link (the RM's calls would be undefined
# and modpost would fail) — but a build that kept the file and lost the hooks
# would link fine and record nothing, which is the failure worth checking for.
#
# ⚠ MEASURED, 2026-08-03: this was written as `nm "$ko" | grep -q …` and it FAILED
# on a module that contained the symbol. `grep -q` exits at the first match and
# closes the pipe; `nm` on a 100k-symbol module then dies of SIGPIPE; `pipefail`
# turns that into a failed pipeline. The check reported "not in the module" about
# a module the symbol was in — a FALSE RED, which is the polite failure. The same
# construct one line down was a FALSE GREEN waiting to happen: `modinfo -p` is
# short enough to finish before `grep -q` exits, so it passed for a reason that
# had nothing to do with the parameter existing. Both now go through a file.
nm "$ko" >"$BUILD/nvidia.ko.nm"
grep -q " [tT] nv_rpctrace_record\$" "$BUILD/nvidia.ko.nm" \
  || die "nv_rpctrace_record not in the module"
grep -q " [tT] nv_rpctrace_set_outcome\$" "$BUILD/nvidia.ko.nm" \
  || die "nv_rpctrace_set_outcome not in the module"
modinfo -p "$ko" >"$BUILD/nvidia.ko.params"
grep -q "^NVreg_RpcTraceKB:" "$BUILD/nvidia.ko.params" \
  || die "NVreg_RpcTraceKB parameter missing"

mkdir -p "$OUT"
cp "$ko" "$OUT/nvidia.ko"
cp "$BUILD/build.log" "$OUT/build.log"
sha256sum "$OUT/nvidia.ko" | tee "$OUT/nvidia.ko.sha256"

say "OK: $OUT/nvidia.ko  version=$got_ver vermagic=$got_kver"
say "next: sudo $HERE/capture.sh"
