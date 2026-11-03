//! The bounded unprivileged host PRECOMP read (`kf_host::precomp::read_with`): READ32/GLOBAL only,
//! fixed-size batches, no write and no channel target, and a refused or malformed batch abandons the
//! whole snapshot. The reply fixture is synthetic.
use kf_host::precomp::read_with;
use std::cell::Cell;

const VERSION: &str = "595.91.07";
fn u32_at(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn reply(cmd: u32, p: &mut [u8], ops: &mut [u8]) -> Result<(), String> {
    assert_eq!(cmd, 0x2080_0122);
    assert_eq!(p.len(), 48);
    assert_eq!(u32_at(p, 20) as usize * 32, ops.len());
    assert!(u32_at(p, 20) <= 32);
    assert_eq!(
        u64::from_le_bytes(p[24..32].try_into().unwrap()),
        ops.as_mut_ptr() as u64
    );
    for (i, byte) in p.iter().enumerate() {
        if !(20..32).contains(&i) {
            assert_eq!(*byte, 0);
        }
    }
    for op in ops.as_chunks_mut::<32>().0.iter_mut() {
        assert_eq!(op[0], 0, "READ32 only");
        assert_eq!(op[1], 0, "GLOBAL only");
        for (i, b) in op.iter().enumerate() {
            if i != 2 && !(12..16).contains(&i) {
                assert_eq!(*b, 0);
            }
        }
        op[2] = 0;
        let value = u32_at(op, 12) ^ 0x5566_0000; // Synthetic, not a die measurement.
        op[20..24].copy_from_slice(&value.to_le_bytes());
    }
    Ok(())
}
fn addresses(n: usize) -> Vec<u32> {
    (0..n)
        .map(|i| 0x640780 + u32::try_from(i).unwrap() * 4)
        .collect()
}

#[test]
fn first_precomp_values_are_read_without_write_or_channel_target() {
    let a = addresses(2);
    assert_eq!(
        read_with(VERSION, true, &a, reply).unwrap(),
        a.iter().map(|x| x ^ 0x5566_0000).collect::<Vec<_>>()
    );
}
#[test]
fn final_bounded_word_is_read_in_fixed_size_batches() {
    let a = addresses(192);
    assert_eq!(
        read_with(VERSION, true, &a, reply).unwrap(),
        a.iter().map(|x| x ^ 0x5566_0000).collect::<Vec<_>>()
    );
}
#[test]
fn all_five_compiled_host_abis_use_the_same_read_only_contract() {
    for v in ["580.65.06", "580.94.02", "580.159.04", "590.48.01", VERSION] {
        assert_eq!(read_with(v, true, &addresses(48), reply).unwrap().len(), 48);
    }
}
fn bad_status(at_end: bool) {
    let called = Cell::new(false);
    let r = read_with(VERSION, true, &addresses(48), |cmd, p, o| {
        called.set(true);
        reply(cmd, p, o)?;
        let n = if at_end { o.len() - 32 } else { 0 };
        o[n + 2] = 32;
        Ok(())
    });
    assert!(called.get());
    assert!(r.is_err());
}
#[test]
fn first_denied_register_abandons_the_whole_snapshot() {
    bad_status(false);
}
#[test]
fn final_denied_register_abandons_the_whole_snapshot() {
    bad_status(true);
}
fn bad_offset(at_end: bool) {
    let called = Cell::new(false);
    let r = read_with(VERSION, true, &addresses(48), |cmd, p, o| {
        called.set(true);
        reply(cmd, p, o)?;
        let n = if at_end { o.len() - 32 } else { 0 };
        o[n + 12] ^= 4;
        Ok(())
    });
    assert!(called.get());
    assert!(r.is_err());
}
#[test]
fn changed_first_register_is_a_malformed_reply() {
    bad_offset(false);
}
#[test]
fn changed_last_register_is_a_malformed_reply() {
    bad_offset(true);
}
fn refusal(detail: &str) {
    let r = read_with(VERSION, true, &addresses(48), |_, _, _| {
        Err(detail.to_string())
    });
    assert!(r.unwrap_err().contains(detail));
}
#[test]
fn absent_control_is_a_named_fallback() {
    refusal("control absent");
}
#[test]
fn permission_refusal_is_a_named_fallback() {
    refusal("register permission denied");
}
#[test]
fn a_later_failed_batch_does_not_return_a_partial_page() {
    let calls = Cell::new(0);
    let r = read_with(VERSION, true, &addresses(48), |cmd, p, o| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            return Err("second batch refused".into());
        }
        reply(cmd, p, o)
    });
    assert_eq!(calls.get(), 2);
    assert!(r.is_err());
}
fn refused_before_ioctl(version: &str, unprivileged: bool, a: &[u32]) {
    let calls = Cell::new(0);
    assert!(
        read_with(version, unprivileged, a, |_, _, _| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .is_err()
    );
    assert_eq!(calls.get(), 0);
}
#[test]
fn root_capability_small_plan_never_uses_administrator_bypass() {
    refused_before_ioctl(VERSION, false, &addresses(1));
}
#[test]
fn root_capability_full_plan_never_uses_administrator_bypass() {
    refused_before_ioctl(VERSION, false, &addresses(48));
}
#[test]
fn empty_plan_never_issues_an_ioctl() {
    refused_before_ioctl(VERSION, true, &[]);
}
#[test]
fn oversized_plan_never_issues_an_ioctl() {
    refused_before_ioctl(VERSION, true, &addresses(193));
}
#[test]
fn uncompiled_host_version_is_not_a_neighbour_layout() {
    refused_before_ioctl("580.95.05", true, &addresses(48));
}
#[test]
fn newer_uncompiled_version_is_a_clean_fallback() {
    refused_before_ioctl("610.43.02", true, &addresses(48));
}
#[test]
fn unaligned_first_address_never_issues_an_ioctl() {
    let mut a = addresses(2);
    a[0] += 1;
    refused_before_ioctl(VERSION, true, &a);
}
#[test]
fn unaligned_last_address_never_issues_an_ioctl() {
    let mut a = addresses(2);
    a[1] += 1;
    refused_before_ioctl(VERSION, true, &a);
}
