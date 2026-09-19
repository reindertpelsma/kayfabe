# The scrub is the last thing still executed on the CPU

**STATUS: OPEN — awaiting an owner ruling on §46 vs §12.26.** Measured 2026-09-20 (w813) on a
GA106 / 580.159.04 guest, branch `w749-fable-legb`. Nothing here has been implemented; the
`SystemDataPlane` rule is untouched.

## What is measured

The `DOORBELL-LEDGER`, per token, on a thin-guest run whose every ledger row verified:

| token | class | emulated | forwarded | reading (the ledger's own rule) |
|---|---|---:|---:|---|
| `0x00000003`–`0x00000007` | the guest's CE channels | 1 | **5** | reached hardware |
| `0x00000009` | guest | 2 | **2** | reached hardware |
| `0x00010001` | kernel / system | **74** | **0** | never reached hardware |
| `0x00010004` | kernel / system | **66** | **0** | never reached hardware |

> *"`forwarded=0` with `emulated>0` means it never went to hardware; `forwarded>0` means it
> did"* — the ledger's own text.

⇒ **The guest's copy work runs on the GPU. The kernel's does not, and it is 140 doorbells.**

## Why this is a decision and not a bug

The kernel tokens are `forwarded=0` **by design today**. `kayfabe_fwd::FwdFault::SystemDataPlane`
implements `l1_concurrency.md` §12.26:

> *"The SYSTEM proc has no data plane … Guest-kernel work that would need a backing — **the
> CeUtils scrub**, the GR golden capture — is **forged** to the system proc's completion queue,
> **never forwarded**, so the system proc never mints host memory."*

★ §46 (owner, 2026-09-19) names that **exact** example and reverses it:

> *"okay no real work kernel channel except the no-op or the ones that we must stub, so the
> scrub and CE, is never executed on CPU. Its always executed on GPU under the single store"*
> and, when I mis-paraphrased it: *"Ceutils scrub is not a no-op, it genuinely needs to clear
> memory. Same for kernel ce"*.

Two rules, one workload, opposite answers. The date is part of the citation, and §46 is the
later one — but §12.26 is documented as a boundary, so the supersession should be **stated**
rather than inferred by whoever next touches the code.

## The argument each way

**For §46 superseding (my reading):**
- §12.26's stated *reason* is *"so the system proc never mints host memory"*. Under the single
  store that premise is obsolete: the store is **one reserved device-local object minted once at
  realize**. A guest-RAM pin against it mints nothing; it names an offset in something that
  already exists.
- The scrub is real work with an observable end state — memory that must read as cleared. §46 is
  explicit that this is not a no-op we may fake.
- The same reasoning already moved the guest's CE copies onto hardware, and nothing about the
  system proc makes its bytes different bytes.

**Against / what to be careful of:**
- §12.26 is about a **lifetime regime**, not a byte count: the system proc's isolate exists for
  the device's own bring-up and is not torn down with any guest process. Handing it data-plane
  state is a new lifetime question — *what frees these pins, and when?*
- `ce_executor_tree.md` (owner, 2026-08-07) separately rules that `ce_copy(Ours)` must keep
  refusing inside the isolate, calling it *"the security boundary refusing to leak guest memory
  into the sandbox, working as designed"*. ⊘ That is a **different** refusal from
  `SystemDataPlane` and is **not** what this page proposes changing — but the two are adjacent
  and must not be conflated in a hurry.

## If the ruling is "§46 wins", the work and its gate

1. `plan_pin_guest_ram` stops refusing `Gpu::SYSTEM_PROC` — and the lifetime question above gets
   an answer in the same change, not later.
2. The scrub's CE doorbell forwards like any other.
3. **The gate already exists and needs no new instrument:** tokens `0x00010001` and `0x00010004`
   must flip from `forwarded=0` to `forwarded>0` in the `DOORBELL-LEDGER`, and
   `run_fast_guest.sh`'s stranded-token check must be widened from guest tokens to all tokens.
   ⚠ Until then that check is deliberately scoped to `tok=0x000000xx` **precisely so it cannot
   pre-empt this ruling** — a gate that asserted the answer would make the question unaskable.

## What must not be read into this page

⊘ This is not a claim that the scrub is broken. It is forged, deliberately, by a rule that was
correct when written. The question is only whether the rule survives the single store.
