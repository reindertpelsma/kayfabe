"""Small evidence-integrity primitives; no device operations or policy changes."""
import datetime as dt
import math

def valid_value(row,key):
    if key not in row or str(row.get(key+'_rc',''))!='0':return None
    value=float(row[key])
    if not math.isfinite(value):raise ValueError('NONFINITE_COUNTER')
    return value

def counter_delta(values):
    if len(values)<2:raise ValueError('INSUFFICIENT_CUMULATIVE_SAMPLES')
    if any(not isinstance(x,int) or x<0 for x in values):raise ValueError('INVALID_CUMULATIVE_COUNTER')
    if any(b<a for a,b in zip(values,values[1:])):raise ValueError('CUMULATIVE_COUNTER_RESET')
    return values[-1]-values[0]

def log_epoch(date,clock,guest_capture):
    zone=dt.datetime.fromisoformat(guest_capture).tzinfo
    if zone is None:raise ValueError('MISSING_GUEST_TIMEZONE')
    return dt.datetime.fromisoformat(date+'T'+clock).replace(tzinfo=zone).timestamp()

def clock_bounds(host_before_ns,host_after_ns,guest_epoch_ms):
    if host_after_ns<host_before_ns:raise ValueError('INVERTED_RESPONSE_BRACKET')
    return host_before_ns/1e9-guest_epoch_ms/1e3,host_after_ns/1e9-guest_epoch_ms/1e3

def inside(t,windows,margin=1):
    return any(w['start']+margin<=t<w['end']-margin for w in windows)
