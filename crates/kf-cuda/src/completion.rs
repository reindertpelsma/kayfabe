// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Walk completion policy, separate from the CUDA binding for GPU-free ordering tests.

pub(crate) fn ready<E>(
    signalled: bool,
    query: impl FnOnce() -> Result<bool, E>,
) -> Result<bool, E> {
    if signalled { Ok(true) } else { query() }
}

pub(crate) const fn poll_ms(_in_flight: bool) -> u32 {
    50
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_signal_cannot_complete_a_pending_gpu_event() {
        let queries = Cell::new(0);
        for signalled in [true, false, true] {
            let observed: Result<bool, ()> = ready(signalled, || {
                queries.set(queries.get() + 1);
                Ok(false)
            });
            assert_eq!(observed, Ok(false));
        }
        assert_eq!(queries.get(), 3);
    }

    #[test]
    fn a_signal_cannot_hide_a_gpu_failure() {
        assert_eq!(
            ready(true, || Err::<bool, _>("CUDA_ERROR_ILLEGAL_ADDRESS")),
            Err("CUDA_ERROR_ILLEGAL_ADDRESS")
        );
    }

    #[test]
    fn a_completed_event_needs_no_callback_signal() {
        assert_eq!(ready(false, || Ok::<_, ()>(true)), Ok(true));
    }

    #[test]
    fn a_pending_event_needs_another_look() {
        assert_eq!(ready(false, || Ok::<_, ()>(false)), Ok(false));
    }

    #[test]
    fn a_walk_is_checked_promptly_and_idle_keeps_the_backstop() {
        assert_eq!(poll_ms(true), 1);
        assert_eq!(poll_ms(false), 50);
    }
}
