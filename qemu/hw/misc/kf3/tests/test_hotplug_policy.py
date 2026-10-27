"""SA-01: compile the real class initializer; model only the cited i440fx guard.

FALSIFIER: initialization leaves the class hotpluggable, or the i440fx guard
permits the initialized conventional PCI function to be ejected.
"""
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[5]
SOURCE = REPO / 'qemu/hw/misc/kf3/kf3.c'

def initializer():
    source = SOURCE.read_text()
    start = source.index('static void kf3_class_init(')
    body = source.index('{', start)
    depth = 1
    end = body + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]

STUBS = r'''
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
typedef struct { bool hotpluggable; unsigned long categories[1]; const char *desc; } DeviceClass;
typedef struct {
    void (*realize)(void); void (*exit)(void); unsigned vendor_id,device_id,class_id;
} PCIDeviceClass;
typedef struct { struct { void (*hold)(void); void (*exit)(void); } phases; } ResettableClass;
typedef struct { DeviceClass dc; PCIDeviceClass pci; ResettableClass reset; } ObjectClass;
#define DEVICE_CLASS(k) (&(k)->dc)
#define PCI_DEVICE_CLASS(k) (&(k)->pci)
#define RESETTABLE_CLASS(k) (&(k)->reset)
#define PCI_CLASS_DISPLAY_VGA 0x300
#define DEVICE_CATEGORY_DISPLAY 0
static void kf3_dev_realize(void) {}
static void kf3_dev_exit(void) {}
static void kf3_reset_hold(void) {}
static void kf3_reset_exit(void) {}
static const int kf3_properties[] = {0};
static void device_class_set_props(DeviceClass *d, const int *p) { (void)d; (void)p; }
static void set_bit(int n, unsigned long *p) { *p |= 1UL << n; }
'''
MAIN = r'''
int main(int argc, char **argv) {
    ObjectClass k = {0};
    k.dc.hotpluggable = true; /* QEMU's inherited default */
    kf3_class_init(&k, NULL);
    if (argc != 2) return 2;
    if (!strcmp(argv[1], "class")) return k.dc.hotpluggable ? 10 : 0;
    /* QEMU v10.2.4 hw/acpi/pcihp.c acpi_pcihp_pc_no_hotplug, applied
       to a conventional non-bridge/non-VF. This is a policy model, not qtest. */
    const bool no_hotplug = !k.dc.hotpluggable;
    return no_hotplug ? 0 : 20;
}
'''

class HotplugPolicy(unittest.TestCase):
    def probe(self, arm):
        with tempfile.TemporaryDirectory(prefix='kf3-hotplug-audit-') as d:
            path = Path(d)
            source = path / 'probe.c'
            binary = path / 'probe'
            source.write_text(STUBS + initializer() + MAIN)
            subprocess.run(['nice','-n','19','cc','-std=c11','-Wall','-Wextra','-Werror',
                            '-Wno-unused-parameter',str(source),'-o',str(binary)],
                           check=True,capture_output=True)
            result = subprocess.run([str(binary),arm],capture_output=True)
            self.assertEqual(result.returncode,0,
                             f'{arm}: unsafe hotplug policy, exit {result.returncode}')

    def test_actual_class_initializer_disables_hotplug(self):
        self.probe('class')

    def test_i440fx_eject_guard_retains_the_initialized_function(self):
        self.probe('eject')

    def test_registered_gpu_type_uses_the_checked_class_initializer(self):
        source=SOURCE.read_text()
        info=source[source.index('static const TypeInfo kf3_type_info'):]
        self.assertRegex(info,r'\.class_init\s*=\s*kf3_class_init\b')

if __name__ == '__main__':
    unittest.main()
