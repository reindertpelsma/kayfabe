"""A receiver must never publish a missing, failed or reordered chunk set."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('checked_chunks', ROOT / 'scripts/bench/diesweep/checked_chunks.py')


class CheckedChunksTests(unittest.TestCase):
    def setUp(self):
        self.m = importlib.util.module_from_spec(SPEC)
        SPEC.loader.exec_module(self.m)

    def test_uneven_plan_covers_every_byte_once(self):
        plan = self.m.plan(23, 4)
        self.assertEqual([b for start, count in plan for b in range(start, start + count)], list(range(23)))

    def test_small_file_has_no_empty_ranges(self):
        self.assertEqual(self.m.plan(3, 8), [(0, 1), (1, 1), (2, 1)])

    def test_zero_file_is_refused(self):
        with self.assertRaises(ValueError):self.m.plan(0, 8)

    def test_unbounded_parallelism_is_refused(self):
        with self.assertRaises(ValueError):self.m.plan(30, 129)

    def test_stream_stops_at_exact_range_boundary(self):
        import io
        src = io.BytesIO(b'0123456789'); dst = io.BytesIO()
        self.m.stream(src, dst, 3, 4)
        self.assertEqual(dst.getvalue(), b'3456')

    def test_truncated_source_refuses(self):
        import io
        with self.assertRaises(ValueError):self.m.stream(io.BytesIO(b'123'), io.BytesIO(), 2, 4)

    def test_missing_source_suffix_refuses(self):
        import io
        with self.assertRaises(ValueError):self.m.stream(io.BytesIO(b'123'), io.BytesIO(), 8, 1)

    def test_failed_first_chunk_does_not_publish(self):
        published=[]
        def send(index, start, size):
            if index == 0:raise RuntimeError('transfer refused')
        with self.assertRaises(RuntimeError):self.m.dispatch([(0, 4), (4, 4)], send, lambda:published.append(True))
        self.assertEqual(published, [])

    def test_failed_last_chunk_does_not_publish(self):
        published=[]
        def send(index, start, size):
            if index == 1:raise RuntimeError('transfer refused')
        with self.assertRaises(RuntimeError):self.m.dispatch([(0, 4), (4, 4)], send, lambda:published.append(True))
        self.assertEqual(published, [])

    def test_every_ack_precedes_publish(self):
        completed=[]
        def send(index, start, size):completed.append(index)
        def publish():self.assertEqual(sorted(completed), [0, 1, 2]);return 'published'
        self.assertEqual(self.m.dispatch([(0, 3), (3, 3), (6, 2)], send, publish), 'published')


if __name__ == '__main__':
    unittest.main()
