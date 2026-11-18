#!/bin/bash
# Which -cpu strings does QEMU (kf3 build) + KVM accept on this Intel host? Starts paused (-S), no guest runs. Output = stderr of qemu + exit status.
Q=${1:?qemu path}
HVF="hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,hv-stimer,hv-crash"
RM="-avx512f,-rtm,-hle,-mpx,-sgx,-vmx,-svm"
EP="EPYC-v1,model-id=AMD Virtual CPU,family=23,model=1,stepping=2"
i=0
while IFS= read -r A; do
  [ -z "$A" ] && continue; i=$((i+1))
  echo "=== [$i] -cpu $A"
  rm -f /tmp/q$i.sock
  timeout 15 "$Q" -accel kvm -machine q35 -m 256 -nodefaults -display none -S -cpu "$A" -qmp unix:/tmp/q$i.sock,server=on,wait=off > /tmp/q$i.out 2>&1 &
  P=$!
  sleep 3
  if kill -0 $P 2>/dev/null; then echo "RESULT started (paused) OK"; kill $P 2>/dev/null; else wait $P; echo "RESULT exited rc=$?"; fi
  wait $P 2>/dev/null
  cut -c1-300 /tmp/q$i.out | head -12
done <<LIST
$EP,$RM,$HVF
$EP,enforce
host,vendor=AuthenticAMD,family=23,model=1,stepping=2,$HVF
host,vendor=AuthenticAMD,family=23,model=1,stepping=2,-avx512f,-rtm,-hle,-mpx,-sgx,-vmx,$HVF
LIST
