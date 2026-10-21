"""Read only the public DUMP_HEADER64 metadata of a Windows dump: bugcheck code, P1..P4, SystemTime.

Public layout (Microsoft DUMP_HEADER64): BugCheckCode @56 (u32), BugCheckParameter1..4 @64,72,80,88 (u64),
SystemTime @4008 (FILETIME). No memory, context or module bytes are read or exported.
Usage: dump_header.py FILE.dmp   (prints one JSON object)
"""
import datetime
import json
import struct
import sys

HEADER_BYTES = 4016


def parse(raw: bytes) -> dict:
    if len(raw) < HEADER_BYTES or raw[:8] != b"PAGEDU64":
        raise ValueError("not a PAGEDU64 dump header")
    code = struct.unpack_from("<I", raw, 56)[0]
    params = [struct.unpack_from("<Q", raw, 64 + 8 * i)[0] for i in range(4)]
    ft = struct.unpack_from("<Q", raw, 4008)[0]
    t = datetime.datetime(1601, 1, 1, tzinfo=datetime.timezone.utc) + datetime.timedelta(microseconds=ft // 10)
    return {
        "bugcheck": hex(code),
        "parameters": [hex(p) for p in params],
        "system_time_filetime": str(ft),
        "system_time_utc": t.isoformat(),
    }


def main(argv):
    with open(argv[1], "rb") as f:
        print(json.dumps(parse(f.read(HEADER_BYTES))))


if __name__ == "__main__":
    main(sys.argv)
