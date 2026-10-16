# Native Windows Heaven reference on a Vast 4070-class card: pre-registration

**STATUS: DESIGN-ONLY, 2026-10-16.** Written before any measurement. No box was rented (see REPORT.txt).

Question: what does the native machine do differently in Heaven D3D11 (power per frame, memory/PCIe
traffic, clocks, GPU busy fraction) that kayfabe's D3D11 does not, while OpenGL matches?
Inherited local-card figures (INFERRED context, not re-measured here): kayfabe scan-off D3D11 about
190-203 fps, GPU 93-95% busy at about 122 W; native D3D11 about 330 fps at about 191 W; OpenGL kayfabe
about 338 vs native about 342.

Hypotheses (state MEASURED vs INFERRED in the report):

- HN1: native D3D11 power at the same utilisation is >= 30% higher than kayfabe scan-off D3D11
  (122 W at 94% busy). FALSIFIED if within 15%.
- HN2: native D3D11 PCIe rx+tx per frame is within 2x of OpenGL's; kayfabe's differs. Report the number;
  no strong prediction.
- HN3: native fps D3D11/OpenGL ratio is about 0.97 (330/342) on this card. FALSIFIED if < 0.9 (then the
  earlier 330 was local to that setup).
- HN4: HAGS / Game Mode / power plan do not change D3D11 fps by more than 5% (one A/B, if time allows).
  FALSIFIED if any one changes D3D11 fps by more than 5%.

Method (fixed in advance): Heaven 4.0 Basic, Ultra, 1920x1080, multisample 0, kayfabe-harness flags and
260 s scripted benchmark; 1 warm-up, then 3 runs per API interleaved D3D11/OpenGL; `nvidia-smi dmon
-s pucvmet -d 1 -o T` for the whole run; idle and load `--query-gpu` of throttle reasons, pstate and
PCIe link. Derived: fps/W, J/frame, GPU busy x frame time, PCIe MB/frame, D3D11/OpenGL ratios.
SKU differences from the owner's 4070 (AD104) are marked INFERRED. No PresentMon.

## ADDENDUM 2026-10-16T22:15Z (coordinator ruling; written before the first measurement)

STATUS: LIVE. Supersedes the "SKU" wording above; the hypotheses HN1-HN4 stay, restated in ratio form.

Ruling: no rentable 4070/4070S/4070 Ti KVM offer exists. The coordinator accepted offer 55828515, an
RTX 4070 Ti Super (Vast name "RTX 4070S Ti"; AD103, not the owner's AD104). Instance 56303758
(rented 2026-10-16T21:58:49Z). Every absolute figure (fps, W, MHz, SM count, memory bus, power limit,
PCIe) is a SKU effect and is INFERRED to differ from the owner's 4070. The headline comparison is
therefore on ratios and per-frame quantities that are expected to transfer across Ada dies, native
vs kayfabe scan-off (kayfabe inputs: existing evidence only, read-only):

- Q1 fps ratio r_fps = fps(D3D11) / fps(OpenGL)
- Q2 energy per frame ratio r_E = (J/frame D3D11) / (J/frame OpenGL)
- Q3 GPU time per frame ratio r_G = (busy fraction x frame time, D3D11) / (same, OpenGL)
- Q4 PCIe per frame ratio r_P = (rx+tx MB/frame D3D11) / (same, OpenGL)
- Q5 power per utilisation, D3D11 over OpenGL: r_W = (W / util% D3D11) / (W / util% OpenGL)

Restated hypotheses (falsifier stated first for each):

- HN1r (was: native D3D11 power >= 30% higher than kayfabe's). Ratio form: kayfabe's D3D11 draws less
  power per utilisation than native D3D11. Prediction: native W/util% (D3D11) >= 1.3x kayfabe's
  scan-off W/util% (122 W / 94% = 1.30 W/%). FALSIFIED if the native W/util% is within 15% of
  kayfabe's. Secondary, from the inherited local-card figures (INFERRED, to be recomputed): energy per
  frame is about equal (native 191 W / 330 fps = 0.58 J, kayfabe 122 W / 195 fps = 0.63 J), so the
  expected difference is in GPU time per frame (kayfabe more), not in J/frame. FALSIFIED if native
  J/frame (D3D11) differs from kayfabe's by more than 25% after correcting by r_E (compare r_E, not
  absolute J).
- HN2 (unchanged): native r_P within [0.5, 2]; kayfabe's r_P outside it. The number is reported
  whatever it is; no strong prediction for kayfabe.
- HN3r (was: native D3D11/OpenGL fps ratio about 0.97): native r_fps = 0.97 +/- 0.05 on this card.
  FALSIFIED if r_fps < 0.9 (then the 330 fps figure was local to the old setup) or > 1.05. Kayfabe's
  scan-off r_fps (about 195 / 338 = 0.58) is expected to be far below native's; the gap is the
  quantity of interest.
- HN4 (unchanged): HAGS / Game Mode / power plan change D3D11 fps by <= 5% (one A/B if time allows).

Setup fixed in advance (kayfabe harness arguments, origin/claude/perf-win-20261015
scripts/bench/windows/perf/jobs/heaven.ps1.in): `Heaven.exe -project_name Heaven -data_path ../
-engine_config ../data/heaven_4.0.cfg -system_script heaven/unigine.cpp -sound_app null -video_app
{direct3d11|opengl} -video_multisample 0 -video_fullscreen 0 -video_mode -1 -video_width 1920
-video_height 1080 -extern_define RELEASE,QUALITY_ULTRA` (windowed, the benchmark is started in-app with
a click and F9). A deviation (for example because the NVIDIA GPU drives no physical display on a rented
box) is recorded in REPORT.txt as a deviation, with its likely effect marked INFERRED.
