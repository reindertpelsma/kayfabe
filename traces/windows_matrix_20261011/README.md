# Windows matrix campaign

**STATUS: LIVE, 2026-10-11.** Product candidate `2c917f3e`, following the
OpenGL gears fix. Results are pending; this is not a matrix pass claim.

The current Windows inventory has 108 tests mapping to 65 Linux rows. The native
Windows reference in `traces/windows_baseline_20261010/results.tsv` covers 100:
99 PASS and `gravitymark_vk` TIMEOUT. Eight new YUV overlay tests have no recorded
reference there. Reference instance 55384005 reports a GPU startup error before
Windows becomes available. The owner authorizes replacement rentals and use of
the project SSD; Vast remains untrusted, with no secrets and no unique evidence
left there.

Harness corrections use the independently verified sign-in helper (including
releasing the single-client QGA connection), attach exFAT images directly as USB
mass storage, and validate the checksum belonging to the selected image. The
explicit `WIN_DISABLE_MPO=1` setting records the same DWM policy used by the
passing production gears run. It must not be used to claim overlay support.

Before the first GPU run, hypothesis: these changes permit the existing matrix
supervisor to run CUDA and OpenGL probes on a fresh production guest. Falsifier:
no verified desktop/app disk, or either probe fails its existing GPU/correctness
oracle. GPU results will be recorded whether the hypothesis holds or fails.

## First measured result

Smoke run 650, production flags, RPC/GSP observers disabled, display enabled:
**4/4 PASS** (`nvidia_smi`, `deviceQuery_demo`, `glgears_wgl`, `matmul_t`),
zero guest TDRs and zero new host Xids. The sign-in/app-disk hypothesis held.
Evidence is in `smoke/`, including app facts, logs, screenshots and command.
The frozen harness is `8385a6cb`; the product binary is `2c917f3e`.

The reference-covered 100-test batch started at 12:01:34 UTC, 2026-10-11,
run numbers 651 onward, eight apps per fresh guest and isolated retries of
non-PASS tests. `run_full.sh` and `post_guest.sh` preserve the exact runner.
Reference instance 55385171 was rented with `vms_enabled=true`, the KVM image,
180 GB disk, and the windows-on-vast installer as its on-start script. Installer
logs confirm reboot into the Windows reinstall environment; Windows setup
completion is not yet verified. No credentials were copied to the rental.

The six existing omitted Linux rows need separate scrutiny: only `simpleIPC`
is explicitly unsupported by WDDM; callback, occupancy, dynamic parallelism,
transformers generation and Geekbench have packaging/build/network limitations
in the current inventory. Those limitations are not proof that the applications
cannot run on Windows, and the 65-row mapping must not be called full parity.

## Controlled harness correction

The initial batch completed `atomics_t` and `bandwidthTest_demo`: **2/2 PASS**,
zero TDRs/new host Xids. It was deliberately stopped (exit 143) before the stale
D3D probe could run. `initial-batch/` preserves that checkpoint. Inspection of
the actual exFAT disk confirmed the old D3D probe hash
`cf231b29f73e711d3c34024954658e0633cd09b4f15edf9602b75b704d911877`.
The reference README had already identified this stale image; the initial
preflight missed that detail. The corrected winding source is unchanged from
`f0e11b89`.

Harness `6cca39b2` adds `--override-dir` with guest SHA256 verification and a
per-guest `tool-overrides.json`. Tests reject a truncated upload before any app
runs; all 71 harness tests pass. The rebuilt corrected probe used here has hash
`3c6ededa77430a18a13321eb72fdca7f60282f8d12ddc147a7e7df1284832c5f`.
`run_full2.sh` resumes the same results, skips the two completed tests, and starts
fresh guests at run 652. Product binary remains `2c917f3e`.
