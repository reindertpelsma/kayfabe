"""Benchmark shutdown census seam; no product or guest-execution changes."""
class CensusRefused(RuntimeError):
    pass

def await_shutdown(owned_alive, census, xids, baseline, sleep, clock):
    if tuple(xids()) != tuple(baseline):
        raise CensusRefused('new Xid during shutdown')
    if owned_alive() or census():
        raise CensusRefused('QEMU still present')
    if tuple(xids()) != tuple(baseline):
        raise CensusRefused('new Xid during shutdown')
    return {'rereads':0,'elapsed_seconds':0}
