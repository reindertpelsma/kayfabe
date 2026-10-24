"""Operator side: the B1n lease observation authority, built from the guard's pinned table.

The admitted pair is the table pair of the lease's candidate die; a die outside the table
refuses here, before any launch. The Linux arm is never an observation arm.
"""

import b1n_arms

MODE = "bw3-saved-xid32-observation-v1"


def build(*, lease_id, die, start, end, table):
    if die not in table:
        raise ValueError("candidate die has no pinned observation pair: " + str(die))
    b1n_arms.validate()
    pair = list(table[die])
    return {
        "version": 1,
        "lease_id": lease_id,
        "record_limit": 8,
        "arms": b1n_arms.windows_arms(),
        "diagnostic_enable_arms": b1n_arms.diagnostic_enable_arms(),
        "not_before_utc": start.isoformat(),
        "expires_utc": end.isoformat(),
        "authority": "owner 2026-10-24 BW4: per-die exact saved Xid32 OBSERVATION, this lease only",
        "mode": MODE,
        "observation_only": True,
        "attestation_allowed": False,
        "exact_channels": pair,
        "host_verification": "UNVERIFIED-HOST",
        "candidate_die": die,
        "expected_page_donor": die if die in ("GB202", "GB206") else "GB202",
        "page_policy": "own exact row" if die in ("GB202", "GB206") else "inherited-with-reason",
        "allowed_pair_class": "exact " + "/".join(pair) + " intr00200000; max8 per arm",
        "other_faults_errors_or_failed_health": "immediate pidfd stop",
    }
