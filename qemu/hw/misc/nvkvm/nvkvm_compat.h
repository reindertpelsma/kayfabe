/*
 * nvkvm_compat.h — every place two hypervisor releases spell the same thing differently.
 *
 * ★★ THE POINT OF THIS FILE IS THAT IT IS SHORT.  `four_axes_of_variation.md` names the
 * hypervisor as one of five axes that must never become a bolt-on.  The test of that is not
 * "does it build on two releases" — it is "how much of the device had to know".  The answer
 * below is: the include paths, one function-pointer signature, one array terminator, and one
 * facility that did not exist yet.  Nothing in nvkvm.c branches on a version; nothing in Rust
 * does either.
 *
 * VERIFIED AGAINST, by building: 9.2.0 and 10.2.4.  The range in between is INFERRED from the
 * release the header rename landed in and has NOT been built.  Said here rather than
 * discovered by a user.
 */

#ifndef NVKVM_COMPAT_H
#define NVKVM_COMPAT_H

#include "qemu/osdep.h"   /* pulls in config-host.h, hence QEMU_VERSION_MAJOR/MINOR */

#if !defined(QEMU_VERSION_MAJOR) || !defined(QEMU_VERSION_MINOR)
#error "nvkvm: QEMU_VERSION_MAJOR/MINOR are not defined; this device must be built inside a QEMU source tree (there is no supported out-of-tree device mechanism)."
#endif

#define NVKVM_QEMU_AT_LEAST(maj, min) \
    (QEMU_VERSION_MAJOR > (maj) || \
     (QEMU_VERSION_MAJOR == (maj) && QEMU_VERSION_MINOR >= (min)))

/*
 * ★★ THE COMPILE-TIME FLOOR — and it is 9.2, not 10.2, deliberately.
 *
 * `l2_qemu_adapter.md` §3.5 asks for two floor assertions at the same number, on the argument
 * that the compile-time one is a claim about the HEADERS and the realize-time one a claim
 * about the BINARY.  §2.1 of the same document proves those cannot differ for this device:
 * there is no supported out-of-tree mechanism, so the shim is compiled INSIDE the binary it
 * runs in and the two claims have one source.
 *
 * What survives is a better distinction: the two floors are about different SUBJECTS.
 *
 *   - this one is about SYMBOLS.  Every hypervisor function this shim names was verified
 *     present at 9.2, so 9.2 is where the build can honestly start.
 *   - the realize-time floor is `kayfabe_vmm_qemu::VERSION_FLOOR`, still 10.2, and it is
 *     about SEMANTICS: the global-lock opt-out below, without which this device runs its
 *     handlers under the hypervisor's global lock.
 *
 * Setting this to 10.2 as well would have made the device unbuildable on the only tree the C
 * artifact ever ran on, and would have hidden the more useful outcome: on a 9.2 build the
 * device compiles, registers, realizes its C half, and is refused BY NAME at the Rust half.
 */
#if !NVKVM_QEMU_AT_LEAST(9, 2)
#error "nvkvm: this device requires QEMU 9.2 or newer.  Below that, symbols it names are absent and the failure would be a link error with no explanation."
#endif

/*
 * The headers moved out of exec/ and sysemu/ into system/ during the 10.0 cycle.  Detected
 * rather than versioned, so a backport or a distribution patch cannot get this wrong.
 */
#if defined(__has_include)
#  if __has_include("system/memory.h")
#    define NVKVM_SYSTEM_HEADERS 1
#  endif
#endif

#ifdef NVKVM_SYSTEM_HEADERS
#  include "system/memory.h"
#  include "system/kvm.h"
#else
#  include "exec/memory.h"
#  include "sysemu/kvm.h"
#endif

/*
 * `class_init`'s data pointer gained a `const` in the same cycle as the header rename.  It
 * cannot be feature-detected — a function-pointer signature has no preprocessor presence — so
 * this one rides on the rename, which is the closest available proxy.
 */
#ifdef NVKVM_SYSTEM_HEADERS
#  define NVKVM_CLASS_DATA const void
#else
#  define NVKVM_CLASS_DATA void
#endif

/*
 * `device_class_set_props` became a macro over ARRAY_SIZE, which means the property array
 * must NOT carry a terminator any more; before that it had to.  Feature-detected on the
 * terminator macro's own existence, which is exact.
 */
#ifdef DEFINE_PROP_END_OF_LIST
#  define NVKVM_PROP_TERMINATOR DEFINE_PROP_END_OF_LIST(),
#else
#  define NVKVM_PROP_TERMINATOR
#endif

/*
 * ★★★ The global-lock opt-out.  One function, and it is the whole reason the RUNTIME floor is
 * 10.2 while this file's floor is 9.2.
 *
 * It is honoured only on the accelerator's dispatch path.  A build without it runs every
 * handler of this device under the hypervisor's global lock — which is not a slow mode to be
 * accepted quietly, it is the amplification the whole threading design exists to avoid.  The
 * device does not refuse to BUILD without it, because a build that refuses cannot tell you
 * anything; it realizes, and the Rust half refuses by name, which can.
 */
#if defined(NVKVM_SYSTEM_HEADERS) && NVKVM_QEMU_AT_LEAST(10, 2)
#  define NVKVM_HAVE_LOCKLESS_IO 1
#else
#  define NVKVM_HAVE_LOCKLESS_IO 0
#endif

#endif /* NVKVM_COMPAT_H */
