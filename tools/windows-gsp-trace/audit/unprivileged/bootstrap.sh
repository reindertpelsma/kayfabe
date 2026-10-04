set -eu
mkdir -p /opt/kf-unpriv/evidence
chmod 755 /opt/kf-unpriv
chown 65534:65534 /opt/kf-unpriv/evidence
cd /opt/kf-unpriv
cc -O2 -Wall -Wextra -Werror -o exec_unprivileged exec_unprivileged.c
cc -shared -fPIC -O2 -o nvdiff.so nvdiff_shim.c -ldl
cc -O2 -o cuinit_probe cuinit_probe.c -ldl
cc -O2 -o vector_add_test vector_add_test.c -ldl
sha256sum *.c *.h *.bt > evidence/input_sha256.txt
cat /proc/driver/nvidia/version > evidence/closed-driver.txt
nvidia-smi -q > evidence/closed-gpu-root-inventory.txt
uname -a > evidence/kernel.txt
for test in smi cuda vector; do
 case "$test" in smi) set -- nvidia-smi -q;; cuda) set -- /opt/kf-unpriv/cuinit_probe;; vector) set -- /opt/kf-unpriv/vector_add_test;; esac
 setpriv --reuid=65534 --regid=65534 --clear-groups --bounding-set=-all --inh-caps=-all --ambient-caps=-all --no-new-privs env AUDIT_PRELOAD=/opt/kf-unpriv/nvdiff.so NVDIFF_OUT=/opt/kf-unpriv/evidence/closed-${test}-ioctl.jsonl /opt/kf-unpriv/exec_unprivileged "$@" > evidence/closed-${test}.out 2> evidence/closed-${test}.posture
 echo "CLOSED_TEST $test rc=$?"
done
