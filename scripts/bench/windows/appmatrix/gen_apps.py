#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""gen_apps.py -- the single source of the Windows app matrix inventory. Writes apps.json (committed, consumed by
winapps.py / verdict.py / summarize.py / the tests; tests/test_apps.py fails if apps.json drifts from this file).

An app is a PowerShell body (`ps`) that the driver wraps in a prelude and runs under the supervisor
(guest/kf_run_app.ps1). Tokens substituted by the driver: @@ID@@ @@CD@@ (drive letter of the app disk).
Pass predicate = the Linux run_apps.sh one: rc 0 AND `rx` matches the log AND `fail_rx` does not AND a GPU
proof holds (any one of `proof`) AND no engine activity on a non-NVIDIA adapter (WARP excluded).
`difficulty` / `why` are REASONED from docs/STATUS_AND_HANDOFF.md and traces/windows_prod_20261010 (nothing
here has run on a kayfabe Windows guest yet).
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# ---------------------------------------------------------------------------------------------- prelude
PRELUDE = r"""$ErrorActionPreference = 'Continue'
. C:\kf\kf_apphelpers.ps1
$K = 'C:\kfapps'; $CD = '@@CD@@'; $OUT = 'C:\kf\out\@@ID@@'
$PY = "$CD\py\python.exe"; $TL = "$CD\py\Lib\site-packages\torch\lib"
New-Item -ItemType Directory -Force -Path $OUT | Out-Null
$env:CUPY_CACHE_DIR = 'C:\kf\cupy_cache'; $env:KF_TORCH_LIB = $TL; $env:CUDA_PATH = "$CD\py\Lib\site-packages\torch"
$env:KF_CUDA_INCLUDE = "$K\cuda_cudart\include;$K\cuda_cccl\include"
$PYA = @('-X', 'utf8', '-X', 'pycache_prefix=C:\kf\pycache')
"""

APPS = []
DEFAULT_FAIL = r"^CHECK .*FAIL"


def app(id, cat, ps, rx, timeout, expect, proof, diff, why, linux=(), pkgs=(), session="service", tier=1,
        fail_rx=DEFAULT_FAIL, kill=(), shot=None, note=None, digest=False, smi=True, nvidia_only=True):
    assert id not in {a["id"] for a in APPS}, id
    APPS.append(dict(id=id, category=cat, linux=list(linux), pkgs=list(pkgs), session=session, tier=tier, timeout_s=timeout,
                     expect_s=expect, rx=rx, fail_rx=fail_rx, proof=list(proof), difficulty=diff, why=why, ps=ps.strip("\n") + "\n",
                     kill_names=list(kill), screenshot_s=shot, note=note, digest=digest, smi=smi, nvidia_only=nvidia_only))


def py(script, *args):
    a = ", ".join("'%s'" % x for x in args)
    return (f"Invoke-KfExe -Exe $PY -ArgList ($PYA + @('C:\\kf\\py\\kf_runpy.py', 'C:\\kf\\py\\{script}'" + (", " + a if a else "") + "))\n"
            "exit $global:KfRc\n")


PYPK = ("python_embed", "pywheels", "vc_redist")
GPU_ENG = "pdh:Cuda|Compute|3D|Copy"
OUT_NV = "out:NVIDIA|GeForce|RTX"

# difficulty vocabulary (reasoned): what a kayfabe Windows guest has to get right for the app
W_COMPUTE_SMALL = ("low", "CUDA context + a few kernels and small copies: the compute path with no display, no video engine and no long run; "
                   "Windows user work runs as Passthrough twins whose guest-visible GP_GET lags the host (traces/windows_tdr_hunt: H-D)")
W_COMPUTE_BIG = ("medium", "sustained cuBLAS/cuDNN/cuFFT work, 1-4 GiB allocations, many launches per second: stresses the twin/USERD relay and the "
                 "completion path for minutes, and the guest TDRs about once a minute today (traces/windows_prod_20261010: 7 resets in ~8 min)")
W_GFX = ("high", "needs a 3D/graphics channel, the display flip/VSync path and DWM composition on kf3's display; the first TDR coincides with "
         "the first new GPU client after sign-in (DWM, Edge, Shorts) in every recorded run, so any windowed app is exposed to it")
W_VIDEO = ("high", "needs the NVENC/NVDEC/DXVA engines through Windows' video-engine nodes (a separate scheduler node from 3D/compute); "
           "no Windows run of these engines exists yet, only the Linux ones")
W_LONG = ("high", "runs longer than the ~1 minute TDR cadence recorded today, so a TDR inside the run is likely; the result is only meaningful "
          "once the guest survives idle for the whole app")

# ---------------------------------------------------------------------------------------------- A. system / probes
app("nvidia_smi", "smi", r"""Invoke-KfExe -Exe "$env:SystemRoot\System32\nvidia-smi.exe"
exit $global:KfRc""", r"(GeForce|RTX|Quadro|Tesla)", 60, 3, ["out:GeForce|RTX|Quadro|Tesla"], *W_COMPUTE_SMALL[:1],
    "RM control path only (GSP info, utilisation); no channel needed", linux=["nvidia_smi"], smi=False)
app("dxgi_adapters", "probe", r"""$a = Get-Content C:\kf\adapters.json -Raw | ConvertFrom-Json
foreach ($x in $a.adapters) { "ADAPTER index=$($x.index) vendor=$($x.vendor) name=$($x.name) luid=$($x.luid_high)_$($x.luid_low) dedicated_mb=$($x.dedicated_mb) software=$((($x.flags -band 2) -ne 0))" }
if (@($a.adapters | Where-Object { $_.vendor -eq 4318 -and (($_.flags -band 2) -eq 0) }).Count -ge 1) { 'DXGI_OK NVIDIA hardware adapter present'; exit 0 } else { 'DXGI_FAIL no NVIDIA hardware adapter'; exit 1 }""",
    r"^DXGI_OK", 60, 2, [r"out:vendor=4318"], "low", "DXGI adapter enumeration only (no GPU work); fails if the NVIDIA device is in Code 43 or absent", smi=False)
app("driver_status", "probe", r"""$v = Get-CimInstance Win32_VideoController | Where-Object { $_.Name -match 'NVIDIA' }
foreach ($x in $v) { "VIDEO name=$($x.Name) driver=$($x.DriverVersion) status=$($x.Status) code=$($x.ConfigManagerErrorCode)" }
if (@($v | Where-Object { $_.ConfigManagerErrorCode -eq 0 }).Count -ge 1) { 'DRIVER_OK'; exit 0 } else { 'DRIVER_FAIL (Code 43 or missing)'; exit 1 }""",
    r"^DRIVER_OK", 60, 2, [r"out:name=NVIDIA"], "low", "device-manager state only; the Code 43 wall of the earlier runs would show here", smi=False)
app("d3d12_signal_probe", "d3d", r"""& powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kf\d3d12_signal_probe.ps1 2>&1 | ForEach-Object { "$_" }
exit 0""", r"COPY Signal\(3\) hr 0x00000000 wait SIGNALED", 180, 10, [r"out:vendor 0x10de", GPU_ENG], "medium",
    "the smallest D3D12 GPU work (fence Signal on DIRECT and COPY queues, 20 s wait each); already used on earlier Windows runs "
    "(scripts/bench/windows/d3d12_signal_probe.ps1) where Passthrough completion interrupts were the wall",
    fail_rx=r"TIMEOUT|no NVIDIA adapter|PROBE exception", pkgs=("vc_redist",))

# ---------------------------------------------------------------------------------------------- B. CUDA toolkit demo suite (prebuilt)
DEMO = r'$d = "$K\cuda_demo\demo_suite"' + "\n"
app("deviceQuery_demo", "cuda-demo", DEMO + r"""Invoke-KfExe -Exe "$d\deviceQuery.exe" -Cwd $d
exit $global:KfRc""", r"Result = PASS", 60, 3, [OUT_NV], *W_COMPUTE_SMALL, linux=["deviceQuery"], pkgs=("cuda_demo_suite", "vc_redist"))
app("vectorAdd_demo", "cuda-demo", DEMO + r"""Invoke-KfExe -Exe "$d\vectorAdd.exe" -Cwd $d
exit $global:KfRc""", r"Test PASSED", 60, 3, [GPU_ENG, r"smi"], *W_COMPUTE_SMALL, linux=["vectorAdd"], pkgs=("cuda_demo_suite", "vc_redist"))
app("bandwidthTest_demo", "cuda-demo", DEMO + r"""Invoke-KfExe -Exe "$d\bandwidthTest.exe" -ArgList @('--memory=pinned') -Cwd $d
exit $global:KfRc""", r"Result = PASS", 120, 15, [OUT_NV, GPU_ENG], *W_COMPUTE_BIG, linux=["bandwidthTest"], pkgs=("cuda_demo_suite", "vc_redist"))
app("nbody_demo_bench", "cuda-demo", DEMO + r"""Invoke-KfExe -Exe "$d\nbody.exe" -ArgList @('-benchmark', '-numbodies=32768') -Cwd $d
exit $global:KfRc""", r"GFLOP/s", 120, 15, [OUT_NV, GPU_ENG], *W_COMPUTE_BIG, linux=["nbody"], pkgs=("cuda_demo_suite", "vc_redist"),
    note="-benchmark should not open a window in the sample; if it does, move to session=interactive")
app("nbody_demo_gl", "cuda-demo", DEMO + r"""Invoke-KfSurvive -Exe "$d\nbody.exe" -ArgList @('-numbodies=16384') -Cwd $d -Seconds 25 -Tag nbody_gl
exit $global:KfRc""", r"KFSURVIVE nbody_gl ALIVE_AT", 90, 25, [GPU_ENG], *W_GFX, linux=["nbody"], pkgs=("cuda_demo_suite", "vc_redist"), session="interactive",
    shot=15, note="CUDA-OpenGL interop: the simulation runs on CUDA, the points are drawn by OpenGL from the same buffer")
app("oceanFFT_demo", "cuda-demo", DEMO + r"""$env:PATH = "$TL;$env:PATH"
Invoke-KfSurvive -Exe "$d\oceanFFT.exe" -Cwd $d -Seconds 25 -Tag oceanFFT
exit $global:KfRc""", r"KFSURVIVE oceanFFT ALIVE_AT", 90, 25, [GPU_ENG], *W_GFX, linux=["simpleCUFFT"], pkgs=("cuda_demo_suite", "pywheels", "python_embed", "vc_redist"),
    session="interactive", shot=15, note="cufft64_11.dll comes from the torch wheel on PATH; CUDA-OpenGL interop")
app("randomFog_demo", "cuda-demo", DEMO + r"""$env:PATH = "$TL;$env:PATH"
Invoke-KfSurvive -Exe "$d\randomFog.exe" -Cwd $d -Seconds 25 -Tag randomFog
exit $global:KfRc""", r"KFSURVIVE randomFog ALIVE_AT", 90, 25, [GPU_ENG], *W_GFX, linux=["MersenneTwisterGP11213"],
    pkgs=("cuda_demo_suite", "pywheels", "python_embed", "vc_redist"), session="interactive", shot=15, note="curand64_10.dll from the torch wheel; CUDA-OpenGL interop")

# ---------------------------------------------------------------------------------------------- C. CUDA driver-API ladder (kayfabe's own, mingw)
LAD = r'$t = "$K\tools"' + "\n"
app("cup2_ladder", "cuda-ladder", LAD + r"""Invoke-KfExe -Exe "$t\cup2.exe"
exit $global:KfRc""", r"CE rv=0xabcd1234 want=0xabcd1234 -> PASS", 60, 3, [r"out:name=NVIDIA|name=.*GeForce", GPU_ENG], "low",
    "one copy-engine launch through the driver API (the first rung of the Linux ladder)", linux=["vectorAddDrv"], pkgs=("vc_redist",))
app("cup3_ladder", "cuda-ladder", LAD + r"""Invoke-KfExe -Exe "$t\cup3.exe"
exit $global:KfRc""", r"KERNEL rv=43 want=43 -> PASS", 60, 3, [r"out:name=NVIDIA|name=.*GeForce|PTX_TARGET", GPU_ENG], "low",
    "one compute kernel (PTX JIT, sm rewrite) through the driver API", pkgs=("vc_redist",))
app("cup8_ladder", "cuda-ladder", LAD + r"""Invoke-KfExe -Exe "$t\cup8.exe"
exit $global:KfRc""", r"CUP8 RESULT N=\d+ bad=0 maxerr=0 ", 180, 10, [GPU_ENG, "smi"], "medium",
    "a 2048x2048 fp32 matmul through the driver API, byte-exact against the host", linux=["matrixMulDrv"], pkgs=("vc_redist",))

# ---------------------------------------------------------------------------------------------- D. CUDA samples counterparts (PyTorch / CuPy / NVRTC)
EQ = [  # id, test, linux names, timeout, expect, difficulty tuple
    ("matmul_t", "matmul", ["matrixMul", "sgemm_cublas", "simpleCUBLAS"], 120, 5, W_COMPUTE_SMALL),
    ("tc16_t", "tc16", ["cudaTensorCoreGemm"], 120, 5, W_COMPUTE_SMALL),
    ("tcbf16_t", "tcbf16", ["bf16TensorCoreGemm"], 120, 5, W_COMPUTE_SMALL),
    ("fft_t", "fft", ["simpleCUFFT", "fft_cufft"], 120, 5, W_COMPUTE_SMALL),
    ("rng_t", "rng", ["MersenneTwisterGP11213"], 120, 5, W_COMPUTE_SMALL),
    ("reduce_t", "reduce", ["reduction", "reduce"], 120, 8, W_COMPUTE_BIG),
    ("scan_t", "scan", ["scan"], 120, 5, W_COMPUTE_SMALL),
    ("sort_t", "sort", ["sortingNetworks"], 120, 5, W_COMPUTE_SMALL),
    ("hist_t", "hist", ["histogram"], 120, 5, W_COMPUTE_SMALL),
    ("transpose_t", "transpose", ["transpose"], 120, 5, W_COMPUTE_BIG),
    ("blackscholes_t", "blackscholes", ["BlackScholes", "blackscholes"], 120, 5, W_COMPUTE_SMALL),
    ("fwt_t", "fwt", ["fastWalshTransform"], 120, 5, W_COMPUTE_SMALL),
    ("streams_t", "streams", ["simpleStreams", "asyncAPI", "concurrentKernels"], 120, 5, W_COMPUTE_SMALL),
    ("graphs_t", "graphs", ["simpleCudaGraphs", "graphMemoryNodes"], 120, 5, W_COMPUTE_SMALL),
    ("atomics_t", "atomics", ["simpleAtomicIntrinsics"], 120, 5, W_COMPUTE_SMALL),
    ("zerocopy_t", "zerocopy", ["simpleZeroCopy"], 120, 8, W_COMPUTE_BIG),
    ("mandelbrot_t", "mandelbrot", ["mandelbrot"], 120, 8, W_COMPUTE_SMALL),
    ("nbody_t", "nbody", ["nbody"], 120, 5, W_COMPUTE_SMALL),
    ("memcpy2d_t", "memcpy2d", ["memcpy2d"], 120, 8, W_COMPUTE_BIG),
    ("triad_t", "triad", ["stream_triad"], 120, 8, W_COMPUTE_BIG),
    ("cg_t", "cg", ["conjugateGradient"], 120, 5, W_COMPUTE_SMALL),
]
for aid, test, lin, tmo, exp, dif in EQ:
    app(aid, "cuda-equiv", py("win_cuda_equiv.py", test), r"^WINEQ_DONE " + test, tmo, exp, [OUT_NV, GPU_ENG], *dif, linux=lin, pkgs=PYPK)
app("managed_t", "cuda-equiv", py("win_cuda_equiv.py", "managed"), r"^WINEQ_DONE managed", 180, 15, [GPU_ENG, OUT_NV], "high",
    "managed (unified) memory: the Linux rows UnifiedMemoryStreams/Perf, conjugateGradientUM and attach_verify are the four that fail on Linux "
    "kayfabe (UVM demand paging, docs/design/V3_UVM_DEMAND_PAGING.md); on Windows WDDM managed memory is the limited model (no oversubscription)",
    linux=["UnifiedMemoryStreams", "UnifiedMemoryPerf", "conjugateGradientUM", "attach_verify"], pkgs=PYPK,
    note="CuPy malloc_managed; expected to be the hardest compute row on both OSes")
for name, lin in (("default", "stream_default"), ("created", "stream_created"), ("nonblocking", "stream_nonblocking"), ("two", "stream_two"),
                  ("created2nd", "stream_created2nd"), ("perthread", "stream_perthread")):
    app("stream_%s_t" % name, "probe", py("win_cuda_equiv.py", "stream_" + name), r"^STREAM_PROBE_DONE %s rc=0" % name, 120, 5, [OUT_NV, GPU_ENG],
        *W_COMPUTE_SMALL, linux=[lin], pkgs=PYPK, note=("CuPy per-thread default stream (cupy.cuda.Stream.ptds)" if name == "perthread" else
                                                           "torch streams are created cudaStreamNonBlocking" if name in ("created", "nonblocking") else None))
for aid, test, lin, exp in (("cupy_cg", "cg", ["simpleCooperativeGroups"], 10), ("cupy_asynccopy", "asynccopy", ["globalToShmemAsyncCopy"], 10),
                            ("cupy_sha256", "sha256", ["sha256"], 10)):
    app(aid, "cuda-equiv", py("cupy_nvrtc.py", test), r"^NVRTC_DONE " + test, 180, exp, [OUT_NV, GPU_ENG], *W_COMPUTE_SMALL, linux=lin,
        pkgs=PYPK + ("cuda_cudart", "cuda_cccl"), note="NVRTC through CuPy: no nvcc/MSVC in the guest")
app("torch_correct", "torch", py("torch_correct.py"), r"TORCH_CORRECT_DONE", 300, 30, [OUT_NV, GPU_ENG], *W_COMPUTE_BIG, linux=["torch_correct", "conv2d"], pkgs=PYPK, digest=True,
    note="unmodified scripts/apps/src/torch_correct.py (the Linux row); includes a 1 GiB round trip and a cuDNN conv")
app("torch_ai_bench", "torch", py("ai_bench.py"), r"CHECK bert_infer_seqs ok", 900, 180, [OUT_NV, GPU_ENG], "high",
    "unmodified scripts/apps/src/ai_bench.py: 8192^2 GEMMs, CNN training, BERT-like inference; minutes of GPU load, so it crosses several TDR windows",
    linux=["torch_ai_bench"], pkgs=PYPK)
app("cupy_check", "torch", py("cupy_check.py"), r"CUPY_DONE", 300, 30, [GPU_ENG], *W_COMPUTE_BIG, linux=["cupy"], pkgs=PYPK + ("cuda_cudart",),
    note="unmodified scripts/apps/src/cupy_check.py via kf_runpy.py (torch imported first so CuPy finds the CUDA DLLs)")
app("torch_burn", "stress", py("torch_burn.py", "60"), r"^GPU 0: OK", 180, 70, [OUT_NV, GPU_ENG], *W_LONG, linux=["gpu_burn"], pkgs=PYPK,
    note="60 s of fp32 matmul with bit-exact comparison (gpu_burn semantics)")
app("ort_directml", "d3d", py("ort_dml.py"), r"^ORT_DML_DONE", 240, 20, [GPU_ENG], "high",
    "ONNX Runtime DirectML = Direct3D 12 compute (root signature, UAV, DIRECT queue) on the NVIDIA adapter; D3D12 device creation was the wall of the "
    "2026-10-08 runs (D3D devices created by a test process RC their twins)", pkgs=PYPK)

# ---------------------------------------------------------------------------------------------- E. LLM
MODEL = r'$m = "$K\models\qwen2.5-1.5b-instruct-q4_k_m.gguf"' + "\n"
app("llama_cuda_gen", "llm", MODEL + r"""$exe = "$K\llama_cuda\llama-completion.exe"
'' | Set-Content "$OUT\empty.txt"
$p = Start-Process -FilePath $exe -ArgumentList @('-m', $m, '-p', '"Explain in three sentences why the sky is blue."', '-n', '64', '-ngl', '99', '--temp', '0', '--seed', '1', '--no-display-prompt') `
    -RedirectStandardInput "$OUT\empty.txt" -RedirectStandardOutput "$OUT\gen.txt" -RedirectStandardError "$OUT\gen.err" -Wait -PassThru -NoNewWindow
Get-Content "$OUT\gen.err" -ErrorAction SilentlyContinue | Select-Object -First 400 | ForEach-Object { "$_" }
Get-Content "$OUT\gen.txt" -ErrorAction SilentlyContinue | ForEach-Object { "GEN $_" }
$txt = (Get-Content "$OUT\gen.txt" -Raw -ErrorAction SilentlyContinue)
if ($p.ExitCode -eq 0 -and $txt -and $txt.Trim().Length -gt 20) { "OUTSHA llama_cpp $(Get-KfDigest ($txt -replace '\s+', ' '))" ; exit 0 } else { 'LLAMA_FAIL no output'; exit 1 }""",
    r"^OUTSHA llama_cpp ", 600, 60, [r"out:Device 0: NVIDIA|offloaded [0-9]+/[0-9]+ layers to GPU|CUDA0", GPU_ENG], "high",
    "greedy generation: ~1 GiB of weights, thousands of small kernel launches per second through the relay; Linux row passes with identical digests",
    linux=["llama_cpp_gen"], pkgs=("llama_cuda", "qwen_gguf", "vc_redist"), digest=True,
    note="digest is compared across Windows runs and informationally with Linux (different llama.cpp build)")
app("llama_cuda_bench", "llm", MODEL + r"""Invoke-KfExe -Exe "$K\llama_cuda\llama-bench.exe" -ArgList @('-m', $m, '-ngl', '99', '-p', '512', '-n', '64', '-r', '2')
exit $global:KfRc""", r"tg64", 900, 90, [r"out:CUDA|NVIDIA", GPU_ENG], *W_LONG, linux=["llama_bench"], pkgs=("llama_cuda", "qwen_gguf", "vc_redist"))
app("llama_vulkan_bench", "llm", MODEL + r"""Invoke-KfExe -Exe "$K\llama_vulkan\llama-bench.exe" -ArgList @('-m', $m, '-ngl', '99', '-p', '512', '-n', '64', '-r', '2')
exit $global:KfRc""", r"tg64", 900, 90, [r"out:Vulkan|NVIDIA", GPU_ENG], "high",
    "Vulkan compute (not CUDA) with 1.5 GiB device-local buffers and many dispatches: the Vulkan queue family/ICD path of the Windows driver",
    pkgs=("llama_vulkan", "qwen_gguf", "vc_redist"))

# ---------------------------------------------------------------------------------------------- F. Vulkan
app("vulkaninfo", "vulkan", r"""Invoke-KfExe -Exe "$K\vulkan_rt\x64\vulkaninfo.exe" -ArgList @('--summary')
exit $global:KfRc""", r"deviceName\s*=\s*NVIDIA", 60, 5, [r"out:deviceName\s*=\s*NVIDIA"], "low",
    "instance/physical-device queries only (no queue submit); the NVIDIA ICD must load", linux=["vulkaninfo"], pkgs=("vulkan_rt", "vc_redist"), smi=False)
app("vkpeak", "vulkan", r"""Invoke-KfExe -Exe "$K\vkpeak\vkpeak.exe" -ArgList @('0')
exit $global:KfRc""", r"fp32-scalar", 600, 60, [r"out:NVIDIA", GPU_ENG], "high",
    "Vulkan compute peak tests: long dependent-FMA dispatches (seconds each) and a bandwidth test; submit/fence on a compute queue", linux=["vkpeak"],
    pkgs=("vkpeak", "vc_redist"))
app("vkcube", "vulkan", r"""$exe = Find-KfFile -Root "$K\vulkan_sdk" -Name vkcube.exe
if (-not $exe) { 'VKCUBE_NOTFOUND (SDK install did not produce vkcube.exe)'; exit 1 }
Invoke-KfWait -Exe $exe -ArgList @('--c', '600') -TimeoutS 90 -Tag vkcube
exit $global:KfRc""", r"KFWAIT vkcube EXITED rc=0", 180, 15, [GPU_ENG], *W_GFX, pkgs=("vulkan_sdk", "vc_redist"), session="interactive", tier=2, shot=5,
    note="installs the LunarG SDK silently (tier 2): the only portable-less item; vkcube presents through a Vulkan swapchain to the DWM window")

# ---------------------------------------------------------------------------------------------- G. OpenGL
app("glgears_wgl", "opengl", r"""Invoke-KfExe -Exe "$K\tools\kf_glgears.exe" -ArgList @('--seconds', '10')
exit $global:KfRc""", r"^KFGL RESULT OK", 90, 15, [r"out:renderer .*NVIDIA", GPU_ENG], *W_GFX, linux=["egl_offscreen"], pkgs=("vc_redist",), session="interactive", shot=5,
    note="WGL window, colour read-back, gears; replaces the Linux EGL device-platform probe (EGL surfaceless is Linux-only)")
GT = r'$gt = "$K\gputest"' + "\n"
for test, dur, lin in (("triangle", 8, None), ("fur", 10, None), ("tess", 10, None), ("pixmark_piano", 10, None), ("plot3d", 10, None), ("gi", 10, None)):
    app("gputest_" + test, "opengl", GT + rf"""Remove-Item "$gt\_geeks3d_gputest_log.txt" -ErrorAction SilentlyContinue
Invoke-KfWait -Exe "$gt\GpuTest.exe" -ArgList @('/test={test}', '/width=1280', '/height=720', '/benchmark', '/benchmark_duration_ms={dur * 1000}', '/no_scorebox') -Cwd $gt -TimeoutS {dur + 60} -Tag gputest_{test}
$rc = $global:KfRc
Get-Content "$gt\_geeks3d_gputest_log.txt" -ErrorAction SilentlyContinue | Select-String -Pattern 'Renderer model|enchmark|score|Score|error|Error' | ForEach-Object {{ "GPUTEST $_" }}
exit $rc""", rf"GPUTEST .*Renderer model: .*NVIDIA", dur + 90, dur + 15, [r"out:Renderer model: .*NVIDIA", GPU_ENG], *W_GFX,
        pkgs=("gputest", "vc_redist"), session="interactive", tier=1 if test in ("triangle", "fur") else 2, shot=5,
        note="Geeks3D GpuTest 0.7.0 (2014, OpenGL 2.1-4.x); /benchmark with a fixed duration and no score box", fail_rx=r"^CHECK .*FAIL|GPUTEST .*[Ee]rror: ")
FM = r'$fm = "$K\furmark2"' + "\n"
for aid, demo, extra, secs, lin, cat in (("furmark_glinfo", None, ["--glrenderer"], 0, None, "opengl"), ("furmark_vkinfo", None, ["--vkinfo"], 0, None, "vulkan"),
                                         ("furmark_gl_bench", "furmark-gl", [], 15, None, "opengl"), ("furmark_vk_bench", "furmark-vk", [], 15, None, "vulkan"),
                                         ("furmark_knot_gl", "furmark-knot-gl", [], 15, None, "opengl"), ("furmark_gl_stress", "furmark-gl", ["STRESS"], 60, ["gpu_burn"], "stress")):
    if demo is None:
        body = FM + "Invoke-KfExe -Exe \"$fm\\furmark.exe\" -ArgList @(%s) -Cwd $fm\nexit $global:KfRc\n" % ", ".join("'%s'" % x for x in extra)
        app(aid, cat, body, r"NVIDIA", 90, 8, [r"out:NVIDIA"], "medium" if cat == "opengl" else "medium",
            "creates a GL/Vulkan context on the NVIDIA device and prints driver strings; no sustained rendering", pkgs=("furmark2", "vc_redist"), session="interactive",
            smi=False, tier=1)
    else:
        stress = "STRESS" in extra
        args = ["--demo", demo, "--width", "1280", "--height", "720", "--max-time", str(secs), "--no-score-box"] + ([] if stress else ["--benchmark"])
        body = FM + rf"""Remove-Item "$fm\_scores.csv" -ErrorAction SilentlyContinue
Invoke-KfWait -Exe "$fm\furmark.exe" -ArgList @({", ".join("'%s'" % x for x in args)}) -Cwd $fm -TimeoutS {secs + 90} -Tag {aid}
$rc = $global:KfRc
if (Test-Path "$fm\_scores.csv") {{ Get-Content "$fm\_scores.csv" | Select-Object -Last 3 | ForEach-Object {{ "FM_SCORE $_" }} }}
Get-ChildItem $fm -Filter '*.log' -ErrorAction SilentlyContinue | Select-Object -First 1 | ForEach-Object {{ Get-Content $_.FullName | Select-String 'GL_RENDERER|Renderer|NVIDIA' | Select-Object -First 5 | ForEach-Object {{ "FM_LOG $_" }} }}
exit $rc"""
        app(aid, cat, body, r"KFWAIT %s EXITED rc=0" % aid, secs + 120, secs + 10, [r"out:NVIDIA", GPU_ENG], *(W_LONG if secs >= 60 else W_GFX), linux=lin or (),
            pkgs=("furmark2", "vc_redist"), session="interactive", shot=min(8, secs - 2), tier=1 if secs <= 15 else 2,
            note="FurMark 2.10.2 command line from its help (--demo/--benchmark/--max-time/--no-score-box); the stress variant is the closest "
                 "free counterpart of gpu_burn's heat, not of its correctness check" if stress else "FurMark 2.10.2 command line from its help text")

# ---------------------------------------------------------------------------------------------- H. Direct3D 11/12
DX = r'$dx = "$K\tools\kf_dxprobe.exe"' + "\n"
app("dxprobe_d3d11", "d3d", DX + r"""Invoke-KfExe -Exe $dx -ArgList @('d3d11', '--iters', '200')
exit $global:KfRc""", r"^KFDX RESULT OK", 120, 8, [r'out:KFDX selected "NVIDIA', GPU_ENG], "high",
    "D3D11 device on the NVIDIA adapter, a compute dispatch and a draw read back through a staging resource; WARP excluded by construction",
    pkgs=("vc_redist",))
app("dxprobe_d3d12", "d3d", DX + r"""Invoke-KfExe -Exe $dx -ArgList @('d3d12', '--iters', '200')
exit $global:KfRc""", r"^KFDX RESULT OK", 120, 8, [r'out:KFDX selected "NVIDIA', GPU_ENG], "high",
    "D3D12 device, DIRECT+COPY queue fences, a compute PSO with a root UAV and a readback: kernel-mode WDDM2 device creation was the wall of runs 53-72", pkgs=("vc_redist",))
GM = r'''$gmx = Find-KfFile -Root "$K\gravitymark" -Name GravityMark.exe
if (-not $gmx) { 'GM_NOTFOUND'; exit 1 }
$gmd = Split-Path $gmx
'''
for aid, api, extra, tier, dif in (("gravitymark_d3d11", "-d3d11", [], 1, W_GFX), ("gravitymark_d3d12", "-d3d12", [], 1, W_GFX), ("gravitymark_vk", "-vk", [], 1, W_GFX),
                                   ("gravitymark_gl", "-gl", [], 2, W_GFX), ("gravitymark_d3d12_raster", "-d3d12", ["-raster", "1"], 2, W_GFX),
                                   ("gravitymark_d3d12_rt", "-d3d12", ["-raytracing", "1"], 3, ("high", "DXR ray tracing on RT cores through the Translated/Passthrough split: the least-trodden path"))):
    args = ["-temporal", "1", "-fps", "1", "-info", "1", "-benchmark", "1", "-close", "1", "-count", "1", "-asteroids", "20000",
            "-width", "1280", "-height", "720", "-fullscreen", "0", api] + extra
    app(aid, "d3d" if "d3d" in aid else ("vulkan" if aid.endswith("vk") else "opengl"), GM + rf"""Invoke-KfWait -Exe $gmx -ArgList @({", ".join("'%s'" % x for x in args)}, '-image', "$OUT\gm.png", '-times', "$OUT\gm_times.txt") -Cwd $gmd -TimeoutS 300 -Tag {aid}
$rc = $global:KfRc
if ((Test-Path "$OUT\gm.png") -and ((Get-Item "$OUT\gm.png").Length -gt 10000)) {{ "GM_OK image=$((Get-Item "$OUT\gm.png").Length) bytes" }} else {{ 'GM_FAIL no benchmark image' ; if ($rc -eq 0) {{ $rc = 1 }} }}
if (Test-Path "$OUT\gm_times.txt") {{ "GM_TIMES lines=$((Get-Content "$OUT\gm_times.txt").Count)" }}
exit $rc""", r"^GM_OK", 360, 60, [GPU_ENG], *dif, pkgs=("gravitymark", "vc_redist"), session="interactive", tier=tier, shot=20,
        note="GravityMark 1.89 flags from its own run_*.bat (-benchmark 1 -close 1 -count 1 -image -times); asteroids reduced to 20000")
UN = r'$u = "$K\%s\bin"' + "\n"
for aid, name, api, tier in (("heaven_d3d11", "heaven", "direct3d11", 2), ("heaven_opengl", "heaven", "opengl", 2), ("valley_d3d11", "valley", "direct3d11", 2), ("valley_opengl", "valley", "opengl", 2)):
    exe = "Heaven" if name == "heaven" else "Valley"
    cfg = "heaven_4.0.cfg" if name == "heaven" else "valley_1.0.cfg"
    scr = "heaven/unigine.cpp" if name == "heaven" else "valley/unigine.cpp"
    proj = exe
    args = ["-project_name", proj, "-data_path", "../", "-engine_config", "../data/" + cfg, "-system_script", scr, "-sound_app", "null",
            "-video_app", api, "-video_multisample", "0", "-video_fullscreen", "0", "-video_mode", "-1", "-video_width", "1280", "-video_height", "720",
            "-extern_define", "RELEASE,QUALITY_MEDIUM"]
    app(aid, "d3d" if api == "direct3d11" else "opengl", (UN % name) + "Invoke-KfSurvive -Exe \"$u\\%s.exe\" -ArgList @(%s) -Cwd $u -Seconds 45 -Tag %s\nexit $global:KfRc\n" % (
        exe, ", ".join("'%s'" % x for x in args), aid), r"KFSURVIVE %s ALIVE_AT" % aid, 150, 50, [GPU_ENG], *W_GFX, pkgs=(name, "vc_redist"), session="interactive",
        tier=tier, shot=30, note="Unigine %s Basic (32-bit); command line rebuilt from the launcher's own js (launcher/js/*.js); Basic has no scripted benchmark, so the "
        "app auto-plays its camera path for 45 s and must still be alive" % exe)

# ---------------------------------------------------------------------------------------------- I. video (ffmpeg)
FF = r'''$ff = "$K\ffmpeg\bin\ffmpeg.exe"; $fp = "$K\ffmpeg\bin\ffprobe.exe"
'''
SRC = "testsrc=size=1280x720:rate=30:duration=20"


def enc(codec):
    return FF + rf"""Invoke-KfExe -Exe $ff -ArgList @('-y', '-hide_banner', '-nostats', '-f', 'lavfi', '-i', '{SRC}', '-c:v', '{codec}', '-preset', 'p4', "$OUT\enc.mp4")
$rc = $global:KfRc
$n = & $fp -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 "$OUT\enc.mp4"
"frame= $n "
exit $rc"""


app("nvenc_h264", "video", enc("h264_nvenc"), r"frame= *600 ", 180, 20, ["pdh:VideoEncode", "smi"], *W_VIDEO, linux=["nvenc_h264"], pkgs=("ffmpeg", "vc_redist"))
app("nvenc_hevc", "video", enc("hevc_nvenc"), r"frame= *600 ", 180, 20, ["pdh:VideoEncode", "smi"], *W_VIDEO, linux=["nvenc_hevc"], pkgs=("ffmpeg", "vc_redist"))
app("nvenc_av1", "video", enc("av1_nvenc"), r"frame= *600 ", 180, 20, ["pdh:VideoEncode", "smi"], *W_VIDEO, pkgs=("ffmpeg", "vc_redist"),
    note="AV1 encode exists on Ada (AD104) and Blackwell, not on Ampere: NOTRUN-by-design on older GPUs")


def dec(hw_args, tag, fail_msg):
    return FF + rf"""Invoke-KfExe -Exe $ff -ArgList @('-y', '-hide_banner', '-nostats', '-f', 'lavfi', '-i', '{SRC}', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', "$OUT\x264.mp4")
if ($global:KfRc -ne 0) {{ 'DECODE_PREP_FAIL libx264 encode'; exit 1 }}
Invoke-KfExe -Exe $ff -ArgList @('-hide_banner', '-nostats', '-progress', "$OUT\prog.txt", {hw_args}, '-i', "$OUT\x264.mp4", '-f', 'null', '-')
$rc = $global:KfRc
$log = (Get-Content "$OUT\prog.txt" -ErrorAction SilentlyContinue | Where-Object {{ $_ -match '^frame=' }} | Select-Object -Last 1)
"frame= $($log -replace 'frame=', '') "
exit $rc"""


app("nvdec_h264", "video", dec("'-hwaccel', 'cuda', '-hwaccel_output_format', 'cuda'", "cuda", ""), r"frame= *600 ", 180, 20, ["pdh:VideoDecode", "smi"], *W_VIDEO,
    linux=["nvdec_h264"], pkgs=("ffmpeg", "vc_redist"), fail_rx=r"^CHECK .*FAIL|Failed setup for format|DECODE_PREP_FAIL|hwaccel initialisation returned error")
app("d3d11va_h264", "video", dec("'-hwaccel', 'd3d11va', '-hwaccel_output_format', 'd3d11'", "d3d11va", ""), r"frame= *600 ", 180, 20, ["pdh:VideoDecode", "smi"], *W_VIDEO,
    pkgs=("ffmpeg", "vc_redist"), fail_rx=r"^CHECK .*FAIL|Failed setup for format|DECODE_PREP_FAIL|hwaccel initialisation returned error",
    note="the DXVA2/D3D11 video-decode API path (what Edge, Media Foundation and VLC use); the Linux matrix has no counterpart")
app("dxva2_h264", "video", dec("'-hwaccel', 'dxva2'", "dxva2", ""), r"frame= *600 ", 180, 20, ["pdh:VideoDecode", "smi"], *W_VIDEO, pkgs=("ffmpeg", "vc_redist"),
    fail_rx=r"^CHECK .*FAIL|Failed setup for format|DECODE_PREP_FAIL|hwaccel initialisation returned error", tier=2)
app("scale_cuda_nvenc", "video", FF + rf"""Invoke-KfExe -Exe $ff -ArgList @('-y', '-hide_banner', '-nostats', '-f', 'lavfi', '-i', '{SRC}', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', "$OUT\x264.mp4")
Invoke-KfExe -Exe $ff -ArgList @('-y', '-hide_banner', '-nostats', '-hwaccel', 'cuda', '-hwaccel_output_format', 'cuda', '-i', "$OUT\x264.mp4", '-vf', 'scale_cuda=640:360', '-c:v', 'h264_nvenc', "$OUT\scaled.mp4")
$rc = $global:KfRc
$n = & $fp -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 "$OUT\scaled.mp4"
"frame= $n "
exit $rc""", r"frame= *600 ", 240, 30, ["pdh:VideoEncode|VideoDecode", "smi"], *W_VIDEO, pkgs=("ffmpeg", "vc_redist"), tier=2,
    fail_rx=r"^CHECK .*FAIL|Failed setup for format|Impossible to convert", note="NVDEC -> CUDA scale filter -> NVENC with frames kept on the GPU")
app("vulkan_video_decode", "video", dec("'-init_hw_device', 'vulkan=vk', '-hwaccel', 'vulkan', '-hwaccel_output_format', 'vulkan'", "vulkan", ""), r"frame= *600 ", 240, 30,
    ["pdh:VideoDecode", "smi"], *W_VIDEO, pkgs=("ffmpeg", "vc_redist"), tier=3,
    fail_rx=r"^CHECK .*FAIL|Failed setup for format|DECODE_PREP_FAIL|hwaccel initialisation returned error|Cannot load", note="Vulkan Video decode extension; optional")

# ---------------------------------------------------------------------------------------------- J. render, crypto, OpenCL
app("blender_cycles", "render", r"""$bl = "$K\blender\blender.exe"
foreach ($dev in 'CUDA', 'OPTIX') {
    Invoke-KfExe -Exe $bl -ArgList @('-b', '--factory-startup', '--python', 'C:\kf\py\blender_render.py', '--', $dev, "$OUT\blender_$dev.png") | Where-Object { $_ -match 'BLENDER_|Error|error|Finished' } | Select-Object -Last 8
}
exit 0""", r"BLENDER_OK OPTIX", 900, 120, [r"out:BLENDER_OK .* gpu=NVIDIA|gpu=NVIDIA", GPU_ENG], "high",
    "Cycles on CUDA then OptiX (RT cores) on a scripted scene; minutes of mixed compute and BVH work; OptiX needs the driver's nvoptix.dll", linux=["blender_cycles"],
    pkgs=("blender", "vc_redist"), fail_rx=r"^CHECK .*FAIL|BLENDER_FAIL", note="the same scripts/apps/src/blender_render.py as the Linux row; CUDA and OPTIX are rendered one after the other")
app("hashcat", "crypto", r"""$env:PATH = "$TL;$env:PATH"
Invoke-KfExe -Exe "$K\hashcat\hashcat.exe" -ArgList @('--potfile-disable', '-O', '-m', '0', '-a', '3', 'e4726719b68b205913167f0975d977ee', '?l?l?l?l?l?l') -Cwd "$K\hashcat"
exit 0""", r"e4726719b68b205913167f0975d977ee:kayfab", 300, 40, [r"out:NVIDIA|CUDA|OpenCL", GPU_ENG], "medium",
    "md5 mask attack; hashcat prefers its CUDA backend (needs nvrtc64_*.dll, found on PATH from the torch wheel) and falls back to the OpenCL ICD",
    linux=["hashcat"], pkgs=("hashcat", "sevenzip", "pywheels", "python_embed", "vc_redist"), note="hashcat is flagged HackTool by Defender: kf_guest_setup excludes C:\\kfapps")
app("clpeak_ocl", "opencl", r"""Invoke-KfExe -Exe "$K\clpeak_ocl\bin\clpeak.exe" -Cwd "$K\clpeak_ocl\bin"
exit $global:KfRc""", r"Global memory bandwidth", 600, 60, [r"out:NVIDIA", GPU_ENG], *W_LONG, linux=["clpeak"], pkgs=("clpeak_ocl", "vc_redist"),
    fail_rx=r"^CHECK .*FAIL|Tests skipped|clFinish \(-")
app("clpeak_cuda", "opencl", r"""Invoke-KfExe -Exe "$K\clpeak_cuda\bin\clpeak.exe" -Cwd "$K\clpeak_cuda\bin"
exit $global:KfRc""", r"Global memory bandwidth", 600, 60, [r"out:NVIDIA", GPU_ENG], *W_LONG, pkgs=("clpeak_cuda", "vc_redist"),
    fail_rx=r"^CHECK .*FAIL|Tests skipped|clFinish \(-", note="clpeak 3.0.1's CUDA backend build")
app("opencl_icd", "opencl", r"""$k = 'HKLM:\SOFTWARE\Khronos\OpenCL\Vendors'
if (Test-Path $k) { (Get-Item $k).Property | ForEach-Object { "OPENCL_ICD $_" } } else { 'OPENCL_ICD (none registered)' }
exit 0""", r"OPENCL_ICD .*nvopencl", 60, 2, [r"out:nvopencl"], "low", "registry only; proves the ICD is installed (the clinfo row's NVIDIA platform check)", linux=["clinfo"], smi=False)

# ---------------------------------------------------------------------------------------------- K. browsers (Edge)
def edge(page, secs, extra=""):
    return rf"""& C:\kf\kf_edge.ps1 -Id @@ID@@ -Page {page} -Seconds {secs}{extra} 2>&1 | ForEach-Object {{ "$_" }}
exit $LASTEXITCODE"""


app("edge_webgl", "browser", edge("webgl", 12), r'^KFEDGE \{.*"ok":true', 150, 30, [r'out:"renderer":"[^"]*NVIDIA', "pdh:3D|Compute|Copy"], *W_GFX, pkgs=(), session="interactive",
    shot=8, smi=True, note="Edge (inbox) WebGL2 through ANGLE/D3D11; the page reports the unmasked renderer and read-back; replaces nothing on Linux")
app("edge_webgl_headless", "browser", edge("webgl", 8, " -Headless 1"), r'^KFEDGE \{.*"ok":true', 150, 30, [r'out:"renderer":"[^"]*NVIDIA', "pdh:3D|Compute|Copy"], "medium",
    "no window or display path: only the GPU process's D3D11 device (ANGLE)", pkgs=(), session="interactive")
app("edge_webgpu", "browser", edge("webgpu", 12), r'^KFEDGE \{.*"ok":true', 150, 30, [r'out:"vendor":"nvidia"', "pdh:Compute|3D|Copy"], *W_GFX, pkgs=(), session="interactive", shot=8,
    note="WebGPU on the D3D12 backend: compute pipeline + render pass each frame; mapAsync read-back")
app("edge_video_h264", "video", edge("video", 15, " -Codec h264"), r'^KFEDGE \{.*"ok":true', 180, 45, ["pdh:VideoDecode"], *W_VIDEO, pkgs=("ffmpeg", "vc_redist"), session="interactive", shot=8,
    note="the owner's own Edge + YouTube Shorts scenario without the internet: a generated 720p H.264 clip played by <video>; HW decode proof = VideoDecode engine activity")
app("edge_video_vp9", "video", edge("video", 15, " -Codec vp9"), r'^KFEDGE \{.*"ok":true', 180, 45, ["pdh:VideoDecode"], *W_VIDEO, pkgs=("ffmpeg", "vc_redist"), session="interactive",
    tier=2, note="VP9 decode through DXVA (Edge)")

# ---------------------------------------------------------------------------------------------- L. Windows inbox
app("dxdiag_report", "probe", r"""$f = "$OUT\dxdiag.txt"
Start-Process dxdiag.exe -ArgumentList @('/whql:off', '/t', $f) -Wait -NoNewWindow
for ($i = 0; $i -lt 60 -and -not (Test-Path $f); $i++) { Start-Sleep -Seconds 1 }
Start-Sleep -Seconds 3
if (Test-Path $f) {
    Get-Content $f | Select-String -Pattern 'Card name|Chip type|Driver Version|DDI Version|Feature Levels|Display Memory|Driver Model|Basic Render' | ForEach-Object { "DXDIAG $($_.Line.Trim())" }
    if ((Get-Content $f -Raw) -match 'Card name: .*NVIDIA') { 'DXDIAG_OK'; exit 0 }
}
'DXDIAG_FAIL'; exit 1""", r"^DXDIAG_OK", 180, 25, [r"out:Card name: .*NVIDIA"], "medium",
    "dxdiag instantiates D3D9/10/11/12 devices to fill its report; any of them failing shows as missing feature levels", pkgs=(), smi=False,
    fail_rx=r"Card name: .*Microsoft Basic Render")

# ---------------------------------------------------------------------------------------------- Linux matrix -> Windows map
# every row of scripts/apps/run_apps.sh must appear here (tests/test_apps.py checks that) either with the Windows apps
# that stand for it (derived from `linux=` above) or with the reason there is none.
NO_EQUIVALENT = {
    "simpleCallback": "cudaLaunchHostFunc/stream callbacks are not exposed by PyTorch and no prebuilt binary exists; a C++ build needs nvcc + MSVC, which the guest lacks",
    "simpleOccupancy": "occupancy API sample needs a cudart C++ build (nvcc + MSVC); NVRTC/CuPy exposes no occupancy sample",
    "simpleIPC": "CUDA IPC (cudaIpcGetMemHandle) is not supported on WDDM Windows (TCC/Linux only)",
    "cdpSimpleQuicksort": "dynamic parallelism needs relocatable device code linked with cudadevrt; no prebuilt Windows binary and no NVRTC route without nvcc/MSVC",
    "hf_generate": "dropped for size: transformers + tokenizers wheels and a 1 GB fp16 model; torch_correct/torch_ai_bench and the llama.cpp rows cover the same stacks",
    "geekbench_gpu": "Geekbench needs the internet to upload results (the Linux pass string is 'Uploading results'); the lane forbids guest internet at run time",
}


def build():
    apps = sorted(APPS, key=lambda a: a["id"])
    linux_map = {}
    for a in apps:
        for l in a["linux"]:
            linux_map.setdefault(l, []).append(a["id"])
    return dict(schema=1, generator="gen_apps.py", prelude=PRELUDE, apps=apps,
                linux_map={k: sorted(v) for k, v in sorted(linux_map.items())}, no_equivalent=NO_EQUIVALENT)


def main(argv):
    out = os.path.join(HERE, "apps.json")
    data = json.dumps(build(), indent=1, sort_keys=False) + "\n"
    if len(argv) > 1 and argv[1] == "--check":
        cur = open(out).read() if os.path.exists(out) else ""
        if cur != data:
            print("apps.json is out of date: run gen_apps.py")
            return 1
        print("apps.json up to date")
        return 0
    with open(out, "w") as f:
        f.write(data)
    print(f"wrote {out}: {len(APPS)} apps")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
