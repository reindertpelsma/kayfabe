#!/bin/bash
# Mode-2 Phase-B starting-line probe: how far does real CUDA get through the
# control-plane+CE emulator?  The CUDA process itself holds /dev/nvidia0 open.
set +e
KO="$HOME/nvmods/nvidia.ko"
sudo systemctl isolate multi-user.target 2>/dev/null; sleep 2
sudo rmmod nvidia 2>/dev/null
sudo modprobe ecdh_generic ecc 2>/dev/null
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null
sudo mknod /dev/nvidiactl c 195 255 2>/dev/null
sudo mknod /dev/nvidia-uvm c 235 0 2>/dev/null
sudo chmod 666 /dev/nvidia* 2>/dev/null
sudo dmesg -C
sudo insmod "$KO" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1
sudo insmod "$HOME/nvmods/nvidia-uvm.ko" 2>&1 | tail -1
sudo mknod /dev/nvidia-uvm c 235 0 2>/dev/null
sudo mknod /dev/nvidia-uvm-tools c 235 1 2>/dev/null
sudo chmod 666 /dev/nvidia-uvm* 2>/dev/null
echo "uvm loaded: $(lsmod | grep -c nvidia_uvm)"
sleep 2
cat > /tmp/cup.c <<'EOF'
#include <cuda.h>
#include <stdio.h>
#define CK(x) do{ CUresult r=(x); if(r!=CUDA_SUCCESS){const char*s=0;cuGetErrorString(r,&s);printf("FAIL %s -> %s (%d)\n",#x,s?s:"?",r);fflush(stdout);return 1;} else printf("ok   %s\n",#x);fflush(stdout);}while(0)
int main(){
  CK(cuInit(0));
  int n=0; CK(cuDeviceGetCount(&n)); printf("devices=%d\n",n); if(n<1)return 1;
  CUdevice d; CK(cuDeviceGet(&d,0));
  char nm[256]={0}; CK(cuDeviceGetName(nm,256,d)); printf("name=%s\n",nm);
  size_t tot=0; cuDeviceTotalMem(&tot,d); printf("totalMem=%zu MiB\n",tot>>20);
  CUcontext ctx; CK(cuCtxCreate(&ctx,0,d));
  CUdeviceptr dp; CK(cuMemAlloc(&dp,4096));
  unsigned hv=0xabcd1234, rv=0;
  CK(cuMemcpyHtoD(dp,&hv,4));
  CK(cuMemcpyDtoH(&rv,dp,4));
  printf("CE-roundtrip rv=0x%x want=0x%x -> %s\n", rv, hv, rv==hv?"PASS":"MISMATCH");
  printf("DONE\n");
  return 0;
}
EOF
nvcc -o /tmp/cup /tmp/cup.c -lcuda 2>&1 | head -5
echo "=== run CUDA probe (timeout 40) ==="
sudo timeout 40 /tmp/cup; echo "exit=$?"
echo "=== dmesg NVRM tail ==="
sudo dmesg | grep -aiE "NVRM|RmInitAdapter|fault|Xid|fail" | tail -8
