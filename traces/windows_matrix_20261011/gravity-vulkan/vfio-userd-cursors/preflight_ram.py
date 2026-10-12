#!/usr/bin/env python3
"""Paused GPU-free QEMU to verify the collector's actual RAM alias/fd profile."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
from sample_userd import aliases, fields, qmp, resolve

ap=argparse.ArgumentParser(description=__doc__)
ap.add_argument('qemu',type=Path)
ap.add_argument('output',type=Path)
args=ap.parse_args()
if subprocess.run(['pgrep','qemu-system'],stdout=subprocess.DEVNULL).returncode==0:
    raise RuntimeError('another QEMU exists')
args.output.mkdir(parents=True,exist_ok=False)
sock=args.output/'qmp.sock'
with (args.output/'qemu.log').open('w') as log:
    process=subprocess.Popen([str(args.qemu),'-machine','pc,accel=kvm,memory-backend=ram0',
        '-object','memory-backend-memfd,id=ram0,size=8G,share=on','-m','8192','-smp','1',
        '-S','-nodefaults','-display','none','-qmp',f'unix:{sock},server=on,wait=off'],stdout=log,stderr=log)
    try:
        for _ in range(100):
            if sock.exists(): break
            if process.poll() is not None: raise RuntimeError('paused QEMU exited')
            time.sleep(.05)
        size=qmp(sock,'qom-get',dict(path='/objects/ram0',property='size'))
        layout=qmp(sock,'human-monitor-command',{'command-line':'info mtree'})
        (args.output/'mtree.txt').write_text(layout)
        ranges=aliases(layout,size)
        matches=[p for p in (Path('/proc')/str(process.pid)/'fd').iterdir()
                 if os.readlink(p).startswith('/memfd:memory-backend-memfd') and p.stat().st_size==size]
        if len(matches)!=1: raise RuntimeError('unexpected guest RAM memfd')
        offsets=[]
        with matches[0].open('rb',buffering=0) as ram:
            for begin,end,base in ranges:
                offset=resolve(ranges,end-4096,4096,size)
                value=fields(os.pread(ram.fileno(),4096,offset))
                if any(s[k] for s in value for k in ['get','put','reference']):
                    raise RuntimeError('paused RAM fields unexpectedly nonzero')
                offsets.append(offset)
        result=dict(result='PASS',gpu='absent',qemu_paused=True,ram_bytes=size,ranges=ranges,offsets=offsets)
        (args.output/'result.json').write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps(result))
    finally:
        if process.poll() is None:
            try: qmp(sock,'quit')
            except (OSError,ValueError): process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=10)
