#!/usr/bin/env python3
import os,re,sys
from pathlib import Path
rev=Path(__file__).parent.name
assert re.fullmatch('[0-9a-f]{8}',rev)
binary='/workspace/bench/kf3-bins/'+rev+'/qemu-system-x86_64'
os.execve(binary,[binary]+sys.argv[1:],os.environ)
