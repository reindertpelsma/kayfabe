"""Conservative clock analysis: keep a sample only for every observed offset bound.

No device operations. Bounds are host_epoch minus guest_epoch from explicit
request/response brackets. Their union covers all observed clock uncertainty;
no narrower intersection or fixed point is substituted for that uncertainty.
"""
import math

def envelope(bounds):
    if not bounds:raise ValueError('NO_CLOCK_OBSERVATIONS')
    if any(not math.isfinite(lo) or not math.isfinite(hi) or lo>hi for lo,hi in bounds):raise ValueError('INVALID_CLOCK_BRACKET')
    return min(lo for lo,hi in bounds),max(hi for lo,hi in bounds)

def certified_interval(start,end,bounds,edge=1):
    lo,hi=envelope([bounds])
    if not math.isfinite(start) or not math.isfinite(end) or edge<0 or end<=start:raise ValueError('INVALID_SUBTEST_INTERVAL')
    first,last=start+hi+edge,end+lo-edge
    return (first,last) if first<last else None

def certified_inside(t,start,end,bounds,edge=1):
    interval=certified_interval(start,end,bounds,edge)
    return interval is not None and interval[0]<=t<interval[1]
