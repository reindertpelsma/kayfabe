"""Inactive seam for pre-enable actual-state observation; never changes the Xid gate."""
def observe(phase, arm, fetch, ready, record, validate, alive, *, attempts=120, pause=lambda:None):
    return False
