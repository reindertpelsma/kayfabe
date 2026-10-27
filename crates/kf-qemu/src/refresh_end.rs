//! Separate mandatory refresh readiness from optional retry without requiring a GPU in tests.

pub(crate) fn finish(
    _critical: bool,
    _ready: impl FnOnce() -> Result<(), String>,
    _retry: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ens11_critical_refresh_propagates_readiness_refusal_without_retry() {
        let result = finish(
            true,
            || Err("poisoned sparse bookkeeping".into()),
            || panic!("optional geometry must not run before split release"),
        );
        assert_eq!(result, Err("poisoned sparse bookkeeping".into()));
    }
    #[test]
    fn ens11_regular_refresh_propagates_the_retry_refusal() {
        let result = finish(
            false,
            || panic!("ordinary refresh must retry"),
            || Err("reservation retry refused".into()),
        );
        assert_eq!(result, Err("reservation retry refused".into()));
    }
}
