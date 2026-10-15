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

Follow-up hypothesis, before the added check: the engine's existing nonzero-ISO
predicate mistakes retained CAX address/target bits for an enabled surface, even
when ENABLE is clear. Falsifier: a disabled, nonzero LO_ISO word is already
classified inactive. Test the activation predicate after the disable latch,
then resolve the enable field from the generated table if it fails.

## GPU-free result

Revisionfc40d0017:477tests pass,0fail,2ignored, across kf-disp and kf-qemu;
checkout-specific target /data/codex/target-blackwell. All-family realization
resolves580.65.06/580.159.04/595.91.07. The added disable predicate fails before
fixing ENABLE and passes after; legacy context-DMA windows keep their nonzero
handle predicate. New model checks cover latched address0/high64bits, allfour
apertures, independent KIND/ENABLE, plane1, cache identity, RGB last-byte bounds,
explicit YUV chroma addresses and a disabled chroma refusal. No CPU GPU executor.

No Blackwell hardware result exists for this change. The merged Ada candidate
will exercise ordinary context-DMA display as a regression, but even a passing
Ada run cannot establish Blackwell's address-named hardware behavior. Linux
fast30/30 +broker, Windows Code0/DWM/D3D/zeroTDR and Blackwell native reference
remain required before any family-coverage or promotion claim.
