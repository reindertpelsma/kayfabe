# E5 compression-store instrument

STATUS: RESEARCH, 2026-10-26.

`e5_alloc_map.c` implements only E5(a,b): an unprivileged raw-RM allocation,
compression coverage query, four kind/page-size maps and read-only PTE queries.
It performs no fill, clear, shader, copy-engine submission or VM operation.

Build on the test host using the open NVIDIA 595.91.07 source headers:

```sh
OGKM_ROOT=/path/to/open-gpu-kernel-modules ./build.sh ./e5_alloc_map
```

Hold the shared GPU lease before running. Run as a non-root user with no effective
capabilities. The client owns every object it frees; freeing a virtual allocation
tears down its mapping before freeing the physical backing. Every syscall/RM status
and PTE block is emitted as JSONL. A refused control is not an empty successful result.

```sh
python3 e5_gate.py probe.jsonl
python3 -m unittest -v test_e5_gate
python3 mutate_gate.py
```

The interpreter requires matching valid PTEs for both page sizes and both requested
kinds. A raw-mode zero comptag-line field is not by itself a compression failure.
Returned ZBC preference is reported separately from allocation/PTE observations.
The interpreter cannot certify ZBC suppression, fresh compression state, bandwidth
or admission to a compression-store product intervention.

CODE READ open595 `mem_mgr_tu102.c`, `mem_mgr_tu102_base.c` and
`mem_mgr_gm107.c`: kind selection, ZBC classification and the returned ZBC preference
must be checked independently. No global hardware ZBC table control is implemented.
