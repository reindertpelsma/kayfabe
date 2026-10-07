# `traces/native_dataplane_ga106/` — the NATIVE data-plane reference

**STATUS: LIVE — captured 2026-08-12.** Read
`docs/reference/native_dataplane_cup2_ga106.md` first; it is the analysis and it opens with two
findings that contradict the brief this was commissioned under.

## What this is

The first recording this project has of what a **native, unvirtualised** `cup2` does on the
**data plane**: the GPFIFO ring, the pushbuffer method stream, USERD `GP_GET`/`GP_PUT`, and the
report semaphore — captured on a real GA106 with host driver open `580.159.04`, **no QEMU, no
emulated GPU, no kayfabe**.

Every other oracle in this tree is control plane (the 56-row RM control table, the `nvdiff` ioctl
differential, the ogkm parsers), and the C artifact's green runs were CPU copies with
**emulator-written** completions. Nothing here is emulated.

## Headline facts

- A 4-byte `cuMemcpyHtoD` uses **no copy engine**. It is the compute class's inline-to-memory
  unit (`NVC7C0` `LINE_LENGTH_IN` / `OFFSET_OUT` / `LAUNCH_DMA` / `LOAD_INLINE_DATA`) and the
  **data is a literal in the pushbuffer** — 15 dwords total.
- The report semaphore is at **`0x2_0440_fff0`**, page offset **`+0xff0`**, in **host RAM**
  (physical address in no GPU BAR), slot 15 of a 16-slot × 16-byte sysmem pool at `+0xf00…+0xff0`.
  **The same address our guest uses.**
- The GPU wrote it: the report's timestamp is a live hardware clock that tracks CPU wall time to
  **43 ppm over 0.72 s**, and the landed payload equals the one the pushbuffer declared.
- `HtoD` and `DtoH` run on **two different channels** (`ch[0]`, `ch[12]`) releasing **two
  different slots** (`+0xff0`, `+0xf30`).

## Runs

| run | host | GPU / driver | notes |
|---|---|---|---|
| `run_20260812T111414Z` | `vh2` (vast 47373001) | GA106 `[10de:2504]` / open `580.159.04` | the reference run; harness at repo rev `1e6fc945` |

## Reading the capture

`nvdp.log` is the annotated transcript. **Every line carries the time it was EMITTED**, not the
time of the event — a recorder that buffers and dumps at teardown reports order correctly and time
not at all, which has cost this project a rung before. The polled series in `raw/samples.csv` is
different: each row is stamped at the moment of sampling, so it carries real time.

`raw/*.bin` are byte-exact dumps (ring, pushbuffer segments, the semaphore page). Decode them with
`ogkm-580.159.04` headers: `clc36f.h` (GPFIFO entries, method headers), `clb06f.h` (the method
word's field positions — note `INCR_ADDRESS` is `11:0`, **not** the Kepler-era `12:2`),
`clc7c0.h` (the compute class), `clc7b5.h` (the copy class).

## Provenance rule

`provenance.txt` in each run carries the source `sha256` and the repo revision. ⚠ Any claim made
from this capture must carry them — this tree has silently served a stale binary for weeks before.

## Harness

`tests/mode2/nvdp/nvdp.c` + `nvdp_run_host.sh`. Self-contained: no CUDA toolkit, no `LD_PRELOAD`.
The runner refuses to start if a guest boot is in flight (the GPU is a serial resource) and writes
a start marker and an exit-status terminator, so a truncated artefact is detectable rather than
reading as "still in flight".
