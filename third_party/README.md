# `third_party/` — reference oracles, pinned. **Not libraries.**

⊘ **Nothing here is compiled into kayfabe, linked against, or vendored.** These are the sources
every provenance claim cites (`docs/design/THE_CONSTRAINTS.md` §50), and they are submodules for
exactly one reason:

> ★★★ **A citation without a revision is not a citation.** *"ogkm says X"* is unfalsifiable until
> you can say *which* ogkm. This campaign has already paid for the general form of that mistake —
> see `a_rulings_date_is_part_of_the_citation`.

| path | upstream | rank | the question it answers |
|---|---|---|---|
| `ogkm` | NVIDIA/open-gpu-kernel-modules @ `57130a2` (**610.43.02**) | ★★★ **primary** | what the driver we must satisfy actually **requires** — §50 level 2, and the only source that *is* the acceptance criterion |
| `ogkm-580` | same @ `b81d58e` (**580.159.04**) | ★★★ | the version **the bench runs**; the diff against 610 is the version axis |
| `linux` | torvalds/linux @ `6f3ed7fec` | ★★ | **nouveau** (`drivers/gpu/drm/nouveau`) — non-GSP behaviour; and **nova** (`drivers/gpu/nova-core`) — the minimal boot surface, vendor-contributed |
| `gvisor` | google/gvisor @ `85b606a` | ★★ | **nvproxy** (`pkg/sentry/devices/nvproxy`) — **ioctl formats to host userspace**, the raw client, and the **forwarded-RM allowlists** |

⊘ **nvproxy answers a different question from the other three**, and conflating them is a category
error: nouveau/nova/ogkm describe *the guest driver talking to hardware*; nvproxy describes *a host
process talking to `/dev/nvidia*`* — kayfabe's host side. It is the reference for the allowlist and
struct layouts, and says nothing about BAR0.

## They are NOT initialised by default

`git clone` of kayfabe stays small. Fetch only what you need, shallow:

```sh
git submodule update --init --depth 1 third_party/ogkm     # ~170 MB
git submodule update --init --depth 1 third_party/linux    # large; nouveau + nova
```

⚠ **`linux` is ~2 GB checked out.** The citation tests
(`crates/kayfabe-doorbell/tests/nouveau_citations.rs`) resolve either this directory **or** a
pre-existing local clone, and **skip loudly** when neither is present — a test that silently passes
on a missing input is worse than no test.

## One consolidation already made, and how it was checked

`nouveau` previously lived in a second, separately-revisioned kernel clone. Before collapsing it
into `linux`, **every file cited by the design docs was diffed between the two trees and found
byte-identical** (`subdev/timer/nv04.c`, `falcon/cmdq.c`, `falcon/msgq.c`, `subdev/fault/gv100.c`,
`falcon/gp102.c`, `falcon/gm200.c`). ⇒ The citations carry over unchanged. If that ever stops
holding, the citation tests are what will say so.
