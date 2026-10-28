+++
title = "Benchmarks"
description = "Speed depends on the work. These results have limits; they are not a promise for your machine."
template = "detail.html"
[extra]
src = "README.md Linux CUDA table; docs/design/V3_LINUX_PERF_BASELINE_6692e621.md; prior landing sources below"
+++

## Windows: new measurements pending

<!-- src: README.md speed bullet (re-measurement on current master pending); traces/perf_status_20261017/README.md historical banner; commits lazy-prod 1/5 to 5/5 on master (2026-10-17) -->
Windows runs real 3D applications. Speed is being benchmarked again on current master, and the results will be posted here.

No Windows frame-rate figure is published here while that work is pending. For the applications that have run, see [What works today](/works-today/).

## Linux: large jobs and small launches

<!-- src: README.md Linux CUDA table and note; docs/design/V3_LINUX_PERF_BASELINE_6692e621.md -->
These older results come from separate campaigns on rented RTX 3060 hosts. Each host was itself a VM. They used older kayfabe revisions. Large jobs kept pace with the host. Many small GPU launches cost more.

| Workload | Guest / host | Context |
|---|---|---|
| Matrix multiply, N=2048 | 0.997 | Host 786 GFLOPS; guest 784 GFLOPS |
| llama.cpp prompt processing | 0.998 | Qwen2.5-1.5B Q4 |
| llama.cpp token generation | 0.877 | Same model |
| Device-to-device copy, 32 MB | 0.98 | Host 327 GB/s; guest 322 GB/s |
| Eager PyTorch decoding | 0.29 | Many small launches |
| NVENC, 1080p | 0.72 | Older box |

<!-- src: README.md Linux CUDA table; docs/design/V3_LINUX_PERF_BASELINE_6692e621.md -->
Creating a CUDA context took about eight times longer in the guest. The full 65-program run took 1,193 seconds in the guest and 1,075 on the host. Sixty-one programs passed with equal output. Four need unified-memory paging, which is not supported yet.

<!-- src: README.md Linux CUDA note; prior landing Where it stands -->
The same set has not been run on the author's RTX 4070 without a cloud VM in between. Do not treat these older results as a current hardware comparison.

## Reading the numbers

<!-- src: docs/howto/technical-faq.md Why is performance uneven; README.md Linux guest -->
Each ratio compares the guest with its host on the same box. A ratio near one means similar speed for that test. It does not mean that every application is fast.

The [performance FAQ](/@docs/FAQ.html#what-is-the-performance-cost) explains the workload costs. The [Linux guide](/@docs/linux-guest.html) covers test completion and the four programs that fail. Hardware coverage is on [What works today](/works-today/).
