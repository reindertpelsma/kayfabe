# Independent review record

**STATUS: ANSWERED, 2026-10-11.** Read-only review by a separate agent; no hardware operations
or source edits by the reviewer. Integration checkpoint: `a2632e64`.

| Reviewed candidate | Result | Findings and disposition |
|---|---|---|
| `417bad6d` | Blocked | DP field existence incorrectly advertised implemented MST/dual-SST; SOR sublink input ignored; reserved-panel output echoed; current README exposed private diagnosis. Fixed at `a590f10a`. |
| `a590f10a` | Blocked | Three physical-RM software bits lacked emulator implementations; SOR audio request/output flags ignored. Fixed at `b88f2f60`. |
| `b88f2f60` | No remaining correctness blockers | Conditional on exact-candidate hardware regression. One orphaned field comment removed at `02afce90`; no behavior change. |
| `e3564bd4` vs `a2632e64` CI | No introduced gate failures | Product source differs from hardware revision `02afce90` only by the verified header citation comment. |

Five failing-first tests reproduce the runtime defects. Generated layout additions were compiled
against each supported open-header version. Physical-GPU measurements remain reference evidence;
they no longer supply capability bits for absent emulator behavior.

Quick CI remains red in both baseline and candidate with identical offenders:

| Gate | Inherited failure |
|---|---|
| Unsafe surface | `crates/kf-mem/src/sim/tests.rs` |
| Untagged citation | `crates/kf-abi/src/chanbudget.rs:10` |
| Claim ledger | existing=3145, new=85, resolved=9; identical offender set |
| Duplicate sections | `V3_COMPLETION_AUDIT.md`: sections 1, 2, 3 |

Full outputs: `logs/ci_baseline.log` and `logs/ci_candidate.log`. This is approval for integration
after the Linux regression, not a CI-green or master merge claim.

## Windows harness follow-up

Independent review found no credential mapping, argv or logging leak in the new sign-in helper.
It required two corrections: a logged-in username can coexist with the lock screen, so acceptance
also requires no LogonUI process; failed sign-in must propagate nonzero after cleanup. Both were
applied. Run 642 predates these refinements; its recovered probes actually executed and their
results remain valid. No product code changed.
