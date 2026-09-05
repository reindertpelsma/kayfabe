# The nine red tests — what they actually assert, 2026-09-06

**STATUS: LIVE, 2026-09-06.** Opened after an external observation (the `nvkvm-pv` agent, relayed
by the owner) landed exactly on this project's own README text. Supersedes the sentence *"a
bookkeeping gap, not a functional defect"* wherever it still appears.

---

## 0. ★★★★★ THE FINDING: THE EXPLANATION WAS INHERITED, AND IT WAS WRONG

`cargo test --workspace --no-fail-fast` at `ee50148d`: **2949 pass, 9 fail, 258 binaries.**
The README described all nine as bookkeeping. Read against what the tests actually assert,
**at least 6 of 9 are functional, safety or structural defects.**

⊘ The sentence was carried across several README rewrites — including one I made the same day,
where I corrected the *numbers* and kept the *explanation*. **Neither the number nor the
explanation had been checked against the tests' own assertion text.**

## 1. The nine, by what they claim

| # | test | class | what its own message says |
|---|---|---|---|
| 1 | `a_guest_doorbell_reaches_the_host_completion_observer` | **functional** | *"THE SEVERANCE … `Served` here means: we rang a doorbell on a host channel into which the guest's methods were never copied"* |
| 2 | `the_observers_negative_verdict_refuses_the_guest_doorbell` | **★ FORGERY** | *"The engine never released the semaphore and the guest was told `Served` … a caller that discards the verdict throws that away and forges the completion"* |
| 3 | `a_wired_device_refuses_a_framebuffer_page_nothing_ever_wrote` | **★ FORGERY** | *"A device that consults its source but ignores residency reads 4 KiB of zeros and reports SERVED"* |
| 4 | `a_device_with_no_fb_source_refuses_the_vidmem_ring` | **refusal** | an unregistered device must be byte-identical to the pre-route-B tree; it is not |
| 5 | `the_logic_crates_carry_no_unnamed_guest_os_assumption` | **AXIS** | `/dev/nvidiactl or /dev/nvidia0` at `kayfabe-abi/src/submit.rs:5095`, with no comment naming the assumption |
| 6 | `every_unranked_lock_a_vcpu_thread_can_hold_is_classified` | **concurrency** | a new unranked `Mutex<HashMap<(u64,u64),usize>>` on the vCPU path in `shim.rs`; *"a wait beneath this will pass every assertion and stall the register plane"* |
| 7 | `the_audited_crate_list_matches_the_tree_and_is_used_by_all_three_sub_gates` | **META-GATE** | `kayfabe-linux-raw` declares **91** relaxations in `ci.yml`, the tree has **93** — *"a ratchet that has quietly become a comment"* |
| 8 | `every_unserviced_id_a_boot_recorded_is_classified` | ledger | census classification, evidence committed as excerpt |
| 9 | `cap1b`/replay ledger residue | ledger | see `w329_wiring_the_release.md` |

★★★ **#2 and #3 are completion forgery**, which this project rules out by name (owner, standing:
*"do the passthrough properly, no mindless completion forgery"*; *"completions should only be
sent if the observed state after it is intended and safe in the guest"*). A red test asserting
that prohibition is not bookkeeping — it is the prohibition's only enforcement.

★★★ **#7 is the gate that checks the other gates' scope.** While it is red, every other gate's
coverage claim is unverified: two relaxations exist in the tree that no audit accounts for.

★★ **#5 and #6 are two of the design axes** the project's value rests on (guest-OS
independence; the vCPU-path non-blocking invariant).

## 2. ◐ HYPOTHESIS, NOT VERIFIED — one cause, not four

Tests 1–4 all emit the same two lines before failing:

```
kayfabe: DOORBELL-XLATE proc=1 chan=0 vchid=VChid(0x31) engine=Ce ... schedule=true
kayfabe-isolate: DOORBELL-VERB engine=Ce host_token=0x200000 scheduled=true → calling ring_doorbell
```

and all four report `Ok(DoorbellOutcome { … kind: Passthrough })` where a refusal was required.
⇒ **Working hypothesis: the passthrough doorbell path is armed and bypasses both the completion
observer and the residency check.** That would make these one regression with four witnesses
rather than four defects.

⊘ **Unverified.** It must not be written down as a cause until a bisect or a code path says so —
this file exists because an unverified explanation was believed once already.

## 3. ★★★★★ THE RULE THIS COST

**Explaining a failure away is how a project loses an instrument.** The question is never
*"is my explanation plausible"* — plausible explanations are exactly what survive — but:

> ### ⚠ **IF I AM WRONG, WHAT NOW GOES UNNOTICED?**

Here the answer was: the completion-forgery guard, the residency guard, the guest-OS axis gate,
the vCPU-lock gate, and the meta-gate that scopes the others. Five instruments, all reported as
clerical.

★ The external formulation that prompted this (`nvkvm-pv` agent, relayed 2026-09-06) — worth
keeping verbatim because it names three shapes we have all five of:
> *"A green suite is evidence about your tests, not about your system. The dangerous bug is the
> one whose only symptom is a number that looks merely disappointing."*
and the worst shape, a misdiagnosis that **deleted its own test** — *"the wrong explanation
removed the instrument."*

⇒ **Rule adopted:** a red test may be deprioritised, but its **class** must be read off its own
assertion text and recorded before it is. *"Known failure"* is not a class. And no failure may be
described in a public document by a phrase that has not been checked against the assertion in
this release.
