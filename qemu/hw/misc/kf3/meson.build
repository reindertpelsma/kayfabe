# kf3-gpu — the v3 device. The Rust archive (crates/kf-qemu, staticlib) is dropped in beside these
# sources by scripts/bench/build_kf3.sh. Two hunks in the QEMU tree, one line each: subdir('kf3') in
# hw/misc/meson.build and a four-line Kconfig stanza.
kf3_archive = meson.current_source_dir() / 'libkf_qemu.a'
# ★ ABI 24, DIAGNOSTIC (default off): kf3's x-gsp-observer links the SHARED GSP observer
# (tools/vfio-gsp-observer, installed into hw/vfio by its apply.py) only when this tree has the
# device-agnostic version and builds vfio-pci; otherwise the property is refused by name at realize.
kf3_c_args = []
kf3_fs = import('fs')
kf3_obs_h = meson.project_source_root() / 'hw/vfio/gsp-observer.h'
if config_all_devices.has_key('CONFIG_VFIO_PCI') and kf3_fs.is_file(kf3_obs_h)
  if kf3_fs.read(kf3_obs_h).contains('gsp_observer_open')
    kf3_c_args += ['-DKF3_GSP_OBSERVER=1']
  endif
endif
kf3_dep = declare_dependency(
  compile_args: kf3_c_args,
  link_args: ['-Wl,--start-group', kf3_archive, '-Wl,--end-group', '-lpthread', '-ldl', '-lm'],
)
system_ss.add(when: 'CONFIG_KF3', if_true: [files('kf3.c'), kf3_dep])
