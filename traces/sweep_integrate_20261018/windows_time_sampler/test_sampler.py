import hashlib
import json
import unittest
import sampler

class SnapshotTests(unittest.TestCase):
    def line(self, fn=76, seq=7, key="0x73010c", result="Ok(())"):
        return f"kf-rm: rpc-trace fn={fn} RmControl seq={seq} cmd={key} result={result}\n".encode()
    def test_ordinal_counts_only_complete_rpc_lines(self):
        r = sampler.snapshot(b"noise\n" + self.line() + self.line(seq=8), "now")
        self.assertEqual(r["commands"], 2)
        self.assertEqual(r["last"]["ordinal"], 1)
    def test_tail_without_newline_is_not_complete(self):
        self.assertEqual(sampler.snapshot(self.line().rstrip(b"\n"), "now")["commands"], 0)
    def test_partial_tail_cannot_claim_unload(self):
        r = sampler.snapshot(self.line() + self.line(fn=47, seq=8).rstrip(b"\n"), "now")
        self.assertEqual(r["unload_ordinals"], [])
        self.assertEqual(r["commands"], 1)
    def test_non_unload_is_not_unload(self):
        self.assertEqual(sampler.snapshot(self.line(fn=76), "now")["unload_ordinals"], [])
    def test_unload_ordinal_comes_from_actual_fn47(self):
        self.assertEqual(sampler.snapshot(self.line() + self.line(fn=47, seq=8), "now")["unload_ordinals"], [1])
    def test_fn48_is_not_unload(self):
        self.assertEqual(sampler.snapshot(self.line(fn=48), "now")["unload_ordinals"], [])
    def test_timestamp_and_hash_retained(self):
        b = self.line()
        r = sampler.snapshot(b, "2026-10-20T01:00:00Z")
        self.assertEqual(r["sample_utc"], "2026-10-20T01:00:00Z")
        self.assertEqual(r["source_sha256"], hashlib.sha256(b).hexdigest())
    def test_private_body_bytes_never_exported(self):
        r = sampler.snapshot(self.line() + b"kf-gsp: reply-body body=0123456789abcdefPRIVATE\n", "now")
        self.assertNotIn("PRIVATE", json.dumps(r))
        self.assertNotIn("0123456789abcdef", json.dumps(r))
        self.assertEqual(r["commands"], 1)
    def test_input_quota_refuses_before_parse(self):
        with self.assertRaises(ValueError):
            sampler.snapshot(b"x" * 4097, "now", max_bytes=4096)
    def test_metadata_quota_refuses(self):
        with self.assertRaises(ValueError):
            sampler.snapshot(self.line() * 3, "now", max_commands=2)

if __name__ == "__main__":
    unittest.main()
