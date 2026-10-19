"""Parse an EVTX public timestamp for a bounded evidence window."""
import datetime

def event_time(utc):
    return datetime.datetime.fromisoformat(utc.replace('Z','+00:00'))
