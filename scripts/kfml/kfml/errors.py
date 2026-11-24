"""Exceptions a workload may raise; the child maps them to a row status."""


# The reasons a NOT_RUN row may be a permitted skip under a strict payload instruction (a row may skip only for a limit outside kayfabe). This tuple is the whole vocabulary: the payload (scripts/ci/vast_payload.py) loads it from here and grades a row's
# `skip_code` by membership, never its free-text `reason`. A NOT_RUN row without one of these codes fails a strict run.
SKIP_CODES = (
    "HOST_RAM_TOO_SMALL",          # the host's RAM cannot hold what the test needs (managed_oversub: 1.25 x VRAM in 60% of MemAvailable)
    "VRAM_TOO_SMALL",              # the card's total VRAM is below the test's floor and kayfabe cannot assign more
    "DIE_LACKS_FEATURE",           # the die lacks a hardware feature the test needs (compute capability below the floor)
    "NOT_RUNNABLE_ON_BARE_METAL",  # the case cannot surely run on bare metal either (no row raises it yet)
)


class NotRun(Exception):
    """The hardware (or a documented known-unsupported feature) cannot run this test. `code` is one of SKIP_CODES or empty (empty is never
    a permitted skip)."""

    def __init__(self, reason, code=""):
        if code != "" and (type(code) is not str or code not in SKIP_CODES):
            raise ValueError("unknown skip code %r (known: %s)" % (code, ", ".join(SKIP_CODES)))
        super().__init__(reason)
        self.code = code


class EnvMissing(Exception):
    """The software environment lacks something; reported as FAIL, never silently green."""


class CheckFailed(AssertionError):
    """A hard self-check failed."""
