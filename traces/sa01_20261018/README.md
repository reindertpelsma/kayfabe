# SA-01: disable the conventional PCI device's hotplug policy

INFERRED hypothesis before the probe: setting hotpluggable=false in the actual
class initializer closes the production i440fx guest-eject path. FALSIFIERS:
initialization retains the inherited true value, the registered type bypasses
that initializer, or QEMU's i440fx eject path bypasses its class-policy guard.

CODE READ primary source, QEMU v10.2.4 (the version named by the repository's
bench/build scripts): DeviceClass defaults hotpluggable=true in
[hw/core/qdev.c](https://github.com/qemu/qemu/blob/v10.2.4/hw/core/qdev.c#L715-L729).
The [i440fx ACPI eject path](https://github.com/qemu/qemu/blob/v10.2.4/hw/acpi/pcihp.c#L154-L211)
checks the class policy before calling unplug and unparent. The same guard masks
the removable slot in that file's update_hotplug_bus. The production launcher
scripts/bench/windows/windows_broker.sh uses machine pc and bus=pci.0; the device
TypeInfo declares INTERFACE_CONVENTIONAL_PCI_DEVICE. These establish the requested
source condition; no live ejection or hardware confirmation is claimed.

MEASURED tests first: aeb368486 compiles the actual kf3_class_init body using small
DeviceClass/PCI/reset interface stubs, cc C11/Wall/Wextra/Werror (unused callback
parameters excluded), nice19. One registration control passes; two policy
assertions fail with runtime exits10/20 on unchanged master. These are assertion
reds, not missing fields, compiler errors or missing targets.

CODE READ fix: one assignment, dc->hotpluggable=false. Startup cold realization
is preserved by the reviewed generic QEMU policy; hot-add and guest ACPI eject
are refused. No teardown implementation, Rust source, guest register path, flag,
timer or unsafe-file rule changes.

MEASURED after:3/3 probes pass. Removing the assignment kills both policy tests;
restored3/3 pass. The eject test models the cited guard for a non-bridge/non-VF;
it is not full QEMU or qtest. Rust Clippy all targets on kf-gsp/rm/qemu matches
unchanged-master source fingerprints257 ->257, added0; it does not validate C.
No assertions, deny sets or lint baselines were loosened. No Cargo build beyond
the required scoped lint comparison was needed for the one-line C change.

INFERRED later validation: cold boot and ACPI eject attempt on production pc,
QMP behavior, q35/native PCIe power-off, failed-realize cleanup and ASan teardown.
The existing exit path still does not join/delete all users of device state;
this guard closes the requested i440fx entry point, not every teardown hazard.
No VM/GPU/KVM was used. C probe temporaries are removed by their context manager.
