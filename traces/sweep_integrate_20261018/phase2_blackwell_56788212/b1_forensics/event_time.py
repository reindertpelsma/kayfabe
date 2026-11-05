"""Parse an EVTX public timestamp for a bounded evidence window."""
import datetime,re

def event_time(utc):
    # Python3.10 accepts microseconds; retain the original string in the evidence.
    normalized=re.sub(r'(\.\d{6})\d+(?=Z$|[+-]\d{2}:\d{2}$)',r'\1',utc)
    return datetime.datetime.fromisoformat(normalized.replace('Z','+00:00'))
