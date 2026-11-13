"""Exceptions a workload may raise; the child maps them to a row status."""


class NotRun(Exception):
    """The hardware (or a documented known-unsupported feature) cannot run this test."""


class EnvMissing(Exception):
    """The software environment lacks something; reported as FAIL, never silently green."""


class CheckFailed(AssertionError):
    """A hard self-check failed."""
