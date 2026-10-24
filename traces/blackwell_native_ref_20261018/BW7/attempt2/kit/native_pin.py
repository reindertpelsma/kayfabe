"""Native staged-policy API backed by the unchanged pre-grant settle behavior (RED)."""
import console_pin as C

def settle_pin(scan, fetch_console, *, first_reachable_at, clock, sleep, record):
    return C.settle_pin(scan, fetch_console, elapsed_since_create=0, factor=10,
                        clock=clock, sleep=sleep, record=record)
