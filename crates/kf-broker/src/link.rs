// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The real [`Link`]: a non-blocking `AF_UNIX` client through `kf-linux-raw`'s doors, and
//! the peer policy's inputs (this process's effective uid, the `display-broker-uid` property).

use crate::conn::{Link, Recv, Sent};
use crate::wire::CMD_SIZE;
use kf_linux_raw::{peer_credentials, recv_bounded, send_record, unix_connect};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::Path;

/// The socket operations over a real `AF_UNIX` stream.
#[derive(Debug, Default)]
pub struct UnixLink;

impl Link for UnixLink {
    type Sock = OwnedFd;

    fn connect(&mut self, path: &Path) -> Result<OwnedFd, String> {
        unix_connect(path).map_err(|e| e.to_string())
    }

    fn fd(&self, sock: &OwnedFd) -> i32 {
        sock.as_raw_fd()
    }

    fn peer_uid(&mut self, sock: &OwnedFd) -> Result<u32, String> {
        peer_credentials(sock.as_fd())
            .map(|c| c.uid)
            .map_err(|e| e.to_string())
    }

    fn effective_uid(&mut self) -> u32 {
        effective_uid()
    }

    fn send(&mut self, sock: &OwnedFd, rec: &[u8; CMD_SIZE], fd: Option<BorrowedFd<'_>>) -> Sent {
        loop {
            return match send_record(sock.as_fd(), rec, fd) {
                Ok(n) if n == CMD_SIZE => Sent::Done,
                // fatal, never resynced: the descriptor went with the first byte
                Ok(n) => Sent::Failed(format!("short write ({n} of {CMD_SIZE} bytes)")),
                Err(e) if e.is_interrupted() => continue,
                Err(e) if e.is_would_block() => Sent::Full,
                Err(e) => Sent::Failed(e.to_string()),
            };
        }
    }

    fn recv(&mut self, sock: &OwnedFd, buf: &mut [u8]) -> Recv {
        loop {
            return match recv_bounded(sock.as_fd(), buf) {
                Ok(r) if r.control_dropped => Recv::FdDropped,
                Ok(r) if r.bytes == 0 => Recv::Closed,
                Ok(r) => Recv::Bytes(r.bytes),
                Err(e) if e.is_interrupted() => continue,
                Err(e) if e.is_would_block() => Recv::Empty,
                Err(e) => Recv::Failed(e.to_string()),
            };
        }
    }
}

/// ★ This process's effective uid NOW (`geteuid`). The relay reads it at every connect attempt,
/// never once at realize: QEMU drops privileges (`-run-with user=`, `-runas`) after its devices
/// are realized, so a value read at realize would name root.
#[must_use]
pub fn effective_uid() -> u32 {
    kf_linux_raw::effective_uid()
}

/// ★ The uids accepted as the broker at one connect attempt: root, `euid` (this process's
/// effective uid at that attempt) and `extra` (the `display-broker-uid` property). Sorted, no
/// duplicates.
///
/// ⊘ The owner of the socket's parent directory is NOT among them (corrected 2026-10-03 by the
/// review of `v3-broker`): the owner of a directory does not decide who can create the path when
/// the directory itself can be created by anyone — `/tmp/kf3` after a reboot is made by whichever
/// local user runs `mkdir` first (sticky `/tmp`), who could then bind the socket, be shown the
/// guest's screen and type into it.
#[must_use]
pub fn broker_uids(euid: u32, extra: Option<u32>) -> Vec<u32> {
    let mut v = vec![0, euid];
    v.extend(extra);
    v.sort_unstable();
    v.dedup();
    v
}

/// The largest uid `display-broker-uid` accepts: `(uid_t)-1` (4294967295) is the kernel's "no
/// change" value, never a process's uid.
pub const MAX_BROKER_UID: i64 = 4_294_967_294;

/// ★ The `display-broker-uid` property: `-1` (the default) is none; `0..=`[`MAX_BROKER_UID`] is
/// one more uid accepted as the broker.
///
/// # Errors
/// Any other value, by name — the device refuses it at realize rather than silently ignoring it.
pub fn broker_uid_property(v: i64) -> Result<Option<u32>, String> {
    match v {
        -1 => Ok(None),
        0..=MAX_BROKER_UID => Ok(u32::try_from(v).ok()),
        _ => Err(format!(
            "display-broker-uid={v} is not a uid: use -1 (the default: none) or a uid in \
             0..={MAX_BROKER_UID}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_this_process_and_the_property_are_the_broker_uids() {
        let me = effective_uid();
        let a = broker_uids(me, Some(4242));
        assert!(a.contains(&0) && a.contains(&me) && a.contains(&4242));
        assert!(a.windows(2).all(|w| w[0] < w[1]), "sorted, no duplicates");
        assert_eq!(broker_uids(0, None), vec![0]);
        assert_eq!(broker_uids(1000, Some(1000)), vec![0, 1000]);
    }

    /// ★ `display-broker-uid` outside `-1` and `0..=4294967294` is refused by name, never
    /// silently ignored (the review of `v3-broker`, 2026-10-03).
    #[test]
    fn an_out_of_range_display_broker_uid_is_refused_by_name() {
        assert_eq!(broker_uid_property(-1), Ok(None));
        assert_eq!(broker_uid_property(0), Ok(Some(0)));
        assert_eq!(broker_uid_property(1000), Ok(Some(1000)));
        assert_eq!(broker_uid_property(MAX_BROKER_UID), Ok(Some(u32::MAX - 1)));
        for bad in [
            -2,
            i64::MIN,
            i64::from(u32::MAX),
            i64::from(u32::MAX) + 1,
            i64::MAX,
        ] {
            let e = broker_uid_property(bad).unwrap_err();
            assert!(
                e.contains("display-broker-uid") && e.contains(&bad.to_string()),
                "{bad}: {e}"
            );
        }
    }
}
