import importlib.util
from pathlib import Path
import struct
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import annotate_caps as annotation
import compare


class AnnotationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.table = annotation.catalogue(annotation.DEFAULT_TABLE, 'C773')

    def page(self, name, words):
        data = bytearray(4096)
        for offset, value in words.items():
            struct.pack_into('<I', data, offset, value)
        path = self.root / name
        path.write_bytes(data)
        return name, path

    def test_c773_ilut_bit_uses_generated_field(self):
        a = self.page('native', {0x784: 1 << 14})
        b = self.page('virtual', {})
        report = annotation.annotate(self.table, [a, b])
        word = report['words'][0x784 // 4]
        field = next(f for r in word['registers'] for f in r['fields'] if f['name'].endswith('_ILUT_SFCLOAD'))
        self.assertEqual(field['values'], dict(native=1, virtual=0))
        self.assertTrue(field['different'])
        self.assertIn('NVC773_PRECOMP_WIN_PIPE_HDR_CAPB_ILUT_SFCLOAD_TRUE', field['value_names']['native'])
        self.assertIn('_ILUT_SFCLOAD', annotation.markdown(report))

    def test_all_existing_caps_classes_source_derived(self):
        for name in ('C373', 'C573', 'C673', 'C773', 'CA73'):
            with self.subTest(name=name):
                table = annotation.catalogue(annotation.DEFAULT_TABLE, name)
                self.assertTrue(table['registers'])
                self.assertTrue(all(0 <= offset <= 4092 and offset % 4 == 0 for offset in table['registers']))
                self.assertEqual(table['version'], '580.65.06')

    def test_unknown_class_and_method_class_refused(self):
        for cls in ('FFFF', 'C77D', '../C773', 'C773X'):
            with self.subTest(cls=cls), self.assertRaises(compare.InvalidEvidence):
                annotation.catalogue(annotation.DEFAULT_TABLE, cls)

    def test_unknown_bits_retained_with_all_per_page_values(self):
        report = annotation.annotate(self.table, [self.page('a', {0xffc: 0x12345678}), self.page('b', {0xffc: 9})])
        word = report['words'][-1]
        self.assertEqual(word['registers'], [])
        self.assertEqual(word['uncovered_values'], word['values'])
        self.assertEqual(word['values']['a'], '0x12345678')
        self.assertIn('uncovered bits', annotation.markdown(report))

    def test_arrays_do_not_name_out_of_source_bounds(self):
        report = annotation.annotate(self.table, [self.page('a', {})])
        last = report['words'][(0x784 + 31 * 32) // 4]
        self.assertEqual(last['registers'][0]['index'], 31)
        next_word = report['words'][(0x784 + 32 * 32) // 4]
        self.assertEqual(next_word['registers'], [])

    def test_source_bounds_fail_closed(self):
        for tail in ('A\tNVC773_X\t4096\t32\nV\tNVC773_X__SIZE_1\t1\nF\tNVC773_X_F\t1\t0\n',
                     'A\tNVC773_X\t0\t4\nF\tNVC773_X_F\t1\t0\n',
                     'V\tNVC773_X\t0\nF\tNVC773_X_F\t32\t0\n',
                     'V\tNVC773_X\t0\nV\tNVC773_X\t4\nF\tNVC773_X_F\t1\t0\n'):
            table = self.root / 'bad.tsv'
            table.write_text('VERSION\tfixture\n' + tail)
            with self.assertRaises(compare.InvalidEvidence):
                annotation.catalogue(table, 'C773')

    def test_page_length_and_duplicate_labels_refused(self):
        item = self.page('a', {})
        with self.assertRaises(compare.InvalidEvidence):
            annotation.annotate(self.table, [item, item])
        item[1].write_bytes(bytes(4095))
        with self.assertRaises(compare.InvalidEvidence):
            annotation.annotate(self.table, [item])

    def test_single_page_is_annotation_not_repeated_stability_claim(self):
        report = annotation.annotate(self.table, [self.page('a', {0x784: 1 << 14})])
        self.assertEqual(len(report['words']), 1024)
        self.assertFalse(any(word['different'] for word in report['words']))
        self.assertNotIn('stable', report['words'][0])


if __name__ == '__main__':
    unittest.main()
