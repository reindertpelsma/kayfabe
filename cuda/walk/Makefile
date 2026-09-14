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

# ═══ THE KNOWN-POSITIVE ══════════════════════════════════════════════════════
# -DKF_BREAK_BOUNDS deletes the two bounds checks and NOTHING else. The bounds
# cases must then fail. A check nobody has ever seen fire is not a check.
kf_tests_broken: kf_tests.cu kf_walk.cu kf_walk.h kf_tables.h
	$(NVCC) $(ARCH) $(CFLAGS) -DKF_BREAK_BOUNDS -o $@ kf_tests.cu kf_walk.cu -lpthread

check-negative: kf_tests_broken kf_tests
	@./kf_tests_broken bounds_window_respected > /tmp/kf_neg1.txt 2>&1; rc=$$?; \
	 if [ $$rc -eq 0 ]; then echo "NEG FAIL: bounds_window_respected PASSED with the bounds check removed"; \
	   cat /tmp/kf_neg1.txt; exit 1; fi; \
	 echo "neg ok: bounds_window_respected fails without the check (rc=$$rc)"
	@./kf_tests_broken pointer_past_end > /tmp/kf_neg2.txt 2>&1; rc=$$?; \
	 if [ $$rc -eq 0 ]; then echo "NEG FAIL: pointer_past_end PASSED with the bounds check removed"; \
	   cat /tmp/kf_neg2.txt; exit 1; fi; \
	 echo "neg ok: pointer_past_end fails without the check (rc=$$rc)"
	@./kf_tests correctness/single_4k > /dev/null 2>&1; rc=$$?; \
	 if [ $$rc -ne 0 ]; then echo "NEG FAIL: the CONTROL case does not pass on the unbroken binary"; exit 1; fi; \
	 echo "neg ok: the control case still passes on the unbroken binary"

# Two more known-positives, for the CLOSURE property. Each restores exactly one
# superseded decision and nothing else; the round-trip cases must then fail, and
# must fail with the CLOSURE message rather than some downstream symptom.
kf_tests_oldmerge: kf_tests.cu kf_walk.cu kf_walk.h kf_tables.h
	$(NVCC) $(ARCH) $(CFLAGS) -DKF_OLD_MERGE -o $@ kf_tests.cu kf_walk.cu -lpthread

kf_tests_badorder: kf_tests.cu kf_walk.cu kf_walk.h kf_tables.h
	$(NVCC) $(ARCH) $(CFLAGS) -DKF_BREAK_ORDER -o $@ kf_tests.cu kf_walk.cu -lpthread

check-closure-negative: kf_tests_oldmerge kf_tests_badorder
	@./kf_tests_oldmerge roundtrip/ > /tmp/kf_neg3.txt 2>&1; rc=$$?; \
	 if [ $$rc -eq 0 ]; then echo "NEG FAIL: the superseded whole-run merge PASSED the round-trip"; exit 1; fi; \
	 if ! grep -qE 'apply\(model, delta\) != full walk|reverse order gives a different model' /tmp/kf_neg3.txt; then \
	   echo "NEG FAIL: the old merge failed, but NOT on a closure-family assertion -- wrong reason:"; \
	   grep '^    !' /tmp/kf_neg3.txt | head -3; exit 1; fi; \
	 echo "neg ok: the superseded whole-run merge breaks the round-trip (rc=$$rc), on:"; \
	 grep -m1 -E 'apply\(model, delta\) != full walk|reverse order gives a different model' /tmp/kf_neg3.txt
	@./kf_tests_badorder roundtrip/ > /tmp/kf_neg4.txt 2>&1; rc=$$?; \
	 if [ $$rc -eq 0 ]; then echo "NEG FAIL: a class emitted DESCENDING in VA PASSED the round-trip"; exit 1; fi; \
	 if ! grep -qE 'apply\(model, delta\) != full walk|reverse order gives a different model' /tmp/kf_neg4.txt; then \
	   echo "NEG FAIL: the bad walk order failed, but NOT on a closure-family assertion -- wrong reason:"; \
	   grep '^    !' /tmp/kf_neg4.txt | head -3; exit 1; fi; \
	 echo "neg ok: a class emitted out of VA order breaks the round-trip (rc=$$rc), on:"; \
	 grep -m1 -E 'apply\(model, delta\) != full walk|reverse order gives a different model' /tmp/kf_neg4.txt
	@./kf_tests roundtrip/ > /dev/null 2>&1; rc=$$?; \
	 if [ $$rc -ne 0 ]; then echo "NEG FAIL: the CONTROL round-trip does not pass on the real binary"; exit 1; fi; \
	 echo "neg ok: the control round-trip still passes on the unbroken binary"

# No SASS at all ⇒ the driver JITs from PTX at load time.
check-ptx: kf_tests
	@if cuobjdump -sass kf_tests 2>/dev/null | grep -q 'code for sm_'; then \
	   echo "JIT FAIL: the binary contains SASS; it is not PTX-only"; exit 1; fi; \
	 echo "jit ok: PTX-only binary"
	@cuobjdump -ptx kf_tests | grep -m1 '.target' || true

check: check-invariants kf_tests check-ptx check-negative check-closure-negative

clean:
	rm -f kf_tests kf_tests_broken kf_tests_oldmerge kf_tests_badorder

.PHONY: all check check-invariants check-ptx check-negative check-closure-negative clean
