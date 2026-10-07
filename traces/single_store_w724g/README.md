# §w724g — is the BAR1/BAR2 trap path REACHABLE?

**Measured 2026-09-14**, vast instance `51054651` (RTX 3060 / GA106, driver **580.159.04**),
binary **`kayfabe-rev:54995fad5acd9fe41b1391818be2dd271eaf9245`** = the tree revision.

⊘ **This is not the backing switch.** `page_backing` still returns memfd leaves and BAR1/BAR2
are still served out of the aperture store. Increments 3 and 6 are blocked — see
`SINGLE_STORE_PLAN.md`'s correction blocks. This boot answers the one question §w724g says must
be answered **before** any of that can be deleted.

## The result

| arm | `FB_TRAP_REFUSALS` | raw client |
|---|---|---|
| `KAYFABE_FB_TRAP=serve` (control) | **0**, by construction | **(P)** 8/8, `MEAN_FALSIFIER=PASS` |
| `KAYFABE_FB_TRAP=refuse` | **0**, **measured** | **(P)** 8/8, `MEAN_FALSIFIER=PASS` |

```
FB-TRAP AT END OF RUN: arm=refuse FB_TRAP_REFUSALS=0 ⇒ ★★★ THE TRAP PATH WAS NEVER REACHED.
Publication covered every BAR1/BAR2 access this boot made, so `zero in practice` becomes
`unreachable` and increment 7's deletion is licensed.
```

★★★ **With the backstop REMOVED, a full raw-client run (8 threads × 8 rounds, with the mean
falsifier) never reached the trap path once.** §w721b's distinction — *"zero in practice"*
versus *"impossible by construction"* — is settled for this workload in the second direction.

⇒ **The demand-fill mirror is not load-bearing for this workload** and increment 7 may delete
it, under §w724g's own rule that the licence is the **guest** suite, not the host's.

## ⚠ What this does NOT license

1. **One workload, not the suite.** §w724g is explicit: *"the gate for unwiring is the GUEST
   suite"* — `rmladder_suite.sh`, all 30 arms. This boot ran the mean client. A green here is
   necessary and not sufficient.
2. **Not CUDA, not the LLM.** Those touch different BAR1 populations.
3. **Zero is a measurement here and 0 in the control is not.** The control arm's 0 is by
   construction and measures nothing; it is present so the armed arm's 0 can be read as a
   difference rather than as a number in isolation.

★ The zero is trustworthy because the refusal is known to fire:
`two_worlds_split::the_armed_trap_path_refuses_by_name_and_counts_it` makes it fire on the same
path a trapped guest access takes, asserts it is *that* refusal by its own sentence, asserts it
is counted, and runs the unarmed control first.

## ★★ The other finding — §22 item 3's sizing relation, on this board

```
BAR1-BUDGET host_bar1=256 MiB at 0000:00:07.0 advertised_guest_bar1=256 MiB headroom=16 MiB
  ⇒ ⊘⊘ DOES NOT FIT — 256 MiB advertised + 16 MiB headroom > 256 MiB host.
```

⇒ **The all-resident BAR1 design cannot work on this board at the size we currently advertise.**
§22(b): *"we CHOOSE the left-hand side"* — the guest's aperture is advertised by us, not
inherited. Advertising **128 MiB** leaves 128 MiB of headroom on this same board, and a
128 MiB-BAR1 GA106 is a real hardware configuration — a different truthful board, not a lie, so
§22 stays intact.

⊘ Queried from `/sys/bus/pci/devices/*/resource`, never compared against a literal (§22(b):
*"256 MiB is ONE measured board, not a spec … a queryable property"*). No device descriptor is
involved, so this stays on the side of decision (b) that needs no ruling.

## Files

- `boot_e36ctl.log` / `boot_e36ref.log` — the harness output with its pre-registered outcomes.
- `census_lines.txt` — the `FB-TRAP` and `BAR1-BUDGET` lines from both boots.
- `run_e36*_dmesg.log` — the guest driver's own ring buffer.

⊘ The QEMU logs are ~7 MB each and are not committed; the lines carrying the result are above.
