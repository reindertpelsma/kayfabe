# The all-DMA baseline — what passed while guest "vidmem" was host memory

**STATUS: LIVE (2026-09-14).** Recorded at the owner's request *("record the commit the raw
client did pass with everything DMA since thats still useful")* **before** BAR1 is moved onto the
reserved device-local object, so the comparison survives the change.

## The commit

**`2ddce6e2`** — *"w711: goal 8's OUTCOME is met without the reactor"*. At this revision every
graded workload passes while **every framebuffer page — BAR1 included — is host memory reached
over PCIe** (`FbPageBacking::Joined` leaves are `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` over a memfd).

| workload | result | at |
|---|---|---|
| raw client, mean test | `W392D_GUEST_OUTCOME=(P)`, `MEAN_FALSIFIER=PASS`, 8/8 threads | w708 |
| **two** raw clients, concurrent | `TWOCLIENT_OUTCOME=(P)`, 28 overlapping graded intervals | w711 / `2ddce6e2` |
| CUDA `cup3` | `CUP3_VAL=43` | w708 |
| LLM | `LLM_OK=1 LLM_TOKENS=16`, 22 671 doorbells | w710 |

All with `TRAP_FILLS=0` and vCPU blocking confined to the sanctioned boot-time PRAMIN re-points.

## ★★★ Why this is worth keeping rather than superseding

This is the **negative control for constraint 18**. It is the measured proof that a DMA-mapped
sysmem page presented as video memory is **correct in value**: four workloads, including an LLM
producing real tokens, all green. ⇒ Any test that would also pass here **cannot** be the falsifier
for the BAR1 work — which is precisely why `--ce-client` is not one, and why the falsifier must
measure **residence or bandwidth**, never data correctness.

⊘ It also bounds the claim in the other direction. *"BAR1-as-sysmem is the parity blocker"* is a
**hypothesis with a mechanism, not a measurement**, at the time of writing: the LLM's throughput
here was never compared against the same runner on the host. Until that ratio exists, this table
says only *"it works"*, never *"it works at what cost"*.

## ⚠ The blemish the `(P)` does not carry

`2ddce6e2` passes **with one host `Xid 31 MMU Fault: ENGINE CE0 HUBCL` per client** (`HOST_DMESG_XID=2`
for two clients; one client produces exactly one, so concurrency is not its cause). The clients
content-verify and grade `(P)` regardless. ⇒ A green grade at this revision is green **on content**,
and the fault is a separate open defect — do not read this baseline as a clean floor.
