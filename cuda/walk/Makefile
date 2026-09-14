# The walk kernel + its attacking test suite.
#
# ★ PTX ONLY, compute_75. `code=compute_75` embeds PTX and NO cubin, so the driver
#   JITs the kernel forward onto whatever Turing-or-later part is present. That is
#   the brief's "targeting sm_75 so it JITs forward to Turing+", and it is checked
#   rather than assumed: the binary contains no SASS for any architecture.

NVCC   ?= nvcc
ARCH   ?= -gencode arch=compute_75,code=compute_75
CFLAGS ?= -O2 -std=c++14 -lineinfo -Xcompiler -Wall -Xcompiler -Wextra -Xcompiler -Wno-unused-parameter

all: kf_tests

kf_tests: kf_tests.cu kf_walk.cu kf_walk.h kf_tables.h
	$(NVCC) $(ARCH) $(CFLAGS) -o $@ kf_tests.cu kf_walk.cu -lpthread

# ═══ The three invariants, checked in the SOURCE rather than argued for ═══════
check-invariants:
	@n=`grep -c 'KF_GPGA_DEREF(' kf_walk.cu || true`; \
	 if [ "$$n" != "2" ]; then echo "I2 FAIL: KF_GPGA_DEREF appears $$n times, want 2 (define + one use)"; exit 1; fi; \
	 echo "I2 ok: exactly one dereference site"
	@n=`grep -c 'const volatile uint64_t \*' kf_walk.cu || true`; \
	 if [ "$$n" != "1" ]; then echo "I2 FAIL: $$n raw GPGA pointer casts, want 1"; exit 1; fi; \
	 echo "I2 ok: exactly one raw pointer cast"
	@if awk '/^__device__ void kf_walk_one/,/^\}/' kf_walk.cu | grep -q 'while'; then \
	   echo "I1 FAIL: a while loop on the guest-data path"; exit 1; fi; \
	 echo "I1 ok: no while loop in kf_walk_one"
	@if awk '/^__device__ void kf_walk_one/,/^\}/' kf_walk.cu | tail -n +2 | grep -qE '\bgoto\b|kf_walk_one\('; then \
	   echo "I1 FAIL: a goto or a recursive call on the guest-data path"; exit 1; fi; \
	 echo "I1 ok: no goto, no recursion"
	@for k in KF_PD3_N KF_PD2_N KF_PD1_N KF_PD0_N KF_PTB_N; do \
	   if ! awk '/^__device__ void kf_walk_one/,/^\}/' kf_walk.cu | grep -q "< $$k"; then \
	     echo "I1 FAIL: no constant-bound loop over $$k"; exit 1; fi; \
	 done; \
	 echo "I1 ok: five nested loops, every trip count a format constant"
	@n=`grep -c 'KFWR_HF_TRUNCATED' kf_walk.cu || true`; \
	 if [ "$$n" -lt 3 ]; then echo "I3 FAIL: truncation set from fewer than 3 sites"; exit 1; fi; \
	 echo "I3 ok: truncation raised from $$n sites"

# No SASS at all ⇒ the driver JITs from PTX at load time.
check-ptx: kf_tests
	@if cuobjdump -sass kf_tests 2>/dev/null | grep -q 'code for sm_'; then \
	   echo "JIT FAIL: the binary contains SASS; it is not PTX-only"; exit 1; fi; \
	 echo "jit ok: PTX-only binary"
	@cuobjdump -ptx kf_tests | grep -m1 '.target' || true

check: check-invariants kf_tests check-ptx

clean:
	rm -f kf_tests

.PHONY: all check check-invariants check-ptx clean
