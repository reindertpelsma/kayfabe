//! Synthetic privileged measurement fixtures; never measured hardware words.
use kf_host::precomp::measure_with;

const VERSION: &str = "595.91.07";
fn plan(n: u32) -> Vec<u32> {
    (0..n).map(|i| 0x640780 + i * 4).collect()
}
fn reply(cmd: u32, params: &mut [u8], ops: &mut [u8]) -> Result<(), String> {
    assert_eq!(cmd, 0x2080_0122);
    assert_eq!(params.len(), 48);
    assert!(ops.len() <= 32 * 32);
    assert_eq!(
        u32::from_le_bytes(params[20..24].try_into().unwrap()) as usize * 32,
        ops.len()
    );
    for op in ops.as_chunks_mut::<32>().0 {
        assert_eq!((op[0], op[1]), (0, 0), "READ32/GLOBAL only");
        for (i, byte) in op.iter().enumerate() {
            if i != 2 && !(12..16).contains(&i) {
                assert_eq!(*byte, 0, "no writes, channel target or flags");
            }
        }
        op[2] = 0;
        let address = u32::from_le_bytes(op[12..16].try_into().unwrap());
        op[20..24].copy_from_slice(&(address ^ 0x5566_0000).to_le_bytes());
    }
    Ok(())
}
#[test]
fn measured_first_word_is_real_read_only_reply() {
    assert_eq!(
        measure_with(VERSION, true, &plan(1), reply).unwrap(),
        vec![0x640780 ^ 0x5566_0000]
    );
}
#[test]
fn measured_last_word_uses_bounded_read_only_batches() {
    let addresses = plan(192);
    assert_eq!(
        measure_with(VERSION, true, &addresses, reply).unwrap(),
        addresses
            .iter()
            .map(|a| a ^ 0x5566_0000)
            .collect::<Vec<_>>()
    );
}
fn must_refuse(version: &str, privileged: bool, addresses: &[u32]) {
    assert!(
        measure_with(version, privileged, addresses, |_, _, _| panic!(
            "invalid request reached RM"
        ))
        .is_err()
    );
}
#[test]
fn measurement_without_admin_never_issues_a_control() {
    must_refuse(VERSION, false, &plan(1));
}
#[test]
fn full_page_without_admin_never_issues_a_control() {
    must_refuse(VERSION, false, &plan(192));
}
#[test]
fn unknown_driver_is_not_a_nearest_abi_measurement() {
    must_refuse("595.91.08", true, &plan(1));
}
#[test]
fn unknown_branch_is_not_a_measurement() {
    must_refuse("999.1.2", true, &plan(192));
}
#[test]
fn empty_measurement_plan_is_refused() {
    must_refuse(VERSION, true, &[]);
}
#[test]
fn oversized_measurement_plan_is_refused() {
    must_refuse(VERSION, true, &plan(193));
}
#[test]
fn first_unaligned_measurement_is_refused() {
    must_refuse(VERSION, true, &[0x640781]);
}
#[test]
fn last_unaligned_measurement_is_refused() {
    must_refuse(VERSION, true, &[0x640780, 0x640785]);
}
fn denied(last: bool) {
    assert!(
        measure_with(VERSION, true, &plan(48), |c, p, o| {
            reply(c, p, o)?;
            let i = if last { o.len() - 32 } else { 0 };
            o[i + 2] = 32;
            Ok(())
        })
        .is_err()
    );
}
#[test]
fn first_denied_word_abandons_measurement() {
    denied(false);
}
#[test]
fn last_denied_word_abandons_measurement() {
    denied(true);
}
fn metadata(index: usize) {
    assert!(
        measure_with(VERSION, true, &plan(2), |c, p, o| {
            reply(c, p, o)?;
            o[index] ^= 1;
            Ok(())
        })
        .is_err()
    );
}
#[test]
fn changed_read_operation_is_refused() {
    metadata(0);
}
#[test]
fn changed_register_address_is_refused() {
    metadata(12);
}
#[test]
fn changed_count_is_refused() {
    assert!(
        measure_with(VERSION, true, &plan(2), |c, p, o| {
            reply(c, p, o)?;
            p[20] ^= 1;
            Ok(())
        })
        .is_err()
    );
}
#[test]
fn ioctl_refusal_does_not_yield_measured_words() {
    assert!(measure_with(VERSION, true, &plan(2), |_, _, _| Err("RM 0x1f".into())).is_err());
}
