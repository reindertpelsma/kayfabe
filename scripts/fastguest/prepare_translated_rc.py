#!/usr/bin/env python3
"""Prepare a guest-only probe from pinned public ogkm source."""


def patch_guest(source):
    """Add an observation after the real guest RC notification operation."""
    return source
