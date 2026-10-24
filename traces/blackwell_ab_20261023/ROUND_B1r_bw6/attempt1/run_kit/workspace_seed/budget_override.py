"""Named per-lease budget override for round B1j (controller 04:20); defaults unchanged.

This lease only: stop 3.00 USD, cap 3.50 USD, price <= 1.00 USD/h; the 8 USD credit reserve and
every other bound stay. The 90-minute floor is computed from this lease's own stop.
"""

import round_budget as B

DEFAULT_STOP_USD = 2.0
DEFAULT_CAP_USD = 2.5
STOP_USD = 3.0
CAP_USD = 3.5
MAX_RATE = 1.0
RESERVE_USD = B.RESERVE_USD


def price_ok(offer):
    return float(offer.get("dph_total", 99)) <= MAX_RATE


def plan(hourly, credit, armed_at):
    if hourly > MAX_RATE:
        raise ValueError("hourly price exceeds this lease's 1.00 USD/h limit")
    saved = (B.STOP_USD, B.CAP_USD)
    try:
        B.STOP_USD, B.CAP_USD = STOP_USD, CAP_USD
        return B.plan(hourly, credit, armed_at, stop_usd=STOP_USD, cap_usd=CAP_USD)
    finally:
        B.STOP_USD, B.CAP_USD = saved


def fields():
    return {
        "budget_override": {
            "stop_usd": STOP_USD,
            "cap_usd": CAP_USD,
            "max_rate": MAX_RATE,
            "authority": "controller 04:20 B1j",
        }
    }
