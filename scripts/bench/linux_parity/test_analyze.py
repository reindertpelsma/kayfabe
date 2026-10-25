import math
import unittest
from analyze import distribution, parse, paired


class AnalysisTests(unittest.TestCase):
    def test_distribution(self):
        row = distribution([1, 2, 3, 4, 100])
        self.assertEqual(row["median_ms"], 3)
        self.assertAlmostEqual(row["p90_ms"], 61.6)
        self.assertEqual(row["over_100_ms_fraction"], 0)
        self.assertEqual(row["over_2x_median_fraction"], 0.2)

    def test_spike_boundary_is_strict(self):
        self.assertEqual(distribution([50, 50, 100])["over_100_ms_fraction"], 0)

    def test_empty_and_invalid_are_unmeasured(self):
        for values in [[], [0], [-1], [math.nan], [math.inf]]:
            with self.assertRaises(ValueError):
                distribution(values)

    def test_identity_pairs(self):
        row = paired([1] * 6, [1] * 6)
        self.assertEqual(row["ratio_ci95"], [1, 1])
        self.assertEqual(row["ratio_median"], 1)

    def test_throughput_ratio_direction(self):
        row = paired([1] * 6, [2] * 6)
        self.assertEqual(row["ratio_median"], 2)

    def test_no_parity_claim_below_five(self):
        with self.assertRaises(ValueError):
            paired([1] * 4, [1] * 4)

    def test_missing_or_unequal_pairs(self):
        with self.assertRaises(ValueError):
            paired([1] * 5, [1] * 6)

    def test_bad_content_is_rejected(self):
        with self.assertRaises(ValueError):
            parse("LX2_BEGIN cuda-spin\nITER N=16 i=1 launch_ms=2 bad=1\n")

    def test_cpu_solo_not_submit(self):
        text = (
            "LX2_BEGIN cuda-spin\n"
            "ITER N=16 i=0 launch_ms=999 bad=0\n"
            "ITER N=16 i=1 launch_ms=2 submit_ms=0.1 bad=0\n"
        )
        self.assertEqual(parse(text)["cuda-spin-N16"]["cpu_ms"], [2])

    def test_surface_gpu_passes(self):
        text = "SURFACE_FRAME 0,10.0,2.5,1.5,0.25\n"
        data = parse(text)["surface"]
        self.assertEqual(data["render_gpu_ms"], [1.5])
        self.assertEqual(data["sample_gpu_ms"], [0.25])


if __name__ == "__main__":
    unittest.main()
