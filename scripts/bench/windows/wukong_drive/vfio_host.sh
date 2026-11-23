#!/bin/bash
# usage: vfio_host.sh bind|restore   (run as root on the PC, with the GPU lock held by the caller)
set -u
G=0000:01:00.0; A=0000:01:00.1
case $1 in
bind)
  systemctl stop gdm; sleep 3
  for s in nvidia-persistenced; do systemctl stop $s 2>/dev/null; done
  fuser -v /dev/nvidia* 2>&1 | head -5
  rmmod nvidia_uvm nvidia_drm nvidia_modeset nvidia 2>&1 | head -5
  lsmod | grep -c '^nvidia ' | sed 's/^/nvidia_still_loaded=/'
  modprobe vfio-pci
  for d in $G $A; do
    [ -e /sys/bus/pci/devices/$d/driver ] && echo $d > /sys/bus/pci/devices/$d/driver/unbind
    echo vfio-pci > /sys/bus/pci/devices/$d/driver_override
    echo $d > /sys/bus/pci/drivers_probe
  done
  for d in $G $A; do echo $d $(basename $(readlink /sys/bus/pci/devices/$d/driver)); done ;;
restore)
  for d in $G $A; do
    [ -e /sys/bus/pci/devices/$d/driver ] && echo $d > /sys/bus/pci/devices/$d/driver/unbind
    echo > /sys/bus/pci/devices/$d/driver_override
  done
  modprobe nvidia; modprobe nvidia_modeset; modprobe nvidia_drm; modprobe nvidia_uvm
  echo $G > /sys/bus/pci/drivers_probe; echo $A > /sys/bus/pci/drivers_probe
  sleep 2; systemctl start nvidia-persistenced 2>/dev/null; systemctl start gdm
  sleep 8; nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv,noheader
  for d in $G $A; do echo $d $(basename $(readlink /sys/bus/pci/devices/$d/driver)); done
  systemctl is-active gdm ;;
esac
