//! Fixed-layout, owned-handle readbacks used only by default-off TSG facts.
use crate::Channel;
pub(crate) fn timeslice(
    _chan: Channel,
    _call: impl FnMut(u32, u32, &mut [u8]) -> Result<u32, u32>,
) -> Result<u64, u32> {
    Err(0x40)
}
pub(crate) fn modes(
    _subdevice: u32,
    _chan: Channel,
    _call: impl FnMut(u32, u32, &mut [u8]) -> Result<u32, u32>,
) -> Result<[u32; 5], u32> {
    Err(0x40)
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
            timeslice(channel(), |_, _, p| {
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
            modes(21, channel(), |_, _, p| {
                p.fill(255);
                Ok(0x56)
            }),
            Err(0x56)
        );
        assert_eq!(modes(21, channel(), |_, _, _| Err(0x40)), Err(0x40));
    }
}
