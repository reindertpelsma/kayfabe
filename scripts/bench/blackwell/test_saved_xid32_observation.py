"""Owner17:00 exact saved-pair observation is separate from the normal guards."""

import copy
import datetime as dt
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

import test_windows_observation as old

SPEC = importlib.util.spec_from_file_location(
    "saved_xid32_observation", Path(__file__).with_name("saved_xid32_observation.py")
)
O = None
if Path(SPEC.origin).exists():
    O = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(O)

MODE = "bw3-saved-xid32-observation-v1"
AUTHORITY = {
    **old.AUTHORITY,
    "mode": MODE,
    "observation_only": True,
    "attestation_allowed": False,
    "exact_channels": ["0x14", "0x1000000b"],
}
PERMIT = {**old.PERMIT, "mode": MODE}


def pair(count=2):
    return [old.xid(0x14 if n % 2 == 0 else 0x1000000B, second=n + 1) for n in range(count)]


class SavedPairObservation(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(O, "saved-pair observation mode missing")

    def judge(self, lines=None, **changes):
        args = dict(phase=old.PHASE, permit=PERMIT, authority=AUTHORITY, now=old.NOW)
        args.update(changes)
        return O.judge_observation(pair() if lines is None else lines, **args)

    def test_saved_pair_is_recorded_without_becoming_pass_or_attestation(self):
        records = self.judge()
        self.assertEqual([r["channel"] for r in records], ["0x14", "0x1000000b"])
        self.assertTrue(all(r["verdict"] == "OBSERVATION" for r in records))
        self.assertTrue(all(r["mode"] == MODE for r in records))
        self.assertTrue(all(r["attestation"] is False and r["pass"] is False for r in records))

    def test_eight_saved_pair_records_are_counted_and_never_silenced(self):
        records = self.judge(pair(8))
        self.assertEqual(len(records), 8)
        self.assertEqual([r["line"] for r in records], pair(8))
        self.assertTrue(all(r["mode"] == MODE and not r["attestation"] for r in records))
        self.assertTrue(all(r["verdict"] == "OBSERVATION" and not r["pass"] for r in records))

    def test_ninth_record_refuses(self):
        with self.assertRaises(ValueError):
            self.judge(pair(9))

    def test_tenth_record_refuses(self):
        with self.assertRaises(ValueError):
            self.judge(pair(10))

    def test_ninth_repeat_of_one_exact_channel_also_refuses(self):
        with self.assertRaises(ValueError):
            self.judge([old.xid(second=n + 1) for n in range(9)])

    def test_full_raw_line_over_240_characters_is_retained(self):
        line = old.xid().replace(" ubuntu ", " " + "h" * 240 + " ")
        self.assertEqual(self.judge([line])[0]["line"], line)

    def test_long_high_channel_raw_record_is_retained_without_truncation(self):
        line = old.xid(0x1000000B).replace(" ubuntu ", " " + "h" * 180 + " ")
        self.assertEqual(self.judge([line])[0]["line"], line)

    def python310_iso(self, value):
        if value.endswith("+0000"):
            raise ValueError("Python3.10 fromisoformat rejects compact offset")
        return old.O.aware(value)

    def test_kernel_record_accepts_compact_offset_with_python310_iso_parser(self):
        with patch.object(O, "aware", side_effect=self.python310_iso):
            self.assertEqual(len(self.judge()), 2)

    def test_kernel_context_accepts_compact_offset_with_python310_iso_parser(self):
        record = self.judge()[0]
        with patch.object(O, "aware", side_effect=self.python310_iso):
            item = O.context(record, [{"line": record["line"]}], record["line"])
        self.assertEqual(item["kernel_context"], [record["line"]])

    def test_original_observer_still_refuses_the_new_high_channel(self):
        with self.assertRaises(ValueError):
            old.O.judge_observation(
                pair(),
                phase=old.PHASE,
                permit=old.PERMIT,
                authority=old.AUTHORITY,
                now=old.NOW,
            )

    def test_original_observer_still_accepts_only_its_original_pair(self):
        records = old.O.judge_observation(
            old.pair(),
            phase=old.PHASE,
            permit=old.PERMIT,
            authority=old.AUTHORITY,
            now=old.NOW,
        )
        self.assertEqual(records[1]["channel"], "0xe00000b")

    def test_context_preserves_raw_xid_and_surrounding_kernel_rows(self):
        records = self.judge()
        rows = [{"line": "2026-10-23T11:59:00.000000+0000 host kernel: before"}]
        rows += [{"line": r["line"]} for r in records]
        rows += [{"line": "2026-10-23T11:59:04.000000+0000 host kernel: NVRM detail"}]
        dmesg = "before\n" + records[0]["line"].split("kernel: ", 1)[1] + "\nafter\n"
        context = O.context(records[0], rows, dmesg)
        self.assertEqual(context["raw_xid"], records[0]["line"])
        self.assertEqual(context["kernel_context"], [r["line"] for r in rows])
        self.assertEqual(context["dmesg_context"], dmesg.splitlines())
        self.assertEqual(context["mode"], MODE)
        self.assertFalse(context["attestation"])

    def test_repeated_dmesg_signature_keeps_all_context_candidates(self):
        record = self.judge()[0]
        raw = record["line"].split("kernel: ", 1)[1]
        dmesg = raw + "\nfirst-detail\n" + "\n".join(["middle"] * 250)
        dmesg += "\n" + raw + "\nsecond-detail\n"
        context = O.context(record, [{"line": record["line"]}], dmesg)
        self.assertIn("first-detail", context["dmesg_context"])
        self.assertIn("second-detail", context["dmesg_context"])


def negative(name, *, line=None, authority=None, permit=None, phase=None):
    def test(self):
        changes = {}
        if authority is not None:
            changes["authority"] = {**copy.deepcopy(AUTHORITY), **authority}
        if permit is not None:
            changes["permit"] = {**PERMIT, **permit}
        if phase is not None:
            changes["phase"] = {**old.PHASE, **phase}
        with self.assertRaises(ValueError):
            self.judge(pair() if line is None else [line], **changes)

    setattr(SavedPairObservation, "test_refuses_" + name, test)


for channel in (0x15, 0x0E00000B, 0x1000000C, 0x13):
    negative(f"channel_{channel:x}", line=old.xid(channel))
for code in (31, 79, 61, 62, 45):
    negative(f"code_{code}", line=old.xid(code=code))
for intr in ("00000000", "00800000"):
    negative("intr_" + intr, line=old.xid(intr=intr))
for pid in (111033, 1):
    negative(f"pid_{pid}", line=old.xid(pid=pid))
negative("nonqemu", line=old.xid().replace("name=qemu-system-x86", "name=another"))
negative("othergpu", line=old.xid().replace("PCI:0000:00:07", "PCI:0000:00:08"))
negative("permit_mode", permit={"mode": old.O.MODE})
negative("authority_mode", authority={"mode": old.O.MODE})
negative("not_observation_only", authority={"observation_only": False})
negative("numeric_observation_only", authority={"observation_only": 1})
negative("attestation_authority", authority={"attestation_allowed": True})
negative("missing_attestation_prohibition", authority={"attestation_allowed": None})
negative("other_channel_authority", authority={"exact_channels": ["0x14", "0xe00000b"]})
negative("another_lease", authority={"lease_id": 12345679})
negative("another_pid_life", permit={"starttime": 191784})
negative("another_arm", permit={"arm": "on_observe_b1"})
negative("expired", authority={"expires_utc": "2026-10-23T12:00:00+00:00"})
negative("before_enable", line=old.xid().replace("11:59:01", "11:58:59"))
negative("second_enable", permit={"enable_count": 2})


if __name__ == "__main__":
    unittest.main()
