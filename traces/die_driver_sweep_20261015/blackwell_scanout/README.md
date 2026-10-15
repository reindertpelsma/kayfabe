# Blackwell address-named window scanout

**STATUS: LIVE, 2026-10-15.** GPU-free candidate; no hardware claim.

Before implementation, the QEMU realize regression must require a scanout vocabulary
for every family, including GB20x. The existing regression explicitly expects GB20x
to lack one. The known gap leaves its console on the boot layer indefinitely.

Hypothesis: resolving ISO surface-address methods from the generated class table
and bounding framebuffer reads to the guest's store allows the existing GPU compositor
to show Blackwell windows. Falsifier: the all-family realization test fails, method
streams decode wrong address/enable/aperture/kind, or a GPU-free boundary test admits
a span outside that store. Hardware display success is a separate, unrun falsifier.

Source: ogkm580.159.04 b81d58ee0224d1d290bef1c080592b619e184042,
nvkms-evo4.c:GetSurfaceAddress/EvoSetISOSurfaceAddressC9 and clca7e.h
SET_SURFACE_ADDRESS_HI/LO_ISO. Address high32 plus low31:4; independent
ENABLE, TARGET and KIND. The whole framebuffer store belongs to this guest;
IOVA has no display IOMMU on discrete GPUs, and system-memory scanout remains
unsupported by the existing compositor. Neither aperture may be treated as vidmem.
