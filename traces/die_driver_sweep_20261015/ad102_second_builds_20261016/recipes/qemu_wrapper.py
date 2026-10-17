#!/usr/bin/env python3
import os,re,sys
from pathlib import Path
rev=Path(__file__).parent.name
assert re.fullmatch('[0-9a-f]{8}',rev)
binary='/workspace/bench/kf3-bins/'+rev+'/qemu-system-x86_64'
extra=['-drive','file=/root/ada2build-payload.iso,if=none,id=kfpayload,format=raw,media=cdrom,readonly=on','-device','ide-cd,drive=kfpayload,bus=ide.1,id=kfpayloadcd']
os.execve(binary,[binary]+sys.argv[1:]+extra,os.environ)
