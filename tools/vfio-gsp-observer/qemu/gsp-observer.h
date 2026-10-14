/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef VFIO_GSP_OBSERVER_H
#define VFIO_GSP_OBSERVER_H
#include "hw/pci/pci_device.h"
#include "hw/vfio/vfio-device.h"

/* ── The device-agnostic GSP observer (2026-10-09) ─────────────────────────────────────────
 * One helper for every NVIDIA PCI function whose guest GSP queues live in guest RAM: vfio-pci
 * (the x-gsp-observer property, wrappers below) and kf3-gpu (qemu/hw/misc/kf3/kf3.c, its own
 * x-gsp-observer property). Same writer, same trigger filter, same QEMU_CLOCK_REALTIME
 * timestamps, same output file format (decode.py). Every call except gsp_observer_is_trigger
 * needs the BQL. */
#define GSP_OBSERVER_MAX_SECONDS 7200u
typedef struct GspObserver GspObserver;
/* Creates `path` exclusively (0600). NULL with errp set on refusal. `seconds` bounds the capture
 * from now (vfio-pci: 300). */
GspObserver *gsp_observer_open(PCIDevice *pdev, const char *path, uint64_t seconds,
                               Error **errp);
/* Stop, drain and free (idempotent with the observer's own exit-notifier drain). NULL is a no-op. */
void gsp_observer_close(GspObserver *s);
/* Whether a BAR0 access is one of the observer's trigger points; no lock needed. */
bool gsp_observer_is_trigger(hwaddr addr, unsigned size, bool write);
/* Call BEFORE forwarding a BAR0 access (a no-op unless it is a trigger point). */
void gsp_observer_mmio(GspObserver *s, hwaddr addr, uint64_t data, unsigned size, bool write);
/* Call BEFORE injecting an interrupt. */
void gsp_observer_irq(GspObserver *s);
void gsp_observer_reset(GspObserver *s);

/* ── vfio-pci integration (integration.patch) ── */
typedef struct VFIOPCIDevice VFIOPCIDevice;
bool vfio_gsp_open(VFIOPCIDevice *vdev, Error **errp);
void vfio_gsp_close(VFIOPCIDevice *vdev);
void vfio_gsp_reset(VFIOPCIDevice *vdev);
void vfio_gsp_irq(VFIOPCIDevice *vdev);
void vfio_gsp_region(VFIODevice *dev, int region, hwaddr addr, uint64_t data,
                     unsigned size, bool write);
#endif
