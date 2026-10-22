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

#[cfg(test)]
mod tests {
    use super::*;
    const WORDS: usize = 3 + 2 * kf_disp::ports::MAX_HEADS;

    fn assert_plain_snapshot<T>(snapshot: &T) {
        assert!(
            !std::mem::needs_drop::<T>(),
            "trap snapshot owns a heap resource/destructor"
        );
        assert!(std::mem::size_of_val(snapshot) <= WORDS * std::mem::size_of::<u32>());
    }

    fn bounded_snapshot(heads: usize) {
        let p = Ports::default();
        p.raise(EventReg::AwakenWin, 0x11);
        p.raise(EventReg::AwakenOther, 0x22);
        p.raise(EventReg::SemWin, 0x44);
        for h in 0..heads {
            p.raise(EventReg::HeadTiming(h), 0x100 | h as u32);
            p.guest_write(EventReg::HeadTimingEn(h), 0x200 | h as u32);
        }
        let before = snapshot(&p, heads);
        assert_eq!(&before[..3], &[0x11, 0x22, 0x44]);
        for h in 0..heads {
            assert_eq!(before[3 + 2 * h], 0x100 | h as u32);
            assert_eq!(before[4 + 2 * h], 0x200 | h as u32);
        }
        assert!(before.len() <= WORDS);
        assert!(before.iter().skip(3 + 2 * heads).all(|v| *v == 0));
        p.guest_write(EventReg::AwakenWin, 1);
        p.guest_write(EventReg::HeadTimingEn(heads - 1), 0x8000);
        let after = snapshot(&p, heads);
        assert_ne!(
            after, before,
            "a racing event change must force another publication pass"
        );
        assert_eq!(after[0], 0x10);
        assert_eq!(after[4 + 2 * (heads - 1)], 0x8000);
        assert_plain_snapshot(&before);
        assert_plain_snapshot(&after);
    }

    #[test]
    fn worst_trap_one_head_event_snapshot_has_no_heap_ownership() {
        bounded_snapshot(1);
    }

    #[test]
    fn worst_trap_all_heads_event_snapshot_has_no_heap_ownership() {
        bounded_snapshot(kf_disp::ports::MAX_HEADS);
    }
}
