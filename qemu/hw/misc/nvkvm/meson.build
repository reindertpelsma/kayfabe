# The overlay's own build file.  Everything about this device that a QEMU tree needs to know
# is here, so the two hunks in the tree itself stay one line each:
#
#   hw/misc/meson.build   +  subdir('nvkvm')
#   hw/misc/Kconfig       +  a four-line stanza
#
# ★ The Rust archive is dropped in beside these sources by scripts/build_qom_shim.sh, rather
# than found through a meson option — adding an option would be a THIRD hunk, in
# meson_options.txt, and the whole argument for an additive overlay is that its conflict
# surface is as small as it can be made.
nvkvm_archive = meson.current_source_dir() / 'libkayfabe_qemu_raw.a'

nvkvm_dep = declare_dependency(
  link_args: ['-Wl,--start-group', nvkvm_archive, '-Wl,--end-group',
              '-lpthread', '-ldl', '-lm'],
)

system_ss.add(when: 'CONFIG_NVKVM', if_true: [files('nvkvm.c'), nvkvm_dep])
