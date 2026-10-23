"""Channel evidence comes from public RPC fields and existing birth diagnostics."""

import datetime as dt
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
try:
    import channel_projection as C
except ImportError:
    C = None


class Channels(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(C, "public channel projection missing")

    def project(self, text, channels=("0x14", "0xe00000b")):
        offset, rows = 0, []
        for line in text.splitlines(keepends=True):
            offset += len(line.encode())
            if "rpc-trace" in line:
                rows.append({"ordinal": len(rows), "file_offset_after": offset})
        return C.project(text.encode(), rows, channels)

    def test_birth_joins_exact_guest_allocation_and_engine_children(self):
        text = (
            "kf-rm: rpc-trace fn=103 RmAlloc seq=0 class=0xc96f "
            "client=0xa parent=0x1 handle=0x10 result=0x0\n"
            "kf-rm: chan 0xa:0x10 BORN Translated: token 0x4 -> host 0x40000014 "
            "in space engine=0x2 ring_va=0x1000\n"
            "kf-rm: rpc-trace fn=103 RmAlloc seq=0 class=0xcb97 "
            "client=0xa parent=0x10 handle=0x11 result=0x0\n"
        )
        row = self.project(text)["births"][0]
        self.assertEqual(row["allocation_ordinal"], 0)
        self.assertEqual(row["channel_class"], "0xc96f")
        self.assertEqual(row["declared_engine"], "0x2")
        self.assertEqual(row["engine_object_classes"], [{"class": "0xcb97", "ordinal": 1}])
        self.assertEqual(row["host_token"], "0x40000014")
        self.assertEqual(row["candidate_xid_channel"], "0x14")

    def test_high_pair_candidate_preserves_guest_identity_and_opaque_token(self):
        text = (
            "chan 0xb:0x22 BORN Passthrough: token 0x7001 -> host 0x400e000b "
            "in space engine=0x1\n"
        )
        result = self.project(text)
        row = result["births"][0]
        self.assertEqual(row["client"], "0xb")
        self.assertEqual(row["handle"], "0x22")
        self.assertEqual(row["candidate_xid_channel"], "0xe00000b")
        self.assertEqual(row["host_token"], "0x400e000b")
        self.assertEqual(result["target_candidates"]["0xe00000b"], [0])
        self.assertEqual(result["join_classification"], "INFERRED")

    def test_missing_target_stays_unresolved(self):
        self.assertEqual(self.project("ordinary log\n")["target_candidates"]["0x14"], [])

    def test_reused_native_id_keeps_all_candidates(self):
        text = "".join(
            f"chan 0xa:{handle:#x} BORN Translated: token 0x1 -> host 0x40000014 "
            "in space engine=0x2\n"
            for handle in [0x10, 0x20]
        )
        self.assertEqual(self.project(text)["target_candidates"]["0x14"], [0, 1])

    def test_private_reply_bytes_are_never_projected(self):
        text = (
            "kf-gsp: reply-body body=PRIVATE_BODY_DO_NOT_EXPORT "
            "chan 0xa:0x10 BORN Translated: token 0x1 -> host 0x40000014 "
            "in space engine=0x2\n"
        )
        result = self.project(text)
        self.assertEqual(result["births"], [])
        self.assertNotIn("PRIVATE_BODY_DO_NOT_EXPORT", json.dumps(result))

    def test_failed_allocations_do_not_establish_a_live_channel_class(self):
        text = (
            "kf-rm: rpc-trace fn=103 RmAlloc seq=0 class=0xc96f "
            "client=0xa parent=0x1 handle=0x10 result=0x1f\n"
            "chan 0xa:0x10 BORN Translated: token 0x1 -> host 0x40000014 "
            "in space engine=0x2\n"
        )
        self.assertIsNone(self.project(text)["births"][0]["channel_class"])

    def test_birth_before_its_allocation_trace_can_still_join_exact_handles(self):
        text = (
            "chan 0xa:0x10 BORN Translated: token 0x1 -> host 0x40000014 "
            "in space engine=0x2\n"
            "kf-rm: rpc-trace fn=103 RmAlloc seq=0 class=0xc96f "
            "client=0xa parent=0x1 handle=0x10 result=0x0\n"
        )
        self.assertEqual(self.project(text)["births"][0]["allocation_ordinal"], 0)

    def test_40_second_window_includes_both_boundaries(self):
        at = dt.datetime(2026, 10, 23, 12, 0, tzinfo=dt.timezone.utc)
        rows = [
            {"ordinal": i, "observed_utc": (at + dt.timedelta(seconds=delta)).isoformat()}
            for i, delta in enumerate([-41, -40, -1, 0, 1])
        ]
        self.assertEqual([r["ordinal"] for r in C.rpc_window(rows, at.isoformat())], [1, 2, 3])

    def test_window_excludes_a_row_older_than_40_seconds(self):
        rows = [{"ordinal": 1, "observed_utc": "2026-10-23T11:59:19+00:00"}]
        self.assertEqual(C.rpc_window(rows, "2026-10-23T12:00:00+00:00"), [])

    def test_window_excludes_a_row_after_the_event(self):
        rows = [{"ordinal": 1, "observed_utc": "2026-10-23T12:00:01+00:00"}]
        self.assertEqual(C.rpc_window(rows, "2026-10-23T12:00:00+00:00"), [])

    def test_foreign_client_does_not_supply_the_allocation_ordinal(self):
        text = (
            "kf-rm: rpc-trace fn=103 RmAlloc seq=0 class=0xc96f "
            "client=0xa parent=0x1 handle=0x10 result=0x0\n"
            "chan 0xb:0x10 BORN Translated: token 0x1 -> host 0x40000014 "
            "in space engine=0x2\n"
        )
        self.assertIsNone(self.project(text)["births"][0]["allocation_ordinal"])

    def test_other_handle_does_not_supply_the_allocation_ordinal(self):
        text = (
            "kf-rm: rpc-trace fn=103 RmAlloc seq=0 class=0xc96f "
            "client=0xa parent=0x1 handle=0x10 result=0x0\n"
            "chan 0xa:0x20 BORN Translated: token 0x1 -> host 0x40000014 "
            "in space engine=0x2\n"
        )
        self.assertIsNone(self.project(text)["births"][0]["allocation_ordinal"])

    def test_unmarked_body_bytes_are_also_excluded(self):
        text = (
            "body=PRIVATE_BODY_DO_NOT_EXPORT chan 0xa:0x10 BORN Translated: "
            "token 0x1 -> host 0x40000014 in space engine=0x2\n"
        )
        self.assertEqual(self.project(text)["births"], [])

    def test_ambiguous_join_is_not_promoted_to_measurement(self):
        self.assertEqual(self.project("ordinary log\n")["join_classification"], "INFERRED")

    def test_public_runlist_field_is_retained_in_the_offline_candidate(self):
        text = (
            "chan 0xa:0x10 BORN Translated: token 0x1 -> host 0x40010005 " "in space engine=0x2\n"
        )
        result = self.project(text, channels=("0x1000005",))
        self.assertEqual(result["births"][0]["candidate_xid_channel"], "0x1000005")
        self.assertEqual(result["target_candidates"]["0x1000005"], [0])
        self.assertEqual(result["join_classification"], "INFERRED")


if __name__ == "__main__":
    unittest.main()
