# Cost — Blackwell resume 2026-10-20

STATUS: LIVE, 2026-10-20 (closed 20:36 UTC).

| Own id | GPU / site | Quote $/h | Created (UTC) | Destroyed / API-absent (UTC) | Runtime | Invoice (re-read 20:36 UTC) |
|---|---|---|---|---|---|---|
| 56909569 | RTX 5090 GB202, Taiwan (machine 38450) | 1.1347 + storage | 15:04:21 | 19:08:58 / 19:09:18 | 4.06 h | GPU 4.640 + storage 0.207 = 4.847 USD |
| 56939139 | RTX 5090 GB202, UK (machine 33614) | 0.9085 + storage | 19:13:30 | 20:35:43 / 20:36:03 | 1.38 h | GPU 1.207 + storage 0.092 = 1.299 USD |
| **total** | | | | | | **6.146 USD** (cap 12, stop 10) |

- Each lease: auto-destroy timer armed at creation, cancelled only after verified API absence; final
  `vastai show instances`: `[]`.
- Shared-account credit: 35.874988 (15:02 UTC) → 29.729421 (20:36 UTC), delta 6.146 = the two invoices.
  The account is shared, so the credit delta alone is not attribution. Invoices may still settle.
- No trusted-host access.
