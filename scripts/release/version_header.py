#!/usr/bin/env python3
"""Generate a C string macro from the sole canonical version file."""
from pathlib import Path
import re
import sys

version=Path(sys.argv[1]).read_text().strip()
if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.]+)?',version):
    raise SystemExit('invalid canonical VERSION')
Path(sys.argv[2]).write_text('#define KAYFABE_VERSION "'+version+'"\n')
