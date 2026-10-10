#!/usr/bin/env bash
# POST_CAPTURE_HOOK (outside the repo): run archive/tests/guest/validate.sh INSIDE the fat guest.
R=/var/lib/kf-windows-20261005/kayfabe-linuxreg
G=$R/scripts/bench/gssh_nv
SRC=$R/archive/tests/guest/validate.sh
echo "VALIDATE_START $(date -Is) src_md5=$(md5sum < $SRC | cut -d' ' -f1) kayfabe_rev=$(git -C $R rev-parse --short=8 HEAD)"
echo "--- guest tooling"
$G 'uname -r; nvidia-smi --query-gpu=name,driver_version --format=csv,noheader; which gcc cc python3 vulkaninfo eglinfo 2>&1; ls /usr/lib/x86_64-linux-gnu | grep -E "libvulkan|libEGL_nvidia|libGLX_nvidia" | head -8' 2>&1
$G 'cat > /tmp/validate.sh' < $SRC || { echo "VALIDATE_PUSH_FAILED"; exit 2; }
$G 'chmod +x /tmp/validate.sh; rm -f /tmp/validate.json; cd /tmp && timeout 900 bash /tmp/validate.sh --expect-gpu "RTX 4070" --expect-driver 595.84 --json /tmp/validate.json > /tmp/validate.out 2>&1; echo $? > /tmp/validate.rc'
echo "VALIDATE_RC=$($G 'cat /tmp/validate.rc' 2>&1)"
$G 'cat /tmp/validate.out' 2>&1
echo "--- json"
$G 'cat /tmp/validate.json' 2>&1 | head -c 6000
echo "VALIDATE_END $(date -Is)"
