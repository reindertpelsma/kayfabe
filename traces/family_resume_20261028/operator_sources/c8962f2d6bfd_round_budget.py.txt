"""Fixed pre-allocation cost plan for the owner's one-lease B1 round."""

import datetime as dt
import math

STOP_USD = 1.50
CAP_USD = 2.00
RESERVE_USD = 8.00
MAX_RATE = 0.60
MAX_SECONDS = 120 * 60
NONRUNTIME_MARGIN = 0.50
TEARDOWN_SECONDS = 60


def positive_number(value, name):
    if type(value) not in (float, int) or not math.isfinite(value) or value <= 0:
        raise ValueError(f"{name} must be a finite positive number")
    return float(value)


def aware(value):
    if not isinstance(value, dt.datetime) or value.utcoffset() is None:
        raise ValueError("round clock must include its timezone")
    return value.astimezone(dt.timezone.utc)


def plan(hourly, credit, armed_at, *, stop_usd=STOP_USD, cap_usd=CAP_USD, transfer_usd=0.0):
    hourly = positive_number(hourly, "hourly price")
    credit = positive_number(credit, "credit")
    armed_at = aware(armed_at)
    stop_usd = positive_number(stop_usd, "round stop")
    cap_usd = positive_number(cap_usd, "round cap")
    if not stop_usd < cap_usd <= CAP_USD or stop_usd > STOP_USD:
        raise ValueError("round overrides must preserve or lower the existing ceilings")
    if hourly > MAX_RATE:
        raise ValueError("hourly price exceeds the owner limit")
    rate_bound = max(hourly * 1.15, hourly + 0.10)
    if (
        type(transfer_usd) not in (int, float)
        or not math.isfinite(transfer_usd)
        or transfer_usd < 0
    ):
        raise ValueError("invalid transfer allocation")
    cost_seconds = int((stop_usd - NONRUNTIME_MARGIN - transfer_usd) * 3600 / rate_bound)
    seconds = min(MAX_SECONDS, cost_seconds - TEARDOWN_SECONDS)
    if seconds <= 0:
        raise ValueError("no runtime remains within the cost stop")
    reservation = rate_bound * seconds / 3600 + NONRUNTIME_MARGIN + transfer_usd
    if reservation >= stop_usd or credit - reservation < RESERVE_USD:
        raise ValueError("round reservation exceeds stop or account reserve")
    return {
        "credit_before": credit,
        "transfer_usd": transfer_usd,
        "nonruntime_margin_usd": NONRUNTIME_MARGIN,
        "armed_utc": armed_at.isoformat(),
        "deadline_utc": (armed_at + dt.timedelta(seconds=seconds)).isoformat(),
        "seconds": seconds,
        "quoted_rate": hourly,
        "rate_bound": rate_bound,
        "reservation_usd": reservation,
        "credit_after_reservation": credit - reservation,
        "stop_usd": stop_usd,
        "cap_usd": cap_usd,
        "reserve_usd": RESERVE_USD,
        "leases_authorized": 1,
    }


def remaining(row, now):
    now = aware(now)
    armed = aware(dt.datetime.fromisoformat(row["armed_utc"]))
    deadline = aware(dt.datetime.fromisoformat(row["deadline_utc"]))
    if now < armed or deadline - armed != dt.timedelta(seconds=row["seconds"]):
        raise ValueError("round clock or fixed deadline changed")
    return max(0, int((deadline - now).total_seconds()))
