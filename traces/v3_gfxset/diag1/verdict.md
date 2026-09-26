### /workspace/gfxset/results/diag1

| item | bare metal | guest | digests vs bare metal | verdict | detail |
|---|---|---|---|---|---|
| ff_vulkan_init | PASS | FAIL (batched) | 0/1 match DIFF:frame | **FAIL** | rc=187: [Vulkan @ 0x61f949a1fc80] Device creation failure: VK_ERROR_INITIALIZATION_FAILED 4s host_xid=0 guest_xid=0 kf3_rc=0 kf3_refusals=0 — GSET_FAIL ffmpeg vulkan init |
| vk_ofa | PASS | PASS (batched) | 2/2 match | **PASS** |  |
| pv_vk_rt_ext | PASS | PASS (batched) | 1/1 match | **PASS** |  |
| vk_info | PASS | PASS (batched) | 6/6 match | **PASS** |  |

FAIL: 1, PASS: 3
GSET_SUITE_SUMMARY pass=3/4 notrun_host=0
