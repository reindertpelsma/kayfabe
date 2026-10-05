/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef VFIO_GSP_OBSERVER_H
#define VFIO_GSP_OBSERVER_H
#include "hw/vfio/vfio-device.h"
typedef struct VFIOGspObserver VFIOGspObserver;
typedef struct VFIOPCIDevice VFIOPCIDevice;
bool vfio_gsp_open(VFIOPCIDevice *vdev, Error **errp);
void vfio_gsp_close(VFIOPCIDevice *vdev);
void vfio_gsp_reset(VFIOPCIDevice *vdev);
void vfio_gsp_irq(VFIOPCIDevice *vdev);
void vfio_gsp_region(VFIODevice *dev, int region, hwaddr addr, uint64_t data,
                     unsigned size, bool write);
#endif
