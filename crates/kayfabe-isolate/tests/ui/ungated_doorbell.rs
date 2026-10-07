// ★★ #14 / ARCHITECTURE.md invariant 5: a ring plan that never passed the ring-gate must
// be UNSPELLABLE outside `kayfabe-isolate`. This is the struct expression the old
// `tests/tests/cross_proc_lifetime.rs` wrote verbatim — every field correct, every value
// legal, and now a compile error (E0639), because `VerbPlan::Doorbell` is
// `#[non_exhaustive]`. The only way to obtain the variant is `VerbPlan::gated_doorbell`,
// which runs the #14 working-set gate before a plan exists to hand a `Worker`.
use kayfabe_arch::ids::EngineKind;
use kayfabe_isolate::VerbPlan;

fn main() {
    let _ungated = VerbPlan::Doorbell {
        host_vas: None,
        channel: None,
        engine: EngineKind::Ce,
        schedule: true,
    };
}
