# Timeline, effects and export-matrix spec

**STATUS: LIVE, 2026-11-16.** A tool-neutral editing timeline written by `timeline_spec.py` (stdlib
only, byte-for-byte deterministic). Measured on 2026-11-16: `spec` and `check` run offline and the
unit tests (`python3 -I scripts/ci/test_timeline_spec.py`) are green; no editing application has
imported these files yet (not measured).

## What it describes

One UHD timeline: 3840x2160, 25 fps, 1500 frames (60 s), 12 video clips on V1, 2 titles on V2, one
audio bed on A1.

- Clips are cut from the pinned assets `video/sintel_trailer_1080p.mp4` and
  `video/bbb_trailer_720p.mov` (names from `test_assets.json`) and from synthetic sources made by
  ffmpeg lavfi (`synthetic/*.mp4`: testsrc2, mandelbrot, smptehdbars at 3840x2160 25 fps, and a
  440 Hz sine). The exact command lines are in `metadata.kayfabe.synthetic_sources` of the OTIO file;
  `make-sources` runs them. Paths are relative (`video/<name>`, `synthetic/<name>`): the importing
  machine points the application at the directory that holds both.
- Speed changes: 50 percent (two clips), 200 percent, a reverse (`time_scalar` -1) and a freeze
  (`time_scalar` 0), all as `LinearTimeWarp`. A clip's `source_range` duration is its length on the
  timeline; the media it consumes is `ceil(duration * abs(time_scalar))` frames (one frame for a
  freeze) starting at `source_range.start_time`. `check` verifies that range lies inside the
  `available_range`.
- Three 12-frame cross dissolves, centred on the cut (6 frames of each neighbour).
- Two titles (generator `Title`, with text), and audio gain keyframes (fade in, duck, fade out).
- Every video clip carries a neutral colour grade in `metadata.kayfabe.grade`: `lift`, `gamma`,
  `gain` (RGB triples), `saturation`, and `tone_curve` (5 points).
- `available_range` of the two pinned assets is a conservative lower bound (50 s and 24 s) taken
  without probing, and their frame rate is assumed conformed to 25 fps. A later probe may raise the
  bounds.

## Files and which importer reads them

| File | Content | Reader |
|---|---|---|
| `timeline.otio` | full spec: clips, tracks, transitions, titles, time warps, grades, synthetic commands | OpenTimelineIO adapters; the application scripts read `metadata.kayfabe` directly |
| `timeline.fcpxml` | FCPXML 1.9: resources, project, sequence, spine, `timeMap`, `adjust-transform`, `transition`, `title` | the FCPXML importer of a non-linear editor |
| `timeline.edl` | CMX3600 list of the 12 cuts, `M2` lines for speed changes, comments for dissolves | any EDL reader |
| `export_matrix.json` | encode jobs (container, codec, bit depth, chroma, resolution, fps, bitrate or CRF, `hardware`, expected duration and fourcc) | the export step of each application script |

The title and dissolve effects in the FCPXML use neutral `kayfabe://` uids; an importer that does not
know them keeps the clip structure and the application script maps them to its own title and dissolve.
The EDL carries cuts only: titles, audio, grades and dissolves exist in OTIO and FCPXML.

## Application-specific layers

The spec stops at what every editor shares. A script for one application imports the timeline, then
layers its own effects on top: it translates `metadata.kayfabe.grade` into that application's colour
tools, maps the `Title` generator to its title tool, and runs each `export_matrix.json` job through its
own exporter. The job `hardware` field is `nvenc` (must run on the GPU encoder), `software`, or
`either`; AV1 jobs carry `requires_ada` (needs an Ada-generation GPU or newer). A run checks the produced
file's container, fourcc, resolution, fps and duration (`tolerance_s`), not its bitrate.

## Commands

```
python3 scripts/bench/assets/timeline_spec.py spec --out DIR
python3 scripts/bench/assets/timeline_spec.py check --out DIR
python3 scripts/bench/assets/timeline_spec.py make-sources --dest DIR [--assets DIR]
```

`check` regenerates the files in memory, compares the sha256 of each, and validates the structure:
source ranges inside available ranges, durations summing, transitions between clips, speed-change math,
FCPXML offsets and EDL record times continuous, and agreement between the three timeline files.
`make-sources` needs `ffmpeg` on `PATH`; with `--assets` it also verifies the two pinned files against
their pinned sha256.
