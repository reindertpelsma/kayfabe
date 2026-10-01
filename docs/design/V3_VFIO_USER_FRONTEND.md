# V3 — a vfio-user frontend: can an unpatched VMM host kayfabe?

**STATUS: DESIGN-ONLY, 2026-10-01 (owner direction, not built).** Owner: *"we should fold it in, even
though it costs perf, a vfio-user backend can help. The only problem is we require a lot of small
bar1/2 maps and vfio-user does that using kvm memslots in shm_map. So I still doubt if unpatched qemu
is possible."* The reason to want it: an install path. Today kayfabe needs a QEMU built with the
out-of-tree `kf3` overlay (`docs/PRODUCT_POSITIONING.md`, 2026-10-01 note: "easier than VFIO, not
merely possible"). A vfio-user server would let an unmodified VMM with a vfio-user client (QEMU, cloud-
hypervisor) attach kayfabe as an out-of-process PCI device.

## 1. Why the owner's doubt holds for today's design

kf3 installs BAR1/BAR2 windows as QEMU memory subregions (`qemu/hw/misc/kf3/kf3.c`:
`memory_region_init_ram_device_ptr` + `memory_region_add_subregion`/`del_subregion`), i.e. KVM memslots,
and adds/removes them as the guest's BAR1/BAR2 mappings change. One archived app-matrix boot running
`llama-bench` (kf3 `4c48ca0c`, `traces/v3_app_matrix/vast53004208_rtx3060_4c48ca0c/all_logs.tgz` →
`m20/llama_bench.kf3.log`, last status line) counted **`views[armed=9193 released=9191]`** and
**`pramin_repoints=1362`**: thousands of CPU-visible remaps per boot.

In vfio-user the server declares a region's mmap areas (fd + sparse areas) when the client asks for
region info; the client then creates its mappings once. As far as checked (2026-10-01, from the
protocol's design — re-verify against the current spec before building), there is no server-initiated
message to add or remove an mmap area afterwards. ⇒ With today's design an unpatched client could only
TRAP those BAR accesses as socket messages — functional, far too slow for BAR1/BAR2 traffic.

## 2. Options, in the order to try them

1. **Static CPU side, dynamism in the host GPU's BAR1 MMU.** The guest BAR1 already must fit inside the
   host's BAR1 aperture (`scripts/fastguest/run_fast_guest.sh`, the 128 MiB guest BAR1 note), so guest
   BAR1 views are host BAR1 mappings. If kayfabe can reserve one host BAR1 range per VM, map it into the
   VMM once, and retarget its pages by host BAR1 page-table updates, each BAR becomes ONE static mmap —
   the guest's BAR1 translation then lives where hardware keeps it (the GPU MMU). This makes an unpatched
   vfio-user client possible AND removes kf3's in-process memslot churn (each memslot change also costs
   vCPU time). ⚠ Open: does host RM let an unprivileged client remap pages inside a fixed BAR1 range?
   Read ogkm (`kern_bus`/BAR1 VAS, `NVOS46` fixed-offset maps) before anything else. PRAMIN repoints and
   BAR2 windows need the same treatment or a separate answer.
2. **A small, generic vfio-user extension:** server-initiated map/unmap of fd-backed windows within a
   region. QEMU already maps host memory into a BAR dynamically in-process for virtio-gpu blob resources,
   so the concept is not foreign; upstreamable in principle and useful beyond kayfabe. Until upstream it
   means a patched VMM — but a generic patch, far easier to carry than the `kf3` overlay.
3. **Trap fallback** for everything — a compatibility mode at best.

## 3. Other costs and notes

- Every other trapped register becomes a socket round trip; the in-process `kf3` stays the fast path
  and vfio-user is a second frontend, not a replacement (kayfabe v3 removed an earlier process split
  because it cost more than it gave — issue #1, owner's comment).
- Doorbells: vfio-user lets the server hand the client ioeventfds for a region, which would keep the
  ioeventfd fast path (`V3_DOORBELL_IOEVENTFD.md`). ⚠ Whether QEMU's client implements it: unverified.
- Interrupts via irqfd and guest-RAM DMA via the protocol's fd-backed DMA regions match what kayfabe
  already requires (`memory-backend-memfd,share=on`).
- **First experiment:** answer §2.1's RM question from ogkm; in parallel, split the measured churn into
  boot-time vs steady-state per workload (the counters exist: `views[...]`, `pramin_repoints`).
