# Budget rationale and cheaper-object options

CODE READ, `V3_SPARSE_TWIN.md` Bounds:8192 blocks per space and32768 reservation objects per session are design constants enforcing finite guest-reachable host allocation. They are not measured RM capacity limits. A reservation's sparse attribute applies to its whole range; a reservation object is not a cheap per-leaf Boolean.

INFERRED from the fixed grain in `V3_SPARSE_TWIN.md`: exact64KiB sparse tiles require16384 objects perGiB, before normal/coarse sibling objects. Uniform2MiB reservations would require512 objects perGiB. This arithmetic does not establish their native memory cost or prove a larger reservation model safe under later partial transitions.

CODE READ, `highwater-available.json`: the retained local evidence does not provide complete per-workload, per-space demand for all nine SPEC workloads, Heaven and Steel. Existing extracted heartbeat session counters include asymmetric creation/release events and cannot supply that demand. The new exact current gauge and bounded per-space peak census address future collection. Native RM costs have not been measured; no capacity result or new numeric quota is claimed here.

| Option | Potential benefit | Required bound and semantic condition |
|---|---|---|
| Empty-normal recycling (implemented) | Reclaims objects spent on dead touched spans | Existing8192/32768 limits; at most4 pairs per call; no live mapping/wanted coverage released |
| Larger adjacent uniform sparse reservations | Fewer objects when a whole region is sparse | Finite objects/bytes; exact guest-derived coverage; safe later splitting without faulting unchanged VAs is required |
| Refine only where a sparse transition needs it | Avoids eager normal-tile objects | Finite pending intent; an occupied coarse block may prevent later refinement, so fidelity needs native validation |
| Per-VM/session shared quota with per-space fairness | Uses idle quota for the busy workload | Keep a hard global native-allocation ceiling and a fair per-space admission bound; do not permit one guest to exhaust the host |
| Guest-VRAM-scaled quota | Relates quota to configured demand | Absolute host object/memory/time ceiling still required; VA span and sparse demand can exceed VRAM |

INFERRED from the missing data in `highwater-available.json` and the unrun `PREREG.md` arms: retain the current constants while validating recycling and collecting the preregistered native cost curve and demand high-water. Choose headroom from observed peak demand and actual allocation/free/memory/refusal costs, including desktop impact. Do not infer an unlimited budget from virtual address size, and do not coalesce active sparse coverage by temporarily releasing unchanged VAs. The prepared native benchmark is a measurement tool, not authorization to run it.
