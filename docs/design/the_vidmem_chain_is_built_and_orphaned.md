# The vidmem chain is built and orphaned — every piece exists, nothing calls it

**STATUS: LIVE (2026-09-06).** Survey + verification of what stands between a
vidmem-declared guest allocation and real device-local backing with a CPU view.
Supersedes nothing. Two of its findings are **blocked on an owner ruling** and are
marked as such.

Owner direction this serves: *"MAP VIDMEM INTO THE GPA: the device-node mmap route we
specified and never issued"* (2026-08-27). Motivating number: operands measured
**2.51 GB/s** in host sysmem versus **313.5 GB/s** in VRAM — the **14.80×**
placement factor in the large-kernel perf gap.

---

## 1. The headline, verified by grep and not taken on trust

**Every part of the vidmem path exists. Not one of them is called by production code.**

| piece | defined at | production callers |
|---|---|---|
| `backing_for(DeclaredPlacement)` | `kayfabe-fwd/src/lib.rs:2390` | **0** |
| `honours_declaration` | `kayfabe-fwd/src/lib.rs:2411` | **0** |
| `DeclaredPlacement` (constructed) | `kayfabe-fwd/src/lib.rs:2344` | **0** |
| `RmBackend::alloc_vidmem` | `kayfabe-isolate-host/src/rm.rs:4423` | 0 on this path |
| `ChildExports::mint_armed_node` | `kayfabe-isolate-host/src/export.rs:146` | **0 anywhere, incl. tests** |
| `ChildBacking::ArmedNode` | `kayfabe-isolate-host/src/export.rs:93` | only inside `mint_armed_node` |

`backing_for` and `DeclaredPlacement` appear in exactly **one file in the whole tree**,
`kayfabe-fwd/tests/aperture_choice.rs` — 14 occurrences, all test code.

⚠ **Trap for whoever checks this next.** `backing_for` at
`kayfabe-vmm-qemu/src/viewer_install.rs:306` and `:694` is an **unrelated trait method**
(`ObjectBacking::backing_for(object, aperture, gpga_base, len) -> HostBacking`). A grep
for the name alone reports production callers that do not exist.

**What actually decides today: two `Joined` literals.**
- `kayfabe-qemu-raw/src/shim.rs:10902`, inside `join_one_fb_leaf` (`:10715`) — the only
  production `back_fb_leaf` call.
- `kayfabe-rt/src/device.rs:4490`, inside `adopt_joined_fb_leaf` — **correct by name**,
  the joined chain's own second half. Not the site to change.

⊘ `SharedDevice::back_fb_leaf` (`device.rs:4420-4441`) decides nothing: `how` is a
pass-through parameter.

## 2. The aperture is available — one layer below where the choice is made

`FbLeaf` (`kayfabe-rt/src/completion_watch.rs:408-415`) carries `{va, len, phys}` and
**no aperture**, so `shim.rs:10902` genuinely cannot see it.

★ But `plan_back_fb_leaf` **already reads it**: `kayfabe-fwd/src/lib.rs:2596` evaluates
`b.aperture() != Aperture::Vidmem` on the binding `b` obtained from
`vas.table.binding_at(va)` (`:2586`). `b.aperture()` is exactly the value
`DeclaredPlacement::Aperture(_)` wants.

⇒ **De-orphaning `backing_for` needs no plumbing at all.** Derive `how` inside
`plan_back_fb_leaf` from the binding already in hand, instead of accepting it as a
parameter. The candidate filter at `device.rs:3692` already guarantees every FB-leaf
candidate is vidmem-declared.

## 3. The 0x4E device-node route is BUILT and IS in production

Worth stating plainly, because the campaign twice recorded the opposite:

- One issue site: `kayfabe-isolate-host/src/rm.rs:2297`, in `map_cpu_windowed_on`
  (`:2253`), which opens `nvidia<N>` for `MapNode::Gpu` (`:2274-2278`) and maps
  `Backing::DeviceFile` (`:2322-2328`). `map_cpu` defaults to `MapNode::Gpu` (`:2214`).
- Production callers: `rm.rs:1520` (usermode window), `:6370` (ring), `:6383` (USERD).

⇒ **The isolate already holds a real CPU view of host VRAM.** What is missing is the
*crossing* to the VMM, not the mapping.

## 4. What is genuinely unbuilt — exactly two things

### (a) The isolate→VMM crossing of the armed node ⚠ NEEDS AN OWNER RULING

`HostRmBackend::export_backing` (`rm.rs:5036-5044`) returns
`RmError::NotExportableAsMemory` for `ExportSource::HostDeviceMemory` **unconditionally,
before any host call**; `kayfabe-isolate/src/lib.rs:2716` gates it again. So
`commit_back_fb_leaf`'s `PublishVidmem` arm yields `backing: None` and nothing reaches
the guest.

Its stated justification was three refusals. **One is ours** — see the correction folded
into `rm.rs` above `export_backing` (commit `38a1a508`):
`Backing::DeviceFile => Err(RawError::DeviceBackingNotPlaceable)` at
`kayfabe-linux-raw/src/window_unsafe.rs:213`.
And **reason 1 is the policy question `mint_armed_node` was built to answer**: the
privilege hazard (`secInfo.privLevel` recomputed per escape, `escape.c:304`) bites a
process that *issues escapes*, and in the armed-node shape the VMM only ever `mmap`s the
fd. That is the shape `nvkvm-pv` ships.

⇒ **This is decision (b)'s scope and the owner's call, not a fact to discover.**

### (b) A host-initiated CE establishment copy

`SparseFb::install_join` fills a newly-installed backing with `region.write(at, src)`
(`kayfabe-device/src/fbwin.rs:1168`). Over a memfd that is HtoH and fine; over a BAR
mapping it becomes a **bulk WC CPU write**, which `copy_placement_policy.md` §2.2
forbids.

`ce_copy` exists and is production-reachable — `kayfabe-isolate/src/lib.rs:1005`, host
impl `rm.rs:4873` → `ce_copy_outcome` `:6761`, driven from `SharedDevice::forward_ce`
(`device.rs:3008`). ⊘ But it is **ring-driven only**: `forward_ce` takes `&[CeSpan]`
whose `dst` is a GPU VA in the guest's host VAS, and there is **no verb meaning "CE-copy
these host-side bytes into this leaf."** `kayfabe-device` also has no dependency on
`kayfabe-rt`/`kayfabe-isolate`, so `fbwin.rs:1168` cannot call it as written.

⇒ Shape: a new method on the `FbJoined` trait (`fbwin.rs:513`), implemented by the
shell's vidmem join type, calling a new `SharedDevice::ce_establish` beside
`forward_ce`.

⚠ **Landing (a) without (b) makes performance WORSE, not better** — it converts the
join's memcpy into a bulk write across the BAR, the exact thing the ruling forbids.

## 5. The ordered gap

1. Derive `how` in `plan_back_fb_leaf` from `b.aperture()` (§2). De-orphans
   `backing_for`. Keep the parameter as `debug_assert!(honours_declaration(..))` or
   delete it and update `device.rs:4427` + `shim.rs:10902`. ⚠ The replay path
   (`:2586-2607`) and `bind_backed_fb_leaf` (`:2911`) must agree — `BackingBytes` is
   derived from `plan.how` (`:3032-3033`).
2. `alloc_vidmem` (`rm.rs:4423`) calls `map_cpu(h, len, WriteCombining)` after
   `alloc_device_local`. Uses the existing production route.
3. **[owner ruling]** relax the two refusals deliberately; mint the armed node; carry
   token+len in `VerbReply::Published`.
4. VMM maps `Backing::DeviceFile` (`shim.rs:10944-10958`), `offset = 0`, length exactly
   as registered (`nv-mmap.c:562-565` refuses anything else); lift our own
   `DeviceBackingNotPlaceable`.
5. `ce_establish` (§4b) — **in the same commit as 3–4**, never after.

## 6. What would make this measurable

⊘ No step above is verifiable by a green test suite: the decision function is already
green over inputs nothing constructs. The falsifier is a **boot** — the `not_vidmem`
counter at `kayfabe-rt/src/device.rs:3692` must fall, and the operand bandwidth must
move off 2.51 GB/s toward the 313.5 GB/s VRAM figure. A unit test that asserts
`backing_for` returns `Vidmem` proves nothing about which value production passes.
