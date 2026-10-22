**STATUS: LIVE,2026-10-22.** Owner-authorized parallel Ada/Blackwell lanes share the account,
but each has its own budget and at most one own active box. The old global-empty account check
incorrectly rejected the independent Ada lane, without creating a new SW2 contract.
Seam247a2baca ->tests-onlyEXPECTEDREDed890f68f(3failures) ->fix4c3f6cb4d.
All7admissioncases green; complete Python97->104/0. Two mutants killed by3distinct tests each,
source restored. Own-id and mixed-account checks still refuse; labels do not transferownership.
The operational runner enumerates only this lane's owned-id ledgers, leaves foreigninstances
untouched, reserves its ownruntime+billing allowance and checks globalcreditreserve5 separately.
No Rust/product change or new Xid permission. Runtimeguardian sourcee16402b7c remains97tests/22mutants.
