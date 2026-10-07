# kf3-gpu — the v3 device. The Rust archive (crates/kf-qemu, staticlib) is dropped in beside these
# sources by scripts/bench/build_kf3.sh. Two hunks in the QEMU tree, one line each: subdir('kf3') in
# hw/misc/meson.build and a four-line Kconfig stanza.
kf3_archive = meson.current_source_dir() / 'libkf_qemu.a'
kf3_dep = declare_dependency(
  link_args: ['-Wl,--start-group', kf3_archive, '-Wl,--end-group', '-lpthread', '-ldl', '-lm'],
)
system_ss.add(when: 'CONFIG_KF3', if_true: [files('kf3.c'), kf3_dep])
