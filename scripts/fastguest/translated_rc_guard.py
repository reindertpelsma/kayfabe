#!/usr/bin/env python3
"""Bounded intentional-fault evidence gate; the default interface refuses."""


class Refused(ValueError):
    """The capture does not establish the requested error delivery."""


def check(manifest, qemu, guest, journal):
    """Check independent host and guest capture sources."""
    raise Refused("negative-control evidence gate not implemented")
