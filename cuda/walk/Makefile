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

kf_bench: kf_bench.cu kf_walk.cu kf_walk.h kf_tables.h
	$(NVCC) $(ARCH) $(CFLAGS) -o $@ kf_bench.cu kf_walk.cu -lpthread

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
	@for f in kf_par_decode_slot kf_par_expand_one kf_par_chunks kf_par_leaf_one; do \
	   if awk "/__device__ __forceinline__ (bool|void) $$f/,/^\}/" kf_walk.cu | grep -q 'while'; then \
	     echo "I1 FAIL: a while loop on the guest-data path in $$f"; exit 1; fi; \
	 done; \
	 echo "I1 ok: no while loop anywhere on the PARALLEL guest-data path"
	@if ! grep -q 'for (uint32_t k = F.first_dir; k < KF_DIRS; k++)' kf_walk.cu; then \
	   echo "I1 FAIL: the parallel walk's DEPTH is not a KF_DIRS-bounded host loop"; exit 1; fi; \
	 echo "I1 ok: the parallel walk's depth is a compile-time-bounded launch loop"
	@n=`grep -cE '< KF_MAX_FRONTIER &&' kf_walk.cu || true`; \
	 if [ "$$n" -lt 6 ]; then echo "I1 FAIL: only $$n parallel loops carry the frontier cap, want >= 6"; exit 1; fi; \
	 echo "I1 ok: $$n parallel loops bounded at compile time by KF_MAX_FRONTIER"
	@if awk '/^__device__ void kf_walk_one/,/^\}/' kf_walk.cu | tail -n +2 | grep -qE '\bgoto\b|kf_walk_one\('; then \
	   echo "I1 FAIL: a goto or a recursive call on the guest-data path"; exit 1; fi; \
	 echo "I1 ok: no goto, no recursion"
	@n=`awk '/^__device__ void kf_walk_one/,/^\}/' kf_walk.cu | grep -cE '< KF_MAX_ENT &&' || true`; \
	 if [ "$$n" -lt 5 ]; then echo "I1 FAIL: only $$n loops bounded by the COMPILE-TIME cap, want >= 5"; exit 1; fi; \
	 echo "I1 ok: $$n nested loops, each bounded at compile time by KF_MAX_ENT"
	@n=`grep -cE '^#define (KF_DIRS|KF_MAX_ENT)' kf_walk.cu || true`; \
	 if [ "$$n" != "2" ]; then echo "I1 FAIL: the nesting bounds are not compile-time constants"; exit 1; fi; \
	 echo "I1 ok: the nesting depth and fan-out caps are #defines, not descriptor fields"
	@if grep -nE '^\s*(#define )?(KF_PD3_N|KF_SH_PD3|VER2_ADDR_|0x3f)' kf_walk.cu | grep -v '^\s*\*'; then \
	   echo "SEAM FAIL: a VER2 geometry constant is still in the kernel"; exit 1; fi; \
	 echo "seam ok: no VER2 geometry constants left in the kernel"
	@if ! grep -q 'KFWR_R_FRONTIER_CAP' kf_walk.cu; then \
	   echo "I3 FAIL: the parallel walk's frontier has no loud cap"; exit 1; fi; \
	 echo "I3 ok: the frontier and task caps are loud"
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

kf_tests_nounmap: kf_tests.cu kf_walk.cu kf_walk.h kf_tables.h
	$(NVCC) $(ARCH) $(CFLAGS) -DKF_DROP_UNMAP -o $@ kf_tests.cu kf_walk.cu -lpthread

check-closure-negative: kf_tests_oldmerge kf_tests_badorder kf_tests_nounmap
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
	@./kf_tests_nounmap roundtrip/ > /tmp/kf_neg5.txt 2>&1; rc=$$?; \
	 if [ $$rc -eq 0 ]; then echo "NEG FAIL: dropping every UNMAP PASSED the round-trip"; exit 1; fi; \
	 if ! grep -q 'apply(model, delta) != full walk' /tmp/kf_neg5.txt; then \
	   echo "NEG FAIL: dropping UNMAPs failed, but NOT on the closure assertion -- wrong reason:"; \
	   grep '^    !' /tmp/kf_neg5.txt | head -3; exit 1; fi; \
	 if grep -q 'reverse order gives a different model' /tmp/kf_neg5.txt; then \
	   echo "NEG FAIL: dropping UNMAPs must NOT disturb order-independence"; exit 1; fi; \
	 echo "neg ok: dropping every UNMAP breaks CLOSURE and nothing else (rc=$$rc), on:"; \
	 grep -m1 'apply(model, delta) != full walk' /tmp/kf_neg5.txt
	@./kf_tests roundtrip/ > /dev/null 2>&1; rc=$$?; \
	 if [ $$rc -ne 0 ]; then echo "NEG FAIL: the CONTROL round-trip does not pass on the real binary"; exit 1; fi; \
	 echo "neg ok: the control round-trip still passes on the unbroken binary"

# ═══ THE SEAM'S OWN KNOWN-POSITIVE ═══════════════════════════════════════════
# "The descriptor is used" is unproven until a WRONG descriptor breaks something.
# Each flag perturbs the setup data by one bit and nothing else; a hardcoded
# fallback anywhere in the decode would keep the suite green through these.
kf_tests_baddesc: kf_tests.cu kf_walk.cu kf_walk.h kf_tables.h
	$(NVCC) $(ARCH) $(CFLAGS) -DKF_BAD_DESCRIPTOR -o $@ kf_tests.cu kf_walk.cu -lpthread

kf_tests_badgeom: kf_tests.cu kf_walk.cu kf_walk.h kf_tables.h
	$(NVCC) $(ARCH) $(CFLAGS) -DKF_BAD_GEOMETRY -o $@ kf_tests.cu kf_walk.cu -lpthread

kf_tests_badabi: kf_tests.cu kf_walk.cu kf_walk.h kf_tables.h
	$(NVCC) $(ARCH) $(CFLAGS) -DKF_BAD_ABI -o $@ kf_tests.cu kf_walk.cu -lpthread

kf_tests_ver3: kf_tests.cu kf_walk.cu kf_walk.h kf_tables.h
	$(NVCC) $(ARCH) $(CFLAGS) -DKF_ALLOW_UNTESTED_VER3 -o $@ kf_tests.cu kf_walk.cu -lpthread

check-seam-negative: kf_tests_baddesc kf_tests_badgeom kf_tests_badabi kf_tests
	@./kf_tests_baddesc correctness/ > /tmp/kf_neg6.txt 2>&1; rc=$$?; \
	 if [ $$rc -eq 0 ]; then echo "SEAM FAIL: a field spec one bit wrong PASSED -- the descriptor is not consulted"; exit 1; fi; \
	 echo "seam ok: addr_local.lo off by one breaks the decode (rc=$$rc), on:"; \
	 grep -m1 '^    !' /tmp/kf_neg6.txt
	@./kf_tests_badgeom correctness/ > /tmp/kf_neg7.txt 2>&1; rc=$$?; \
	 if [ $$rc -eq 0 ]; then echo "SEAM FAIL: a level stride one bit wrong PASSED -- the geometry is not consulted"; exit 1; fi; \
	 echo "seam ok: dir[3].va_lo off by one breaks the walk (rc=$$rc), on:"; \
	 grep -m1 '^    !' /tmp/kf_neg7.txt
	@./kf_tests_badabi correctness/single_4k > /tmp/kf_neg8.txt 2>&1; rc=$$?; \
	 if [ $$rc -eq 0 ]; then echo "SEAM FAIL: an abi_version skew PASSED -- the gate is not there"; exit 1; fi; \
	 if ! grep -q 'format descriptor REFUSED: abi_version' /tmp/kf_neg8.txt; then \
	   echo "SEAM FAIL: the abi skew failed, but NOT at the abi gate -- wrong reason:"; \
	   head -3 /tmp/kf_neg8.txt; exit 1; fi; \
	 echo "seam ok: an abi_version skew is refused AT LAUNCH (rc=$$rc), on:"; \
	 grep -m1 'format descriptor REFUSED' /tmp/kf_neg8.txt
	@./kf_tests correctness/ > /dev/null 2>&1; rc=$$?; \
	 if [ $$rc -ne 0 ]; then echo "SEAM FAIL: the CONTROL does not pass on the real descriptor"; exit 1; fi; \
	 echo "seam ok: the control passes on the real descriptor"

# ⚠ The ONLY thing ever asserted about VER3: that the SKETCHED descriptor is well
# formed. Not that it decodes anything — no VER3 table has ever existed here.
check-ver3-sketch: kf_tests_ver3
	@./kf_tests_ver3 format/refuses_untested_ver3 > /tmp/kf_ver3.txt 2>&1; rc=$$?; \
	 if [ $$rc -ne 0 ]; then echo "VER3 FAIL: the sketched descriptor does not pass kf_format_check:"; \
	   cat /tmp/kf_ver3.txt; exit 1; fi; \
	 echo "ver3 ok: the SKETCH is well formed (geometry only -- it has never decoded a table)"

# No SASS at all ⇒ the driver JITs from PTX at load time.
check-ptx: kf_tests
	@if cuobjdump -sass kf_tests 2>/dev/null | grep -q 'code for sm_'; then \
	   echo "JIT FAIL: the binary contains SASS; it is not PTX-only"; exit 1; fi; \
	 echo "jit ok: PTX-only binary"
	@cuobjdump -ptx kf_tests | grep -m1 '.target' || true

# ★★★★★ THE ARTIFACT KAYFABE SHIPS — `crates/kayfabe-cuda` embeds this file.
#
# ⊘ NVRTC, not nvcc: it is the one CUDA front end that runs with **no GPU and no toolkit**, so
# the PTX is generated where the rest of the tree is generated rather than on a rented box.
# `make_ptx.py` drives `libnvrtc.so.12` through ctypes and supplies the two libc headers NVRTC
# does not ship. `KF_DEVICE_ONLY` selects the half it can compile — see kf_walk.cu's own note.
#
# ⚠ REGENERATE AFTER ANY EDIT TO kf_walk.cu OR kf_walk.h. The Rust side has three independent
# checks that catch a stale PTX — the `.param` size, the entry names and the `.target` — but
# only if somebody runs them; regenerating is cheaper than diagnosing.
kf_walk.ptx: kf_walk.cu kf_walk.h
	python3 make_ptx.py kf_walk.cu kf_walk.ptx compute_75

.PHONY: ptx
ptx: kf_walk.ptx

check: check-invariants kf_tests check-ptx check-negative check-closure-negative check-seam-negative check-ver3-sketch

clean:
	rm -f kf_tests kf_tests_broken kf_tests_oldmerge kf_tests_badorder kf_tests_nounmap \
	      kf_tests_baddesc kf_tests_badgeom kf_tests_badabi kf_tests_ver3

.PHONY: all ptx check check-invariants check-ptx check-negative check-closure-negative check-seam-negative clean
