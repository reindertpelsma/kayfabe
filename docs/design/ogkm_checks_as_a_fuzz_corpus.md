# ogkm's own refusals as our fuzz corpus

**STATUS: LIVE, 2026-09-18 (w760). ✔ The two LIVE BUGS (#1, #2) are FIXED in w760h and
regression-tested on hardware (78/78, container 51417213, RTX 3090 / 580.95.05). Cases
#3-#9 below are OPEN.**

Owner, 2026-09-18:

> *"in fact, for a fuzz test, trying to do stuff that fails in ogkm checks is the best way to
> test malicious input, its guaranteed not happy path you see when running any workload"*

The premise, restated: **a malicious guest runs none of the vendor's checks.** RM is not in the
loop — the guest writes page-table bytes directly, so every condition RM validates before
writing a table is a condition our walker must assume is violated. That makes ogkm's rejection
set a corpus that is *guaranteed* off the happy path, which no benign workload can produce and
therefore no differential against a benign workload can cover. See `THE_CONSTRAINTS.md` §39(b).

The mining was done against `research_clones/ogkm-580.159.04`. The richest source turned out
**not** to be the fault/translate walker but the **UVM encoders**
(`kernel-open/nvidia-uvm/uvm_{pascal,turing,ampere}_mmu.c`) — the code that actually authors
GA10x VER2 tables — because they document *which bit patterns mean something other than what
they look like*.

---

## ★★★★★ TWO LIVE CORRECTNESS BUGS — reachable by an HONEST guest  ✔ FIXED w760h

Both are one rule — **the big half can veto the small one** — implemented as
`kf_big_half_vetoes_small` in the serial and parallel duals, and covered by
`ogkm/unmapped_big_pte_hides_4k` and `ogkm/sparse_big_half_hides_small`, each with a control
(`dual_control_runs`) because *"0 runs"* is also what a tree that never got built looks like.

These are the important half of the result, and they are **not** fuzz gaps. Both make us report
a **wrong mapping** from tables a stock driver writes in the ordinary course of business.

### 1. The "unmapped" big PTE (`0x20`) — hardware stops, we keep walking

`uvm_mmu.h:203-212`:

> *"**Invalid** big PTEs indicate to the GPU MMU that there might be 4k PTEs present instead…
> **Unmapped** big PTEs indicate that there are **no 4k PTEs below the unmapped big entry, so
> MMU should stop its walk and not cache any 4k entries which may be in memory**… means we
> don't have to initialize the 4k PTEs which are covered by big PTEs since the MMU will never
> read them."*

The encoding is `VALID=0 | VOL=0 | PRIVILEGE=1` for a 64 KiB big PTE
(`uvm_pascal_mmu.c:229-241`), pinned literally as `0x20` by `uvm_page_tree_test.c:1774`, and
GA10x uses it (`entry_test_page_size_ampere` → `_volta` → `_pascal`). The live use is
`uvm_va_block.c:6484-6492`: *"First make the big PTEs unmapped to disable future lookups of the
4ks under it… Subsequent MMU fills will stop at the now-unmapped big PTEs, so we only need to
invalidate the 4k PTEs **without actually writing them**."*

⇒ **UVM unmaps 64 KiB of VA by writing ONE big PTE and deliberately leaving the 4 KiB PTEs
stale.** Our dual level computes `has_b` and `has_s` independently and emits every valid small
PTE regardless, so we would report 16 mappings the GPU will never honour — **after the guest
asked for them to be gone.** That is a stale mapping we hold open, i.e. exactly the class §39(b)
says matters if the memory is later reused.

### 2. A sparse BIG half must suppress the SMALL half

`gmmu_trace.c:429-466`, `_gmmuIsInvalidPdeOk`:

```c
if (pFmtGmmu->bSparseHwSupport && (sublevel == 0) && bSparse)
    return NV_FALSE;
```

and its caller `mmu_trace.c:552-558` turns that into `NV_ERR_INVALID_XLATE` and **leaves the
function** — not `continue`. Sublevel 0 **is the big half** (`kern_gmmu_fmt_gp10x.c:108-109`),
and `bSparseHwSupport` is true from GM20X on (`kern_gmmu_fmt_gm20x.c:40`).

⇒ A sparse big half aborts translation for the whole 2 MiB **without ever reading sublevel 1**.
We walk the small table anyway and report every leaf under it.

---

## Ranked NEW hostile cases (none of these are covered by the existing 20)

| # | case | what it constructs | what we should do |
|---|---|---|---|
| 3 | `comptagline_is_address_high_bits` | `PTE | (1<<36)`, then bit 55 | decode `{55:36, 32:8}<<12` or refuse by name — never report the low-25-bit truncation as the GPGA |
| 4 | `frontier_amplification` | 3 pages: PD2×512 → one PD1, PD1×512 → one PD0 ⇒ frontier 1 048 576 vs cap 131 072 | `FRONTIER_CAP` + `TRUNCATED`, and **must not install the table** so the next refresh RESYNCs |
| 5 | `peer_leaf_is_not_our_gpga` | `AP_PTE_PEER`, peer index 5 (`5<<33`) | refuse by name — a peer address is **another GPU's**, and publishing it as ours is a wrong mapping |
| 6 | `encrypted_bit_joins_run_identity` | `PTE_ENCRYPTED` (bit 4) on half of 64 contiguous pages | two runs, not one — the rule KIND already paid for |
| 7 | `large_sysmem_leaf` | `map512m(va, .., AP_PTE_SNC)` — 46-bit address field | no wrap, reported with its aperture so the host can reject it |
| 8 | `nv4k_is_not_sparse` | big-table slot `0x28` (VOL|PRIV, VALID clear) | must **not** count as sparse — RM's NV4K (`kern_gmmu_gv100.c:66-74`) and UVM's unmapped-big are different things and only one is "the guest declared this empty" |
| 9 | `poisoned_pte` | `map4k(va, 0x1bad000000, .., PRIV|RO)` | walk safely; the value is pinning that a PRIVILEGE mapping is not blindly publishable |

⚠ On #3, `cuda/walk/README.md:197-205` currently states the **opposite** conclusion — *"The
decode is unaffected… both decoders mask them off identically"* — on the same page that records
**COMPTAGLINE non-zero on 6 017 of 7 005 real leaves**. On a ≤128 GiB GA106 `addr_hi` is always
0, so this is unobservable for honest guests *on that die*, which is exactly why every benign
differential will keep passing over it. A hostile guest sets the bits anyway.

⚠ On #4: `KFWR_R_FRONTIER_CAP` is asserted by **no test in the suite**. The three cycle cases
each create one child per level, so the frontier never grows. A cap nobody has seen fire.

---

## Two findings that are NOT fuzz cases

- **`KFWR_R_FOREIGN_AP` would refuse a legitimate guest.** ogkm places page directories in
  **sysmem** as a matter of course (`gmmu_walk.c:170-183`, `:249-260`, gated on
  `NV_REG_STR_RM_INST_LOC_{PDE,PTE}`), and `_gmmuPdeAddrSpace` treats `SYS_COH`/`SYS_NONCOH` as
  a normal outcome. The refusal is the right *default*, but `t_hostile_foreign_aperture`'s name
  and comment assert a falsehood about the driver.
- **128 KiB big pages are reachable in RM and unrepresentable in our descriptor.**
  `kgmmuFmtInitLevels_GP10X:55` accepts `bigPageShift == 17`; UVM declines to use it but
  **RM does not reject it** (`uvm_turing_mmu.c:143-147` is an explicit TODO). Big page size is
  **instance-block state** (`NV_RAMIN_BIG_PAGE_SIZE`), so a guest selects it *without touching a
  single page-table byte* and our walker then misdecodes every big PT with no refusal. Wants a
  `KfFormat` variant and a launch-time check, not a fuzz case.

## Whole ogkm classes that are irrelevant to a read-only walker — deliberately skipped

- `mmu_walk*.c`'s ~60 asserts are the driver's own **shadow-state** bookkeeping (level-instance
  refcounts, btree nodes, memdescs). None are conditions on table bytes.
- VER3 PCF validation (`kern_gmmu_gh100.c:69-99`) — real, but VER3 is refused at `kf_create`.
  When VER3 ships this becomes the richest single source, because PCF is an **enumerated** field
  so most of its values are illegal, unlike VER2's bit-per-flag.
- *"Reserved aperture value"* is an **empty class in VER2**: all four PDE and all four PTE
  aperture encodings are defined (`dev_mmu.h:26-42`, `:79-83`). Don't write that test.
