# Native USERD cursor observations around ring-growth registration

**STATUS: RESEARCH, 2026-10-12.** Prepared; no cursor hardware result yet.

H-CURSOR-OBS: read-only sampling of native SYSTEM memory-list USERD pages captures
usable old/new cursor transitions during the same passing Gravity Vulkan workload.
Falsifier: no usable slot transitions, incompatible public memory-list/RAM layout,
collector error, or native workload failure. A PASS of the app alone is not a
cursor result. Even useful passive samples are not atomic GPU trigger execution
or a general old-backing retirement proof. Compare against previous183s reference;
only passive sampling is added, same QEMU source5a407dcd8/SHA1325fefb009c... .

A controller-side process opens only the owned native QEMU's guest-RAM memfd,
read-only. Verify actual QOM8GiB ram0 size and public PC RAM aliases from QMP.
Decode only public single-page SYSTEM-list requests, with page-count/length/PFN
bounds, then read that page from the RAM file. It cannot read GPU BAR MMIO or
arbitrary process virtual memory. Retain only GP_GET/GP_PUT/Reference for the
page's eight512-byte slots; other bytes are discarded. Two samples flag changed
fields; neither is claimed atomic. Alias names/offsets are matched, source fd
size/name is checked; mismatches fail rather than guessing physical translation.

Page candidates and free barriers carry allocation serials/generations; a passive
capture can omit earlier frees or aliases, so these are not product ownership pins.
Tail live GSP output has writer delay; registration observation need not precede
the GPU trigger. Full strict GSP footer/export-hash validation remains mandatory
after shutdown. Sample output has its own hash/footer,100000-record/330s bounds.
The1ms interval/observation deadline never releases or changes GPU work. Sampling
stops when the matrix finishes, before normal native VM shutdown.

Fresh disposable baseline child, personal1380 untouched; serial GPU lock and
post-run NVIDIA/DMA-FQ/disk checks. No privileged driver call, pause, private
driver analysis, disk/RAM export, or secret is added. GPU-free boundary/public
memory-list/field-selection checks pass. Linux/Windows product runtime unchanged.
