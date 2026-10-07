// ★ Cacheability has no silent default. `CachePolicy` deliberately implements no
// `Default`, so there is no `..Default::default()`, no `unwrap_or_default()` and no
// implicit write-back to fall back on — the attribute must be *stated* at the mapping
// site or the call does not compile (the parameter is also non-`Option`, which arity
// alone enforces).
//
// This is the refusal the C never had: its first policy was a blanket, and every
// subsequent fix was an after-the-fact correction to it. A default IS a blanket.
use kayfabe_linux_raw::CachePolicy;

fn main() {
    let _implicit: CachePolicy = Default::default();
    let _also: CachePolicy = CachePolicy::default();
}
