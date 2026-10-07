# w744 — B1(d) hinge probe: PREDICTIONS, committed BEFORE the probe has produced a line

**STATUS: LIVE, 2026-09-15.** Written and committed **before any output exists**. The box
(`vast 51133827`, RTX 3060 / GA106, driver `580.159.03`) is rented and the binary is built, and
`--dup-vaspace-probe` has **not** been run at this revision. Each row carries the value that
**refutes** it.

⊘ Predictions are not evidence. They exist so that a result cannot be read as whatever was
convenient afterwards — this campaign's `fix_the_criterion_before_the_boot` lesson.

| # | line | predicted | ⊘ REFUTED BY |
|---|---|---|---|
| 1 | `B1D_CLIENT_S` vs `B1D_CLIENT_P` | **different** | equal ⇒ not two clients, every row below is vacuous, rung prints `NOTRUN` |
| 2 | ★★★ `B1D_Q1A_DUP_VASPACE status=` | **`0x0000`** (`NV_OK`) — `FERMI_VASPACE_A` is a `RsResource` like any other and RM's dup path is class-generic | non-zero ⇒ route B is dead. `0x33` = `INVALID_OBJECT_HANDLE`, `0x1B` = `INSUFFICIENT_PERMISSIONS` (the interesting refusal: it would mean dup is privilege-gated, not class-gated) |
| 3 | ★★ `B1D_Q1B_DUP_VIRTUAL status=` | **`0x0000`** | non-zero ⇒ route A is dead |
| 4 | ★★★★★ `B1D_HINGE=` | **`PASS`** (at least one route) | `FAIL` ⇒ **(d) as written cannot be built, and that is the full deliverable.** Stop, do not improvise |
| 5 | ★★★ `B1D_Q1C_RANGE_OVER_DUPED status=` | **`0x0000`** — a range is parented at the *device*, and `hVASpace` is a parameter, not a parent | non-zero ⇒ route B needs the space to be locally created, so only route A survives |
| 6 | ★★★ `B1D_Q2A_MAP_DUPED_HDMA status=` | **`0x0000`**, `placed_as_asked=true` | non-zero ⇒ RM requires `hDma` and `hMemory` to share a client ⇒ **brief question 2 answered NO** |
| 7 | ★★★ `B1D_Q2B_MAP_LOCAL_RANGE_OVER_DUPED_SPACE status=` | **`0x0000`**, `placed_as_asked=true` | non-zero ⇒ route B cannot map ⇒ only route A survives |
| 8 | `B1D_WORKING_ROUTE=` | **`B (local range over duped space)`** — preferred, because S's `hDma` is then S's own object and the map names nothing foreign | `A` is acceptable; `none` ⇒ `(F)` |
| 9 | ★★★ `B1D_Q3_REQUIRED_HONOURED=` | **`3/3`** — the raw client's own ring VAs (`0x80_0000_1000`, `0x82_0000_1000`, `0x86_0000_1000`), the addresses production must place | `<3` ⇒ **brief question 3 answered NO for the addresses that matter**; FIXED is constrained and the data plane needs a different answer |
| 10 | `B1D_Q3_INFORMATIONAL_HONOURED=` | **not predicted** — a fact about RM's VA layout, deliberately outside the verdict in **both** directions | — |
| 11 | ★★ `B1D_Q3_KNOWN_POSITIVE=` | **`FIRED`** — a VA past RM's default VAS limit (`0x100_0000_0000`) is refused | `DID NOT FIRE` ⇒ every "success" above is unmeasured: a sweep in which nothing can fail is not measuring placement |
| 12 | ★★ `B1D_Q4_CONTROL_OK=` | **`true`** — P maps its own object at a VA nobody claimed | `false` ⇒ the collision below says nothing; `B1D_SHARING=INDETERMINATE` |
| 13 | ★★★★★ `B1D_SHARING=` | **`ONE SPACE`** — P's map at a VA S already occupied is **REFUSED** (expect `0x51 NV_ERR_NO_MEMORY`, the status the C reads as *"the VA is ALREADY mapped"*) | **`TWO SPACES`** ⇒ the dup made an independent copy; the stub's channels would walk different page tables than the scratchpad mapped ⇒ **(d) is void however green rows 2-9 are. Stop and report.** |
| 14 | `B1D_PROBE=` | **`(P)`** | `(F)` — read rows 4, 9, 11, 12, 13 to see which clause failed; `(F)` is a **finding**, not a failure |

## ⊘ What this probe CANNOT say, stated before it runs

- It is **one driver on one chip** (`580.159.03`, GA106). The bench ran `580.159.04 OPEN`.
  Same branch, not the same build.
- A green `B1D_SHARING` shows the two clients **share a VA namespace**. It does **not** show a
  channel born by P executes against S's mapping — that needs a submission, and this rung
  deliberately does not build one. ★ Scoping it now so a green cannot later be cited as more.
- It says nothing about `Worker::execute`'s `ForeignHandle` gate or about
  `RING_NOT_A_JOINED_WINDOW`. Those are **host-side kayfabe gates**, not RM, and no RM probe
  can answer them.
