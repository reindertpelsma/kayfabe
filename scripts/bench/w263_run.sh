#!/usr/bin/env bash
# w263 — leg B. Build at HEAD, assert the stamp, then two boots (off / ring).
# ★ START marker and EXIT line so "file exists but has no terminator" is detectable.
OUT=/workspace/w263_run.log
exec >"$OUT" 2>&1
finish() { echo "=== W263 EXIT rc=$1 at $(date -Is) ==="; exit "$1"; }
echo "=== W263 START $(date -Is) pid=$$ ==="

export PATH=/root/.cargo/bin:$PATH
REPO=/workspace/kayfabe_w263
cd "$REPO" || finish 90
HEAD=$(git rev-parse HEAD)
echo "=== HEAD=$HEAD ==="
DIRT=$(git status --porcelain --untracked-files=no)
[ -z "$DIRT" ] || { echo "=== ★ TREE IS DIRTY ==="; echo "$DIRT"; finish 91; }

export CARGO_TARGET_DIR=/workspace/bench/cargo-target-w263
export KAYFABE_SHIM_FEATURES=host-isolates
echo "=== BUILD START $(date -Is) ==="
scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build
BRC=$?
echo "=== BUILD RC=$BRC $(date -Is) ==="
[ $BRC -eq 0 ] || finish 92

# ★★★ THE STAMP GATE
STAMP=$(strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -o 'kayfabe-rev:[0-9a-f]*\(-dirty\)\?' | sort -u)
echo "=== STAMP: [$STAMP] WANT: [kayfabe-rev:$HEAD] ==="
if [ "$STAMP" != "kayfabe-rev:$HEAD" ]; then
  echo "=== ★★★ STAMP MISMATCH — REFUSING TO BOOT. ==="
  finish 93
fi
echo "=== STAMP GATE: PASS ==="
echo "kayfabe-rev:$HEAD" > /workspace/bench/BUILD_REV.txt

# ★ CONTENT CHECK: the changed code is actually IN this binary. A stamp says which
#   revision; these say which CODE. ⊘ zero for any of them means the feature was
#   compiled out (host-isolates) and the boot would measure a build I did not write.
echo "=== CONTENT CHECK ==="
for s in "GUEST-USERD" "USERD_NOT_A_JOINED_WINDOW" "phys=UNREADABLE" "fbuserd@" "guest_userd=" "the guest's OWN KERNEL resolved"; do
  printf '  %-34s = %s\n' "$s" "$(strings /workspace/bench/qemu-build/qemu-system-x86_64 | grep -c -- "$s")"
done

export KAYFABE_ISOLATES=real
export KAYFABE_CE_EXECUTOR=host
export NVKVM_RAM_BACKEND=memfd
export KAYFABE_GUEST_RAM=memfd
export KAYFABE_FB_JOIN=shared
export POST_CAPTURE_HOOK=$REPO/scripts/bench/cup2_hook_w232.sh
export GQ_TIMEOUT=300
export BOOT_TIMEOUT=180

boot() {
  local tag=$1 arm=$2
  echo "=== BOOT $tag arm=$arm START $(date -Is) ==="
  if [ "$arm" = "off" ]; then unset KAYFABE_GUEST_RING; else export KAYFABE_GUEST_RING=$arm; fi
  echo "KAYFABE_GUEST_RING=[${KAYFABE_GUEST_RING:-unset}] KAYFABE_FB_JOIN=[$KAYFABE_FB_JOIN]"
  timeout 900 "$REPO/scripts/bench/boot_capture.sh" "$tag"
  echo "=== BOOT $tag RC=$? $(date -Is) ==="
  # ⊘ pgrep -x qemu-system-x86 (comm truncates at 15); NOT -f (matches the asker)
  echo "--- post-boot liveness: pgrep -x qemu-system-x86 = [$(pgrep -x qemu-system-x86 | tr '\n' ' ')] ss2223 = [$(ss -tln 2>/dev/null | grep -c 2223)]"
  echo "--- ENOSPC_LLVM=$(grep -c 'No space left on device\|LLVM ERROR' /workspace/bench/run_${tag}_qemu.log 2>/dev/null || echo '?')"
}

boot w263_off  off
boot w263_ring ring

echo "=== ARTEFACT SIZES ==="
ls -l /workspace/bench/run_w263_* 2>/dev/null
finish 0
