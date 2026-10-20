#!/usr/bin/env python3
"""Replace one known raw-client entry in newc; never extract guest paths."""
import argparse
import gzip
from pathlib import Path


def replace_client(data, client):
    offset, found, result = 0, 0, []
    while offset < len(data):
        header = data[offset:offset + 110]
        if len(header) != 110 or header[:6] != b"070701":
            raise ValueError("expected newc archive header")
        fields = [int(header[i:i + 8], 16) for i in range(6, 110, 8)]
        size, namesize = fields[6], fields[11]
        name_start = offset + 110
        name_end = name_start + namesize
        payload_start = (name_end + 3) & ~3
        end = (payload_start + size + 3) & ~3
        if namesize < 1 or name_end > len(data) or end > len(data):
            raise ValueError("truncated newc entry")
        name = data[name_start:name_end]
        if not name.endswith(b"\0"):
            raise ValueError("unterminated archive name")
        name = name[:-1]
        if name in (b"bin/kftool", b"./bin/kftool", b"bin/rmladder", b"./bin/rmladder"):
            if fields[1] & 0o170000 != 0o100000 or fields[4] != 1:
                raise ValueError("client must be a regular unlinked file")
            found += 1
            result.append(header[:54] + f"{len(client):08x}".encode() + header[62:])
            result.append(data[name_start:payload_start])
            result.append(client + b"\0" * (-len(client) % 4))
        else:
            result.append(data[offset:end])
        offset = end
        if name == b"TRAILER!!!":
            result.append(data[offset:])
            break
    if found != 1:
        raise ValueError(f"expected one client entry, found {found}")
    return b"".join(result)


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("source")
    p.add_argument("client")
    p.add_argument("output")
    args = p.parse_args()
    result = replace_client(gzip.decompress(Path(args.source).read_bytes()), Path(args.client).read_bytes())
    with Path(args.output).open("xb") as out:
        out.write(gzip.compress(result, compresslevel=1, mtime=0))
