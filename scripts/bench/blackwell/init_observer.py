"""Bench-only pre-enable actual CIM observer. Does not change fault admission."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time

IDENTITY=('arm','pid','starttime','exe','pci')

def state(path):
    with Path(path).open() as stream:text=stream.read(65537)
    if len(text)>65536:raise ValueError('phase byte bound')
    return json.loads(text)

def result(text):
    if len(text)>65536:raise ValueError('diagnostic byte bound')
    rows=[line[len('BW580_RESULT '):] for line in text.splitlines() if line.startswith('BW580_RESULT ')]
    if len(rows)!=1:raise ValueError('one actual CIM result required')
    return json.loads(rows[0])

def clean(row, code):
    return type(row.get('code')) is int and row['code']==code and row.get('driver')=='32.0.15.8088' and type(row.get('tdr4101')) is int and row['tdr4101']==0

def observe(phase, arm, fetch, ready, record, validate, alive, *, attempts=120, pause=lambda:None):
    if not 1<=attempts<=120:raise ValueError('observation attempt bound')
    frozen=state(phase)
    if frozen.get('arm')!=arm:raise ValueError('observer belongs to another arm')
    handshaken = False
    for attempt in range(attempts):
        if not alive():return False
        current=state(phase)
        if any(current.get(k)!=frozen.get(k) for k in IDENTITY):raise ValueError('VM identity changed')
        validate(current)
        try:
            text = fetch()
        except subprocess.TimeoutExpired:
            # A timed-out command supplies no usable output or state evidence.
            current = state(phase)
            if any(current.get(k) != frozen.get(k) for k in IDENTITY):
                raise ValueError('VM identity changed during query')
            validate(current)
            if not alive():
                return False
            pause()
            continue
        row=result(text)
        current=state(phase)
        if any(current.get(k)!=frozen.get(k) for k in IDENTITY):raise ValueError('VM identity changed during query')
        validate(current)
        if not alive():
            return False
        if not handshaken:
            if not clean(row,22):raise ValueError('disabled580.88 handshake required before enable')
            ready()
            handshaken = True
        elif clean(row,0):
            record(text)
            return True
        pause()
    return False

def main():
    import phase_xid_guard as G
    p=argparse.ArgumentParser();p.add_argument('phase');p.add_argument('arm');p.add_argument('ready');p.add_argument('diag');p.add_argument('command',nargs=argparse.REMAINDER)
    a=p.parse_args();command=a.command[1:] if a.command[:1]==['--'] else a.command
    if not command:p.error('diagnostic command required')
    end=time.monotonic()+120;expected=state(a.phase)
    def validate(current):
        current=dict(current,expected_pci=current['pci']);G.validate_phase_owner(current)
    def alive():
        return time.monotonic()<end and Path('/proc',str(expected['pid']),'exe').exists()
    def fetch():
        r=subprocess.run(command,stdin=subprocess.DEVNULL,capture_output=True,text=True,timeout=10)
        if r.returncode == 124:
            # The fixed outer `timeout 10` can expire before subprocess.run does.
            raise subprocess.TimeoutExpired(command, 10)
        if r.returncode:raise ValueError('diagnostic command failed: '+str(r.returncode))
        return r.stdout
    def ready():
        G.atomic_json(a.ready,dict(arm=a.arm,pid=expected['pid'],starttime=expected['starttime'],disabled_handshake=True))
    def record(text):
        Path(a.diag).write_text(text)
        G.record_windows_init(a.phase,a.diag,a.arm)
    print('WINDOWS_INIT_OBSERVER_START',a.arm,flush=True)
    ok=observe(a.phase,a.arm,fetch,ready,record,validate,alive,pause=lambda:time.sleep(1))
    print('WINDOWS_INIT_OBSERVER_END code0='+str(int(ok)),flush=True)
    return 0 if ok else 2

if __name__=='__main__':sys.exit(main())
