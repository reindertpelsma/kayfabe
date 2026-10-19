#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Stream one exact, bounded image range without seeking on stdout (which is a pipe)."""
import os
import stat
import sys


def stream(path, offset, count):
    if not 0 <= offset <= (1 << 63) - 1 or not 0 < count <= 1 << 30:
        raise ValueError('image range outside bounds')
    fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or offset + count > info.st_size:
            raise ValueError('image range outside regular file')
        left = count
        while left:
            block = os.pread(fd, min(left, 131072), offset)
            if not block:
                raise OSError('image truncated during range read')
            sys.stdout.buffer.write(block)
            offset += len(block)
            left -= len(block)
        sys.stdout.buffer.flush()
    finally:
        os.close(fd)


if __name__ == '__main__':
    try:
        stream(sys.argv[1], int(sys.argv[2]), int(sys.argv[3]))
    except (OSError, ValueError, IndexError) as error:
        print('image range refused: ' + str(error), file=sys.stderr)
        sys.exit(1)
