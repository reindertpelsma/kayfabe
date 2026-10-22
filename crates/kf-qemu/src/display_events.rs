//! Bounded display event values sampled before publishing the read shadow.
#![cfg_attr(
    not(test),
    deny(clippy::arithmetic_side_effects, clippy::indexing_slicing)
)]
use kf_disp::ports::{EventReg, Ports};

pub(crate) fn snapshot(ports: &Ports, heads: usize) -> Vec<u32> {
    let mut words = vec![
        ports.event(EventReg::AwakenWin),
        ports.event(EventReg::AwakenOther),
        ports.event(EventReg::SemWin),
    ];
    for h in 0..heads {
        words.push(ports.event(EventReg::HeadTiming(h)));
        words.push(ports.event(EventReg::HeadTimingEn(h)));
    }
    words
}
