//! Fixed-layout, owned-handle readbacks used only by default-off TSG facts.
use crate::Channel;
pub(crate) fn timeslice(
    chan: Channel,
    mut call: impl FnMut(u32, u32, &mut [u8]) -> Result<u32, u32>,
) -> Result<u64, u32> {
    let mut p = [0u8; 8];
    match call(chan.tsg, 0xa06c_0104, &mut p)? {
        0 => Ok(u64::from_le_bytes(p)),
        status => Err(status),
    }
}
pub(crate) fn modes(
    subdevice: u32,
    chan: Channel,
    mut call: impl FnMut(u32, u32, &mut [u8]) -> Result<u32, u32>,
) -> Result<[u32; 5], u32> {
    let mut p = [0u8; 24];
    p[..4].copy_from_slice(&chan.chan.to_le_bytes());
    match call(subdevice, 0x2080_1235, &mut p)? {
        0 => Ok(std::array::from_fn(|i| {
            let at = 4 + i * 4;
            u32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
        })),
        status => Err(status),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn channel() -> Channel {
        Channel {
            tsg: 31,
            chan: 41,
            token: 51,
            born_user: None,
        }
    }
    #[test]
    fn timeslice_uses_only_owned_group_and_zeroed_eight_byte_output() {
        let result = timeslice(channel(), |object, cmd, p| {
            assert_eq!((object, cmd), (31, 0xa06c0104));
            assert_eq!(p, &[0; 8]);
            p.copy_from_slice(&1001u64.to_le_bytes());
            Ok(0)
        });
        assert_eq!(result, Ok(1001));
    }
    #[test]
    fn timeslice_native_refusal_never_uses_the_reply_bytes() {
        assert_eq!(
            timeslice(channel(), |object, cmd, p| {
                assert_eq!((object, cmd), (31, 0xa06c0104));
                p.fill(255);
                Ok(0x1a)
            }),
            Err(0x1a)
        );
        assert_eq!(timeslice(channel(), |_, _, _| Err(0x40)), Err(0x40));
    }
    #[test]
    fn modes_uses_our_subdevice_and_channel_not_tsg_or_submit_token() {
        let result = modes(21, channel(), |object, cmd, p| {
            assert_eq!((object, cmd), (21, 0x20801235));
            assert_eq!(p.len(), 24);
            assert_eq!(&p[..4], &41u32.to_le_bytes());
            assert_eq!(&p[4..], &[0; 20]);
            for (i, value) in [1u32, 2, 0, 2, 1].into_iter().enumerate() {
                p[4 + i * 4..8 + i * 4].copy_from_slice(&value.to_le_bytes());
            }
            Ok(0)
        });
        assert_eq!(result, Ok([1, 2, 0, 2, 1]));
    }
    #[test]
    fn modes_refusal_and_transport_failures_keep_their_status() {
        assert_eq!(
            modes(21, channel(), |object, cmd, p| {
                assert_eq!((object, cmd), (21, 0x20801235));
                assert_eq!(&p[..4], &41u32.to_le_bytes());
                p.fill(255);
                Ok(0x56)
            }),
            Err(0x56)
        );
        assert_eq!(modes(21, channel(), |_, _, _| Err(0x40)), Err(0x40));
    }
    #[test]
    fn getters_decode_all_high_bits_and_keep_mode_field_order() {
        let us = 0x0102_0304_0506_0708u64;
        assert_eq!(
            timeslice(channel(), |_, _, p| {
                p.copy_from_slice(&us.to_le_bytes());
                Ok(0)
            }),
            Ok(us)
        );
        let values = [11u32, 12, 13, 14, 15];
        assert_eq!(
            modes(21, channel(), |_, _, p| {
                for (i, v) in values.iter().enumerate() {
                    p[4 + i * 4..8 + i * 4].copy_from_slice(&v.to_le_bytes());
                }
                Ok(0)
            }),
            Ok(values)
        );
    }
    #[test]
    fn both_getters_preserve_busy_and_memory_statuses_even_with_valid_bytes() {
        for status in [0x40, 0x51, 0x1a, 0x1f] {
            assert_eq!(
                timeslice(channel(), |_, _, p| {
                    p.copy_from_slice(&1000u64.to_le_bytes());
                    Ok(status)
                }),
                Err(status)
            );
            assert_eq!(modes(21, channel(), |_, _, _| Ok(status)), Err(status));
        }
    }
}
