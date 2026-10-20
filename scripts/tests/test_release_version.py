#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]

class ReleaseVersion(unittest.TestCase):
    def test_all_three_build_parts_use_the_canonical_version(self):
        self.assertIn('kf3_version.h',(ROOT/'qemu/hw/misc/kf3/kf3.c').read_text())
        self.assertIn('KAYFABE_VERSION',(ROOT/'broker/Makefile').read_text())
        self.assertIn('include_str!("../../../VERSION")',(ROOT/'crates/kf-qemu/src/device.rs').read_text())

    def test_qemu_description_and_broker_print_the_release_version(self):
        c=(ROOT/'qemu/hw/misc/kf3/kf3.c').read_text()
        self.assertTrue(any('dc->desc' in line and 'KAYFABE_VERSION' in line for line in c.splitlines()))
        broker=(ROOT/'broker/nvkvm_broker.c').read_text()
        main=broker[broker.index('int main('):]
        self.assertLess(main.index('"--version"'),main.index('struct nb_config cfg'))

    def test_publication_never_replaces_an_existing_revision(self):
        spec=importlib.util.spec_from_file_location('publish',ROOT/'scripts/release/publish_binary.py')
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as raw:
            root=Path(raw);source=root/'source';destination=root/'binary'
            source.write_bytes(b'new');destination.write_bytes(b'old')
            with self.assertRaises(FileExistsError):module.publish(source,destination)
            self.assertEqual(destination.read_bytes(),b'old')
            self.assertEqual(sorted(path.name for path in root.iterdir()),['binary','source'])

    def test_publication_is_an_exact_copy_and_second_writer_is_refused(self):
        spec=importlib.util.spec_from_file_location('publish',ROOT/'scripts/release/publish_binary.py')
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as raw:
            root=Path(raw);source=root/'source';destination=root/'binary';source.write_bytes(b'first')
            module.publish(source,destination);source.write_bytes(b'second')
            with self.assertRaises(FileExistsError):module.publish(source,destination)
            self.assertEqual(destination.read_bytes(),b'first')

if __name__=='__main__':unittest.main()
