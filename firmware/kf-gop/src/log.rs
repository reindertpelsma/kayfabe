// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! One formatted line in a fixed buffer, for the test build's debug port. No allocator exists in the
//! driver, so `format!` is not available; a line longer than the buffer is cut, never grown.

use core::fmt;

/// Bytes a line holds.
pub const LINE: usize = 192;

/// A line being formatted.
#[derive(Debug)]
pub struct Line {
    buf: [u8; LINE],
    len: usize,
}

impl Default for Line {
    fn default() -> Line {
        Line {
            buf: [0; LINE],
            len: 0,
        }
    }
}

impl Line {
    /// The formatted bytes, newline included if one was written.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = LINE - self.len;
        let n = s.len().min(room);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// Format `args` into a [`Line`] and append a newline.
#[must_use]
pub fn line(args: fmt::Arguments<'_>) -> Line {
    use fmt::Write as _;
    let mut l = Line::default();
    let _ = l.write_fmt(args);
    let _ = l.write_str("\n");
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_format_and_truncate() {
        assert_eq!(
            line(format_args!("kf-gop: {} {:#x}", "start", 0x10)).bytes(),
            b"kf-gop: start 0x10\n"
        );
        let long = [b'a'; LINE + 10];
        let l = line(format_args!("{}", core::str::from_utf8(&long).unwrap()));
        assert_eq!(l.bytes().len(), LINE);
    }
}
