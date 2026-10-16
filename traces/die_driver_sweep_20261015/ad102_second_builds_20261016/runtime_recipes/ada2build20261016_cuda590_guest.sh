
set -euo pipefail
cd ~/codex590
printf 'ACTUAL_GUEST_DRIVER='; nvidia-smi --query-gpu=driver_version --format=csv,noheader
test "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" = 590.48.01
test "$(modinfo -F version nvidia)" = 590.48.01
echo "ACTUAL_GUEST_KERNEL=$(uname -r)"
cuda=$(readlink -f /usr/lib/x86_64-linux-gnu/libcuda.so.1)
echo "ACTUAL_LIBCUDA=$cuda"
test "${cuda##*.so.}" = 590.48.01
I=-Iscripts/bench/cuda_min; L=/usr/lib/x86_64-linux-gnu/libcuda.so.1
for rung in cup2 cup3 cup8 cup8bench; do
 case "$rung" in cup2) src=archive/nvkvm/tests/mode2/cup2.c;; *) src=scripts/bench/$rung.c;; esac
 gcc -O2 "$I" -o "$rung" "$src" "$L" -lm
 echo "CUDA_RUNG_START=$rung $(date -u -Is)"
 set +e
 BENCH_SIZES=16,2048 BENCH_ITERS=20 BENCH_BATCH=10 timeout 180 "./$rung" >"$rung.log" 2>&1
 rc=$?
 set -e
 cat "$rung.log"
 echo "CUDA_RUNG_EXIT=$rung rc=$rc $(date -u -Is)"
 test "$rc" = 0
 case "$rung" in
 cup2) grep -q 'CE rv=0xabcd1234 want=0xabcd1234 -> PASS' "$rung.log";;
 cup3) grep -q 'KERNEL rv=43 want=43 -> PASS' "$rung.log";;
 cup8) grep -q 'CUP8 RESULT N=2048 bad=0 maxerr=0' "$rung.log";;
 cup8bench) grep -q '^BENCH_TOTAL_BAD=0$' "$rung.log";grep -q '^BENCH_VERDICT: PASS' "$rung.log";;
 esac
 echo "CUDA_CELL rung=$rung verdict=PASS"
done
echo CUDA590_LADDER=PASS
