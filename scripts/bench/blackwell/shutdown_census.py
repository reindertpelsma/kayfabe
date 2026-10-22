"""Bounded benchmark-only shutdown observation; never suppress a new Xid."""
class CensusRefused(RuntimeError):
    pass

def await_shutdown(owned_alive, census, xids, baseline, sleep, clock):
    start=clock();deadline=start+60
    def healthy():
        if tuple(xids()) != tuple(baseline):
            raise CensusRefused('new Xid during shutdown')
    while True:
        healthy()
        if not owned_alive():break
        if clock()>=deadline:raise CensusRefused('owned QEMU alive after60s')
        sleep(min(5,deadline-clock()))
    for rereads in range(4):
        healthy()
        pending=census()  # Never reuse a prior process-list result.
        healthy()
        if not pending:return {'rereads':rereads,'elapsed_seconds':clock()-start}
        if rereads<3:sleep(5)
    raise CensusRefused('QEMU process list remains nonempty after3rereads')

def main():
    import argparse,json,os,subprocess,sys,time
    from pathlib import Path
    import phase_xid_guard as G
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('owned_receipt','accepted_receipt','output'):p.add_argument(name)
    a=p.parse_args()
    owner=json.loads(G.N.bounded(a.owned_receipt));pid=int(owner['pid']);assert pid>1
    baseline=json.loads(G.N.bounded(a.accepted_receipt))['approved_native_xid_lines']
    def alive():
        root=Path('/proc')/str(pid)
        try:
            fields=(root/'stat').read_text().rpartition(')')[2].split()
            if int(fields[19])!=int(owner['starttime']):return False
            return (root/'exe').resolve(strict=True)==Path(owner['exe']).resolve(strict=True)
        except FileNotFoundError:return False
    def census():
        r=subprocess.run(['pgrep','-x','qemu-system-x86'],capture_output=True,text=True,timeout=5)
        if r.returncode not in (0,1):raise CensusRefused('process census unavailable')
        return [int(x) for x in r.stdout.split()]
    try:
        result=await_shutdown(alive,census,lambda:G.N.xid_lines(G.N.kernel_snapshot()),baseline,time.sleep,time.monotonic)
        G.atomic_json(a.output,dict(result,verdict='CLEAN',pid=pid,rule='owned exit<=60s; initial+3fresh reads5s apart; no new Xid'))
    except Exception as error:
        G.atomic_json(a.output,{'verdict':'FATAL','error':str(error),'pid':pid});raise
    return 0

if __name__=='__main__':raise SystemExit(main())
