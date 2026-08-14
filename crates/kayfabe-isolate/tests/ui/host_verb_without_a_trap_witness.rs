// ★★★★★ w323 row 1: THE VIOLATION — a host RM verb issued with no proof of where the
// caller is standing. Before this rung the line below compiled, and it is exactly the
// shape that put a 3.70 s disposal (w317) and 13 313 serialized round trips (w319) on the
// vCPU thread with the QEMU BQL held, freezing every vCPU and QEMU's main loop.
//
// ⊘ This row is about the SIGNATURE, not about the panic. The runtime half —
// `OffTrap::claim` refusing inside a `TrapGuard` — is a known-positive in
// `kayfabe-util/src/trapwitness.rs`. The two are complements: a type cannot see a caller
// that never names it, and a panic cannot see a call that was never written.
fn main() {
    let mut w: kayfabe_isolate::Worker = unreachable!();
    let plan: kayfabe_isolate::VerbPlan = unreachable!();
    let _ = w.execute(&plan);
}
