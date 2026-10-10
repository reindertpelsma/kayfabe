#!/bin/bash
# B leg v2. Fail FAST if the checkout does not actually land the fix, instead
# of rebuilding the old code and reporting a meaningless result.
cd /opt/nvkvm-steamos-latest
S=./steamos-ssh
{
echo "### fetch with an explicit refspec (a bare 'fetch origin <branch>' only moved FETCH_HEAD)"
git fetch origin '+refs/heads/*:refs/remotes/origin/*' 2>&1 | tail -3
git checkout -B verify-libcuda-fix origin/fix/steamos-profile-keeps-libcuda 2>&1 | tail -3
git log --oneline -1
echo "### GUARD: the fix must be in the tree"
grep -n "^TRIM_RE=" boot/steamos_boot.sh
if grep -q "^TRIM_RE='libcuda|" boot/steamos_boot.sh; then echo "ABORT: still the old TRIM_RE"; echo "### B2-DONE"; exit 1; fi
n=$(grep -c "libcuda.so.1" boot/steamos_boot.sh)
echo "libcuda.so.1 references: $n"
[ "$n" -lt 1 ] && { echo "ABORT: sentinel missing"; echo "### B2-DONE"; exit 1; }
echo "GUARD PASSED"
echo
echo "### guest still broken before we start?"
$S "ldconfig -p | grep -c libcuda" 2>&1 | tail -2
echo
echo "### rebuild + restart"
docker compose build 2>&1 | tail -3
docker compose down 2>&1 | tail -1
docker compose up -d 2>&1 | tail -3
echo "### confirm the GUEST now sees the fixed script"
for i in $(seq 1 80); do $S true >/dev/null 2>&1 && { echo "guest up after $((i*15))s"; break; }; sleep 15; done
$S "grep -n '^TRIM_RE=' /run/nvkvm/boot/steamos_boot.sh" 2>&1 | tail -2
echo
echo "### waiting for converge to repair (downloads a 400MB runfile)"
for i in $(seq 1 45); do
  n=$($S "ldconfig -p | grep -c libcuda" 2>/dev/null | tr -dc 0-9)
  [ -n "$n" ] && [ "$n" -gt 0 ] && { echo "libcuda back after ~$((i*20))s"; break; }
  sleep 20
done
echo
echo "### RESULT"
$S "ldconfig -p | grep libcuda; ls -la /usr/lib/libcuda.so.* /usr/lib32/libcuda.so.* 2>/dev/null"
$S "sudo -n journalctl -u nvkvm-boot.service --no-pager -n 80 2>/dev/null | grep -iE 'INCOMPLETE|libcuda|trimm|restored|Part 1 finished|checks passed|matches host'" 2>&1 | tail -12
echo "### B2-DONE"
} > /root/pc-b2.log 2>&1
