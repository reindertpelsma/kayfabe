#!/usr/bin/env python3
"""Print an NVIDIA notice as comment lines, for the generators to put at the top of their output.

    notice.py ogkm '#'           NVIDIA open-gpu-kernel-modules (MIT), `#` comments
    notice.py open-gpu-doc '//'  NVIDIA open-gpu-doc (MIT), `//` comments

The texts are `nvidia_ogkm_mit.txt` and `nvidia_open_gpu_doc_mit.txt` next to this file; they are
also quoted in `THIRD_PARTY_NOTICES.md` (a test keeps the two in step). A blank line of the text
becomes a bare marker, so the block has no trailing spaces.
"""

import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SOURCES = {
    "ogkm": HERE / "nvidia_ogkm_mit.txt",
    "open-gpu-doc": HERE / "nvidia_open_gpu_doc_mit.txt",
}


def comment_block(source, marker):
    """The notice for `source` as a list of lines, each behind `marker`."""
    text = SOURCES[source].read_text()
    return [f"{marker} {line}" if line else marker for line in text.splitlines()]


def main(argv):
    if len(argv) != 3 or argv[1] not in SOURCES:
        sys.stderr.write(f"usage: notice.py {{{'|'.join(SOURCES)}}} <comment-marker>\n")
        return 2
    for line in comment_block(argv[1], argv[2]):
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
