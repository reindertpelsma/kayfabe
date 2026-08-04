#!/usr/bin/env python3
"""test_decoder_refusals.py — break a REAL trace and watch the decoder refuse it.

Task #178, `replay-conformance`.

★★★ WHY THIS MUTATES THE REAL CAPTURE AND NOT A SYNTHETIC FILE.

A synthetic fixture is a file I wrote to be refused, checked by a decoder I wrote
to refuse it — the two agree because they came from the same idea, and neither
has met the recorder. Mutating the actual bytes that came off `vb` tests the
decoder against the format the RECORDER produces. If those two ever disagree,
this catches it and a synthetic fixture never would.

★★ AND IT ASSERTS THE UNMUTATED FILE PASSES FIRST. A refusal suite whose subject
is refused for an unrelated reason reports a perfect score having tested nothing —
this project's recurring failure. If line 1 is not "clean trace ACCEPTED", every
result below it is void.

⊘ ONE MUTATION HERE IS EXPECTED TO BE INERT, and it is listed rather than
quietly omitted: flipping a byte inside a record's PAYLOAD changes the file and
changes nothing the decoder can see, because there is no integrity check over
bodies. That is a real limit of the instrument, not an oversight: the recorder
copies bodies out of driver memory with no checksum to compare against, so a
decoder that claimed to detect body corruption would be lying. Recorded here so
nobody reads "9 of 10 mutations caught" as "the decoder validates payloads".

  usage: test_decoder_refusals.py TRACE.bin
  exit 0 — every mutation behaved as predicted (refused, or inert-as-documented)
  exit 1 — some mutation did NOT behave as predicted
"""

import os
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DECODER = os.path.join(HERE, "decode_rpctrace.py")
FILE_HDR_SIZE = 128
REC_HDR_SIZE = 48


def run_decoder(path, extra=()):
    r = subprocess.run([sys.executable, DECODER, path, "--quiet", *extra],
                       capture_output=True, text=True)
    return r.returncode, (r.stderr or "").strip()


def mutate_truncate_1(b):
    return b[:-1]


def mutate_truncate_1000(b):
    return b[:-1000]


def mutate_truncate_header(b):
    return b[:FILE_HDR_SIZE - 4]


def mutate_append_garbage(b):
    return b + b"\xde\xad\xbe\xef" * 8


def mutate_file_magic(b):
    return b"\x00\x00\x00\x00" + b[4:]


def mutate_version(b):
    return b[:4] + struct.pack("<I", 99) + b[8:]


def mutate_rec_hdr_size(b):
    return b[:12] + struct.pack("<I", 40) + b[16:]


def mutate_claim_dropped(b):
    """Set n_dropped in the header: the wrap/overflow refusal."""
    off = 4 * 4 + 8 * 4          # magic..rec_hdr_size, capacity..n_payload_bytes
    return b[:off] + struct.pack("<Q", 7) + b[off + 8:]


def mutate_claim_rx_failed(b):
    off = 4 * 4 + 8 * 7
    return b[:off] + struct.pack("<Q", 3) + b[off + 8:]


def mutate_second_record_magic(b):
    """Corrupt the magic of the SECOND record — mid-stream, not at the edge."""
    m = bytearray(b)
    first_cap = struct.unpack_from("<I", m, FILE_HDR_SIZE + 40)[0]
    off = FILE_HDR_SIZE + REC_HDR_SIZE + ((first_cap + 7) & ~7)
    struct.pack_into("<I", m, off, 0x11111111)
    return bytes(m)


def mutate_zero_cap_len(b):
    """★ The row that must not exist: a length with no bytes."""
    m = bytearray(b)
    struct.pack_into("<I", m, FILE_HDR_SIZE + 40, 0)
    return bytes(m)


def mutate_skip_seq(b):
    """Renumber the second record so the recorder's own counter has a gap."""
    m = bytearray(b)
    first_cap = struct.unpack_from("<I", m, FILE_HDR_SIZE + 40)[0]
    off = FILE_HDR_SIZE + REC_HDR_SIZE + ((first_cap + 7) & ~7)
    struct.pack_into("<I", m, off + 8, 999999)
    return bytes(m)


def mutate_n_records(b):
    off = 4 * 4 + 8 * 2
    n = struct.unpack_from("<Q", b, off)[0]
    return b[:off] + struct.pack("<Q", n + 1) + b[off + 8:]


def mutate_payload_byte(b):
    """⊘ EXPECTED INERT — see the module docstring."""
    m = bytearray(b)
    off = FILE_HDR_SIZE + REC_HDR_SIZE + 8
    m[off] ^= 0xFF
    return bytes(m)


MUTATIONS = [
    ("truncate by 1 byte",              mutate_truncate_1,          "refuse"),
    ("truncate by 1000 bytes",          mutate_truncate_1000,       "refuse"),
    ("truncate inside the file header", mutate_truncate_header,     "refuse"),
    ("append 32 bytes of garbage",      mutate_append_garbage,      "refuse"),
    ("zero the file magic",             mutate_file_magic,          "refuse"),
    ("bump the format version",         mutate_version,             "refuse"),
    ("wrong record-header size",        mutate_rec_hdr_size,        "refuse"),
    ("claim n_dropped=7 (RING WRAPPED)", mutate_claim_dropped,      "refuse"),
    ("claim n_rx_failed=3",             mutate_claim_rx_failed,     "refuse"),
    ("corrupt record #1's magic",       mutate_second_record_magic, "refuse"),
    ("set record #0 cap_len=0",         mutate_zero_cap_len,        "refuse"),
    ("gap in the record seq counter",   mutate_skip_seq,            "refuse"),
    ("header claims one extra record",  mutate_n_records,           "refuse"),
    ("flip a byte inside a payload",    mutate_payload_byte,        "inert"),
]


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    src = sys.argv[1]
    with open(src, "rb") as fh:
        blob = fh.read()

    rc, err = run_decoder(src)
    if rc != 0:
        print("‼ the UNMUTATED trace is refused (%s) — every result below would be "
              "meaningless, so stopping here." % err)
        return 1
    print("clean trace ACCEPTED (%d bytes) — the mutations below are therefore "
          "testing what they claim to test\n" % len(blob))

    failures = 0
    with tempfile.TemporaryDirectory() as td:
        for name, fn, expect in MUTATIONS:
            path = os.path.join(td, "m.bin")
            with open(path, "wb") as fh:
                fh.write(fn(blob))
            rc, err = run_decoder(path)
            got = "refuse" if rc == 2 else ("accept" if rc == 0 else "error%d" % rc)
            ok = (got == "refuse") if expect == "refuse" else (got == "accept")
            if not ok:
                failures += 1
            mark = "ok  " if ok else "FAIL"
            reason = err.replace("‼ REFUSED: ", "").split(".")[0][:96]
            print("%s %-36s expect=%-6s got=%-6s %s"
                  % (mark, name, expect, got, reason if got == "refuse" else ""))

    print()
    if failures:
        print("‼ %d mutation(s) did not behave as predicted" % failures)
        return 1
    print("all %d mutations behaved as predicted "
          "(13 refused, 1 documented-inert)" % len(MUTATIONS))
    return 0


if __name__ == "__main__":
    sys.exit(main())
