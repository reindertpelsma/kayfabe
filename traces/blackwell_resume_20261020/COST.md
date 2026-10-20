# Cost — Blackwell resume 2026-10-20

STATUS: LIVE, 2026-10-20 (closed).

| Own id | Offer | GPU / site | Quote $/h | Created (UTC) | Destroyed / API-absent (UTC) | Runtime | Invoice |
|---|---|---|---|---|---|---|---|
| 56909569 | 56894506 | RTX 5090 (GB202), Taiwan, 128 vCPU, 175 GB | 1.1347 (+0.05 storage) | 15:04:21 | 19:08:58 / 19:09:18 | 4.06 h | GPU 4.553 + storage 0.203 = **4.756 USD** |

- One lease only; auto-destroy timer (360 min, `/root/night/arm_autodestroy.sh`) armed at creation and
  cancelled only after API absence was verified. `vastai show instances` after destroy: `[]`.
- Shared-account credit: 35.874988 (15:02 UTC, before) → 31.119007 (before destroy) → 31.106745 (after).
  The 4.756 USD delta equals this lease's invoice; the account is shared, so the credit delta is not by
  itself attribution.
- Cap 12 USD / stop 10 USD: respected (4.756 USD). No other lease; no trusted-host access.
