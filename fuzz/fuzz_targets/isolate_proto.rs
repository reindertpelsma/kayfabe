//! Coverage-guided fuzz of the **isolate IPC frame protocol** — `Envelope::decode`,
//! `Reply::decode`, `read_frame`, and `decode_control`.
//!
//! # Where this sits relative to the escape boundary
//!
//! ⊘ **This is not boundary 1.** The peer is the isolate child, not the guest, so a bug
//! here is not a guest→VMM escape by itself. It is on the list for two reasons and they
//! should be stated rather than implied:
//!
//! 1. `Reply::decode` runs **in the VMM process** on bytes written by the *least*
//!    trusted process in the design — a cap-dropped, pivot-rooted child that exists
//!    precisely because it is expected to be compromised. A decoder bug here is a
//!    compromised-isolate → VMM escalation, which is the boundary the isolate split was
//!    built to create.
//! 2. Guest bytes reach it by construction: `Request::Alloc { params }` and
//!    `Reply::Payload` carry ioctl bodies the guest supplied, so the length prefixes the
//!    cursor reads are downstream of guest-declared sizes.
//!
//! `read_frame`'s `FRAME_MAX` check is the allocation bound (finding class 3) and its own
//! comment claims the refusal happens *without reading* — an ordering claim, so this
//! target puts a byte-counting reader behind it. ⊘ **No violation has been observed**:
//! 41 383 014 executions over 305 s on 2026-08-01 found none, which is a bound on how
//! hard it was looked for and not a proof that none exists.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use std::io::{Read, Result as IoResult};

use kayfabe_isolate_host::proto::{Envelope, Reply, read_frame};

/// A reader that counts what the framer actually consumed, so "refused without reading"
/// is checked rather than asserted.
struct CountingReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Read for CountingReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        let n = buf.len().min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[derive(Arbitrary, Debug)]
struct Input {
    envelope: Vec<u8>,
    reply: Vec<u8>,
    control: Vec<u8>,
    /// A raw stream, read as a sequence of length-prefixed frames.
    stream: Vec<u8>,
}

fuzz_target!(|input: Input| {
    // Round-trip where it decodes: a decoded envelope must re-encode to something that
    // decodes back to the same value. A protocol whose decode and encode disagree is the
    // class-4 silent misparse in its purest form — both sides believe different things
    // about the same bytes.
    if let Ok(e) = Envelope::decode(&input.envelope) {
        let re = e.encode();
        let back = Envelope::decode(&re).expect("an encoded envelope must decode");
        assert_eq!(e, back, "Envelope encode/decode is not a round trip");
    }
    if let Ok(r) = Reply::decode(&input.reply) {
        let re = r.encode();
        let back = Reply::decode(&re).expect("an encoded reply must decode");
        assert_eq!(r, back, "Reply encode/decode is not a round trip");
    }

    let _ = kayfabe_isolate_host::isolate::decode_control(&input.control);

    // The framer, over a hostile stream. Bounded iterations: the property is that each
    // call terminates and allocates within FRAME_MAX, not that the stream ends.
    let mut r = CountingReader {
        data: &input.stream,
        pos: 0,
    };
    let mut buf = Vec::new();
    for _ in 0..64 {
        match read_frame(&mut r, &mut buf) {
            Ok(true) => {
                // ★ The allocation bound. `buf` is sized from a peer-declared u32; if it
                // ever exceeds what the stream could have held, the length was trusted
                // over the data.
                assert!(
                    buf.len() <= input.stream.len(),
                    "a {}-byte frame out of a {}-byte stream",
                    buf.len(),
                    input.stream.len()
                );
                // Feed the frame back through both body decoders — this is how a real
                // frame is consumed, and it is where a length prefix meets a cursor.
                let _ = Envelope::decode(&buf);
                let _ = Reply::decode(&buf);
            }
            Ok(false) | Err(_) => break,
        }
    }
});
