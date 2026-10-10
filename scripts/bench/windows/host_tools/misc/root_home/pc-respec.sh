#!/bin/bash
# Restart the SteamOS guest with more CPU and RAM, to test whether the RDR2
# average was CPU-limited by the guest's 8 vCPUs rather than by forwarding.
cd /opt/nvkvm-steamos-latest
{
echo "### host has"; nproc; free -g | awk 'NR==2{print $2" GB total, "$7" GB available"}'
echo "### before"; grep -E "VM_MEM|VM_SMP" docker-compose.yml | head -4
docker compose down 2>&1 | tail -2
export STEAMOS_VM_SMP=22
export STEAMOS_VM_MEM=18G
echo "### starting with STEAMOS_VM_SMP=$STEAMOS_VM_SMP STEAMOS_VM_MEM=$STEAMOS_VM_MEM"
docker compose -f docker-compose.yml -f override-dri.yml up -d 2>&1 | tail -4
for i in $(seq 1 60); do ./steamos-ssh true >/dev/null 2>&1 && { echo "guest up after $((i*15))s"; break; }; sleep 15; done
echo "### guest now sees"
./steamos-ssh "nproc; free -g | awk '\''NR==2{print \$2\" GB\"}'\''" 2>&1 | tail -3
echo "### qemu command line, as proof"
docker compose exec -T vmm sh -c "tr '\0' ' ' < /proc/\$(pgrep -f qemu-system | head -1)/cmdline" 2>/dev/null | tr ' ' '\n' | grep -A1 -E "^-smp|^-m$" | head -6
echo "### RESPEC-DONE"
} > /root/respec.log 2>&1
