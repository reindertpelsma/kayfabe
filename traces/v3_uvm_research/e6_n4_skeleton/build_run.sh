#!/usr/bin/env bash
set -uo pipefail
exec >/root/kf_run.log 2>&1
UUID="$1"; STAGEC="${2:-0}"
echo "KF_RUN_START $(date -Is)  stage_c=$STAGEC"
cd /root/kf_uvm || { echo "no module dir"; echo "KF_RUN_DONE rc=9"; exit 9; }

VER=$(head -1 /proc/driver/nvidia/version 2>/dev/null); echo "version: $VER"
echo "$VER" | grep -q "580.159.04" && echo "V580159=yes" || echo "V580159=NO"
echo "$VER" | grep -q "Open Kernel Module" && echo "OPEN=yes" || echo "OPEN=NO"

NV_SYMVERS=$(find /var/lib/dkms/nvidia/580.159.04 -name Module.symvers 2>/dev/null | xargs -r grep -l nvUvmInterfaceRegisterGpu 2>/dev/null | head -1)
echo "NV_SYMVERS=$NV_SYMVERS"
[ -z "$NV_SYMVERS" ] && { echo "NO_SYMVERS"; echo "KF_RUN_DONE rc=8"; exit 8; }

mkdir -p /root/kf_uvm/inc
for h in nv_uvm_types.h nvtypes.h cpuopsys.h nvstatus.h nvstatuscodes.h nvgputypes.h nvCpuUuid.h nv_uvm_user_types.h; do
  f=$(find /usr/src/nvidia-580.159.04 -name "$h" 2>/dev/null | head -1); [ -n "$f" ] && cp "$f" /root/kf_uvm/inc/
done

# ensure our module is not loaded, nvidia loaded, nvidia_uvm UNLOADED
rmmod kf_uvm_probe 2>/dev/null || true
modprobe nvidia 2>&1 | tail -1
nvidia-smi >/dev/null 2>&1
rmmod nvidia_uvm 2>/dev/null || true
echo "modules(nvidia): $(cat /proc/modules | grep -iE '^nvidia' | awk '{print $1}' | tr '\n' ' ')"
echo "nvidia_uvm loaded = $(grep -c '^nvidia_uvm ' /proc/modules)  (MUST be 0)"
echo "nvidia loaded = $(grep -c '^nvidia ' /proc/modules)  (MUST be 1)"

echo "--- build ---"
make -C /root/kf_uvm clean >/dev/null 2>&1 || true
make -C /root/kf_uvm NV_SYMVERS="$NV_SYMVERS" 2>&1 | egrep -i "error|warning: .*undefined|CC |LD |MODPOST|Module.symvers|No rule" | tail -20
[ -f /root/kf_uvm/kf_uvm_probe.ko ] || { echo "BUILD_FAILED"; echo "KF_RUN_DONE rc=7"; exit 7; }
echo "built: $(ls -la /root/kf_uvm/kf_uvm_probe.ko | awk '{print $5}') bytes"

echo "--- insmod (gpu_uuid=$UUID do_stage_c=$STAGEC) ---"
dmesg -C 2>/dev/null || true
insmod /root/kf_uvm/kf_uvm_probe.ko gpu_uuid="$UUID" do_stage_c="$STAGEC"
echo "insmod_rc=$?"

# trigger helper: hold /dev/nvidia0 open, write /proc/kf_uvm_trigger
cat > /tmp/trig.c <<'CEOF'
#include <fcntl.h>
#include <unistd.h>
#include <stdio.h>
#include <errno.h>
#include <string.h>
int main(void){
  int nv=open("/dev/nvidia0",O_RDWR);
  printf("open /dev/nvidia0 fd=%d errno=%d(%s)\n",nv,errno,strerror(errno));
  int p=open("/proc/kf_uvm_trigger",O_WRONLY);
  printf("open trigger fd=%d errno=%d(%s)\n",p,errno,strerror(errno));
  if(p>=0){ ssize_t w=write(p,"go",2); printf("write rc=%zd errno=%d\n",w,errno); }
  if(p>=0)close(p);
  if(nv>=0)close(nv);
  return 0;
}
CEOF
gcc -o /tmp/trig /tmp/trig.c && echo "--- trigger ---" && /tmp/trig
sleep 3
echo "=== DMESG kf_uvm BEGIN ==="
dmesg | grep kf_uvm || echo "(no kf_uvm lines)"
echo "=== DMESG kf_uvm END ==="
echo "KF_RUN_DONE rc=0 $(date -Is)"
