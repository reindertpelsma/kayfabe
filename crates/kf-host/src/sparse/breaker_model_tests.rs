/// STRICT: every invariant, including live-page occupancy accuracy (I1).
#[test]
fn breaker_model_strict_roomy_budget() {
    if let Some(r) = fz::campaign(5000, 1200, 90, false, false) {
        panic!("invariant violations (clean RM, budget never reached):{r}");
    }
}
#[test]
fn breaker_model_strict_tight_budget() {
    if let Some(r) = fz::campaign(12, 1200, 90, false, false) {
        panic!("invariant violations (clean RM, budget 12):{r}");
    }
}
#[test]
fn breaker_model_strict_refusing_rm() {
    if let Some(r) = fz::campaign(5000, 1200, 90, true, false) {
        panic!("invariant violations (transient RM refusals):{r}");
    }
}
/// LENIENT: the known occupancy defect is tolerated (a lost page is forgotten) so the
/// campaign can look for OTHER breaks (I2, I3, tree/RM agreement, routing).
#[test]
fn breaker_model_lenient_tight_budget() {
    if let Some(r) = fz::campaign(12, 1200, 90, false, true) {
        panic!("invariant violations beyond occupancy (budget 12):{r}");
    }
}
#[test]
fn breaker_model_lenient_refusing_rm() {
    if let Some(r) = fz::campaign(5000, 1200, 90, true, true) {
        panic!("invariant violations beyond occupancy (RM refusals):{r}");
    }
}
#[test]
fn breaker_model_lenient_refusing_rm_tight_budget() {
    if let Some(r) = fz::campaign(30, 1200, 90, true, true) {
        panic!("invariant violations beyond occupancy (RM refusals, budget 30):{r}");
    }
}
