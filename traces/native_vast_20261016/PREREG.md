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
