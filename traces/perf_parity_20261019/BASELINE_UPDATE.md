# P1 mirror baseline update

STATUS: VALIDATION IN PROGRESS, 2026-10-19 02:09 CEST.

CODE READ: the coordinator explicitly changed the hardware baseline to mirror binary `0d7ba74389cd85ac65826417ad53f5c0bcd1ef2f`, with the same mirror plus P1 diagnostics as candidate. Source head `c2685412e159534b88097ea824d05fe963be83b1` changes only docs/evidence after that binary revision. M1 now owns hardware first; P1 waits for its DONE or the revised 04:30 hand-off, with the existing 06:00 bound and 06:30 stop unchanged.

CODE READ: P1 keeps its published branch fast-forwardable. A normal commit adopts the requested mirror build inputs and binding/reset/display docs, then overlays P1 changes. No Git merge, rewritten public history, force-push or modification to upstream TSG logic is needed. Unrelated upstream measurement tools and raw evidence remain on their source branch. A file census verifies every adopted file outside the P1 overlay is byte-identical to the mirror source. Relative to that source, only 12 crate paths differ: the two optional diagnostic features, their wiring, the GET measurement and its tests.

MEASURED (TSG author's report, not a P1 hardware run): base DIRECT 840/849/851 us becomes 268/262/279 us; COMPUTE 1486/1558/1483 becomes 229/239/233 us; Steel Nomad base 2479/2432/2478 versus mirror ON 2876/2881/2996/2908/2929/2943. The report did not retrace the GPU leg; its attribution remains INFERRED pending P1's fence-value trace. CODE READ: M1 has published DONE and its report; its additional CUDA/Vulkan/video/broker gates are UNRUN because it stopped before taking the GPU. This is absent validation, not a measured hardware failure. P1 is now eligible for bounded GPU acquisition after Part A.

CODE READ: updated owner ruling AM (trust-boundary scrub policy) and the mirror's optional display meters/event-ring defaults have been read. The GET experiment does not change those paths. Production morning choice remains gated by the exact binary's full lane criteria.
