#!/usr/bin/env python3
"""Publish a completed binary atomically, refusing every existing revision file."""
import os
from pathlib import Path
import shutil
import sys
import tempfile


def publish(source, destination):
    source,destination=Path(source),Path(destination)
    destination.parent.mkdir(parents=True,exist_ok=True)
    descriptor,temporary=tempfile.mkstemp(prefix='.publish-',dir=destination.parent)
    os.close(descriptor)
    try:
        shutil.copyfile(source,temporary)
        os.chmod(temporary,source.stat().st_mode & 0o777)
        os.link(temporary,destination)  # exclusive creation; never replaces an existing inode
    finally:
        Path(temporary).unlink()


if __name__=='__main__':publish(*sys.argv[1:])
