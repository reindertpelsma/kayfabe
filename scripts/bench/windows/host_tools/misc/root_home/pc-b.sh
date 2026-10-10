#!/bin/bash
# B leg: same guest, same broken state (no libcuda anywhere), now under the
# FIX. Converge must notice INCOMPLETE and repair it in place, unattended.
cd /opt/nvkvm-steamos-latest
S=./steamos-ssh
{
echo "### switching to the fix branch"
git fetch -q origin fix/steamos-profile-keeps-libcuda 2>&1 | tail -2
git checkout -q -B verify-libcuda-fix origin/fix/steamos-profile-keeps-libcuda 2>&1 | tail -2
git log --oneline -1
grep -n "^TRIM_RE=" boot/steamos_boot.sh
grep -c "libcuda.so.1" boot/steamos_boot.sh
echo
echo "### confirm the guest is still broken before we start"
$S "ldconfig -p | grep -c libcuda" 2>&1 | tail -2
echo
echo "### rebuild + restart"
docker compose build 2>&1 | tail -4
docker compose down 2>&1 | tail -2
docker compose up -d 2>&1 | tail -4
echo "### waiting for the guest, and for converge to finish repairing"
for i in $(seq 1 80); do $S true >/dev/null 2>&1 && { echo "guest up after $((i*15))s"; break; }; sleep 15; done
sleep 30
for i in $(seq 1 40); do
  n=$($S "ldconfig -p | grep -c libcuda" 2>/dev/null | tr -dc 0-9)
  [ -n "$n" ] && [ "$n" -gt 0 ] && { echo "libcuda back after ~$((i*20))s of converge"; break; }
  sleep 20
done
echo
echo "### AFTER -- did it repair itself?"
$S "ldconfig -p | grep libcuda; ls -la /usr/lib/libcuda.so.* /usr/lib32/libcuda.so.* /usr/local/nvidia-guest/lib/libcuda.so.* 2>/dev/null"
echo "### converge journal"
$S "sudo -n journalctl -u nvkvm-boot.service --no-pager -n 60 2>/dev/null | grep -iE 'INCOMPLETE|libcuda|trimm|restored|Part 1 finished|checks passed'" 2>&1 | tail -12
echo "### B-DONE"
} > /root/pc-b.log 2>&1
