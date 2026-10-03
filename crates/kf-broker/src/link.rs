// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The real [`Link`]: a non-blocking `AF_UNIX` client through `kf-linux-raw`'s doors, and
//! the peer policy's inputs (this process's effective uid, a directory's owner).

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

    fn dir_owner(&mut self, dir: &Path) -> Option<u32> {
        use std::os::unix::fs::MetadataExt as _;
        std::fs::metadata(dir).ok().map(|m| m.uid())
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

/// ★ This process's effective uid, from `/proc/self/status` (`Uid:` real, EFFECTIVE, saved, fs).
///
/// # Errors
/// When the file cannot be read or parsed.
pub fn effective_uid() -> Result<u32, String> {
    let s = std::fs::read_to_string("/proc/self/status").map_err(|e| e.to_string())?;
    s.lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| "no effective uid in /proc/self/status".to_string())
}

/// ★ The uids accepted as the broker besides the socket directory's owner (evaluated at each
/// connect): root, this process's effective uid, and `extra` (the `display-broker-uid`
/// property). The directory's owner already decides who can create the path, so admitting it
/// adds no squatter; a uid outside all of these is refused before a byte is read.
///
/// # Errors
/// When the effective uid cannot be read.
pub fn allowed_uids(extra: Option<u32>) -> Result<Vec<u32>, String> {
    let mut v = vec![0, effective_uid()?];
    v.extend(extra);
    v.sort_unstable();
    v.dedup();
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_effective_uid_is_this_processes_and_root_is_always_allowed() {
        let me = effective_uid().expect("/proc/self/status");
        let a = allowed_uids(Some(4242)).unwrap();
        assert!(a.contains(&0) && a.contains(&me) && a.contains(&4242));
        assert!(a.windows(2).all(|w| w[0] < w[1]), "sorted, no duplicates");
    }
}
