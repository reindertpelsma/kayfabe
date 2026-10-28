"""Durable create intent, exact ownership, one issue and lost-receipt recovery."""

import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

try:
    import create_receipt_guard as G
except ImportError:
    G = None

NOW = dt.datetime(2026, 10, 28, 18, 0, tzinfo=dt.timezone.utc)
SOURCE = "7dcedefea093de078188efd5f712561bde746362"
SECRET = "FAKE_CREATE_SECRET_MUST_NEVER_PERSIST"


class GuardFixture(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(G, "create-receipt guard missing")
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.offer = dict(id=900, machine_id=21018, host_id=120071)
        self.plan = dict(
            armed_utc=NOW.isoformat(),
            deadline_utc=(NOW + dt.timedelta(hours=2)).isoformat(),
            seconds=7200,
            rate_bound=0.34,
            quoted_rate=0.24,
            reserve_usd=8,
            reservation_usd=1.23,
            stop_usd=1.5,
            capture_lead_seconds=420,
        )
        self.ref = G.prepare(self.root, self.offer, self.plan, 71, self.snapshot(), SOURCE)
        self.intent = G.load(self.ref)
        self.events = []

    def snapshot(self, rows=None, **changes):
        value = dict(owner_id=71, complete=True, rows=[] if rows is None else rows)
        return dict(value, **changes)

    def row(self, **changes):
        value = dict(
            id=42, label=self.intent["label"], machine_id=21018, host_id=120071, dph_total=0.24
        )
        return dict(value, **changes)

    def issue(self, mode="empty"):
        def invoke(label):
            self.events.append(("create", label))
            if mode == "timeout":
                raise subprocess.TimeoutExpired("fake-create", 45, output=SECRET.encode())
            if mode == "nonzero":
                return subprocess.CompletedProcess([], 1, SECRET.encode(), SECRET.encode())
            if mode == "success":
                data = json.dumps(dict(success=True, new_contract=42, instance_api_key=SECRET))
                return subprocess.CompletedProcess([], 0, data.encode(), SECRET.encode())
            data = b"" if mode == "empty" else SECRET.encode()
            return subprocess.CompletedProcess([], 0, data, SECRET.encode())

        return G.issue(
            self.root,
            self.ref,
            {"state": "ARMED", "intent_sha256": self.ref.sha256},
            invoke,
            now=lambda: NOW,
        )

    def run_watch(self, *, mode="empty", rows=None, inventory_error=False, dead=True):
        state = [self.snapshot([self.row()] if rows is None else rows)]
        clock = [NOW]

        def ready(_):
            self.events.append(("armed", G.load(self.ref)["label"]))
            try:
                self.issue(mode)
            except G.Unresolved:
                pass

        def inventory():
            if inventory_error:
                raise OSError(SECRET)
            return state[0]

        def pause(seconds):
            clock[0] += dt.timedelta(seconds=seconds)

        def publish(value):
            self.events.append(("state", value))
            if value["state"] == "UNRESOLVED" and clock[0] >= NOW + dt.timedelta(hours=2):
                raise StopFake(value)

        def destroy(iid):
            self.events.append(("destroy", iid))
            state[0] = self.snapshot([])

        def capture(iid):
            self.events.append(("capture", iid))
            return dict(id=iid, capture_verified=True)

        return G.watch(
            self.root,
            self.ref,
            inventory=inventory,
            credit=lambda: 40,
            now=lambda: clock[0],
            pause=pause,
            parent_alive=lambda _: not dead,
            ready=ready,
            publish=publish,
            capture=capture,
            destroy=destroy,
        )


class StopFake(Exception):
    pass


class IntentTests(GuardFixture):
    def test_intent_exists_and_matches_before_arm_empty_response(self):
        self.run_watch()
        self.assertEqual(self.events[0], ("armed", self.intent["label"]))
        self.assertEqual(self.events[1], ("create", self.intent["label"]))
        self.assertEqual(G.load(self.ref), self.intent)

    def test_intent_exists_before_arm_timeout_parent_death(self):
        self.run_watch(mode="timeout")
        self.assertEqual(self.events[0][0], "armed")
        self.assertEqual(self.events[1][1], self.intent["label"])

    def test_intent_cannot_be_overwritten(self):
        with self.assertRaises(FileExistsError):
            G.prepare(self.root, self.offer, self.plan, 71, self.snapshot(), SOURCE)

    def test_tampered_intent_is_refused(self):
        (self.root / "intent.json").chmod(0o600)
        value = dict(self.intent, host_id=999)
        (self.root / "intent.json").write_text(json.dumps(value))
        with self.assertRaises(G.Unresolved):
            G.load(self.ref)

    def test_two_labels_differ_despite_same_timestamp(self):
        other = self.root / "other"
        other.mkdir()
        ref = G.prepare(other, self.offer, self.plan, 71, self.snapshot(), SOURCE)
        self.assertNotEqual(self.intent["label"], G.load(ref)["label"])

    def test_labels_have_full_collision_resistant_nonce(self):
        self.assertRegex(self.intent["label"], r"^codex-sw5-c-[0-9a-f]{32}$")

    def test_wrong_ready_intent_cannot_issue(self):
        with self.assertRaises(G.Unresolved):
            G.issue(
                self.root,
                self.ref,
                dict(state="ARMED", intent_sha256="0" * 64),
                lambda _: self.fail("create reached"),
                now=lambda: NOW,
            )

    def test_unarmed_ready_cannot_issue(self):
        with self.assertRaises(G.Unresolved):
            G.issue(
                self.root,
                self.ref,
                dict(state="NOT_ARMED", intent_sha256=self.ref.sha256),
                lambda _: self.fail("create reached"),
                now=lambda: NOW,
            )


class IdentityTests(GuardFixture):
    def test_exact_owned_candidate_binds(self):
        self.assertEqual(G.match(self.intent, self.snapshot([self.row()])), 42)

    def reject(self, rows, **changes):
        with self.assertRaises(G.Unresolved):
            G.match(self.intent, self.snapshot(rows, **changes))

    def test_foreign_label_never_binds(self):
        self.reject([self.row(label="foreign")])

    def test_prefix_label_never_binds(self):
        self.reject([self.row(label=self.intent["label"] + "x")])

    def test_wrong_owner_context_never_binds(self):
        self.reject([self.row()], owner_id=72)

    def test_missing_owner_context_never_binds(self):
        self.reject([self.row()], owner_id=None)

    def test_wrong_machine_never_binds(self):
        self.reject([self.row(machine_id=1)])

    def test_missing_machine_never_binds(self):
        self.reject([self.row(machine_id=None)])

    def test_wrong_host_never_binds(self):
        self.reject([self.row(host_id=1)])

    def test_missing_host_never_binds(self):
        self.reject([self.row(host_id=None)])

    def test_optional_foreign_client_id_never_binds(self):
        self.reject([self.row(client_id=72)])

    def test_optional_malformed_client_id_never_binds(self):
        self.reject([self.row(client_id="71")])

    def test_multiple_matching_candidates_stay_unresolved(self):
        self.reject([self.row(), self.row(id=43)])

    def test_same_label_mismatch_prevents_partial_binding(self):
        self.reject([self.row(), self.row(id=43, host_id=999)])

    def test_partial_inventory_never_binds(self):
        self.reject([self.row()], complete=False)

    def test_missing_completeness_never_binds(self):
        self.reject([self.row()], complete=None)

    def test_preexisting_id_is_excluded(self):
        self.intent["excluded_ids"] = [42]
        self.reject([self.row()])

    def test_another_preexisting_id_is_excluded(self):
        self.intent["excluded_ids"] = [43]
        self.reject([self.row(id=43)])


class ReceiptTests(GuardFixture):
    def test_empty_response_retains_safe_outcome(self):
        with self.assertRaises(G.Unresolved):
            self.issue()
        self.assertEqual(
            json.loads((self.root / "create_outcome.json").read_text())["kind"], "UNRESOLVED"
        )

    def test_timeout_retains_safe_outcome(self):
        with self.assertRaises(G.Unresolved):
            self.issue("timeout")
        v = json.loads((self.root / "create_outcome.json").read_text())
        self.assertEqual(v["exception_type"], "TimeoutExpired")

    def test_malformed_output_never_persists_secret(self):
        with self.assertRaises(G.Unresolved):
            self.issue("malformed")
        self.assertNotIn(SECRET, "".join(p.read_text() for p in self.root.glob("*.json")))
        self.assertTrue((self.root / "create_outcome.json").exists())

    def test_timeout_never_persists_secret(self):
        with self.assertRaises(G.Unresolved):
            self.issue("timeout")
        self.assertNotIn(SECRET, "".join(p.read_text() for p in self.root.glob("*.json")))
        self.assertTrue((self.root / "create_outcome.json").exists())

    def test_success_only_retains_validated_fields(self):
        self.assertEqual(self.issue("success"), 42)
        self.assertNotIn(SECRET, (self.root / "create_outcome.json").read_text())

    def test_nonzero_only_retains_safe_metadata(self):
        with self.assertRaises(G.Unresolved):
            self.issue("nonzero")
        self.assertNotIn(SECRET, (self.root / "create_outcome.json").read_text())

    def test_empty_response_cannot_be_retried(self):
        with self.assertRaises(G.Unresolved):
            self.issue()
        with self.assertRaises(G.Unresolved):
            self.issue("success")
        self.assertEqual(sum(e[0] == "create" for e in self.events), 1)

    def test_timeout_cannot_be_retried(self):
        with self.assertRaises(G.Unresolved):
            self.issue("timeout")
        with self.assertRaises(G.Unresolved):
            self.issue("success")
        self.assertEqual(sum(e[0] == "create" for e in self.events), 1)

    def test_started_marker_precedes_transport_empty(self):
        self.marker_order("empty")

    def test_started_marker_precedes_transport_timeout(self):
        self.marker_order("timeout")

    def marker_order(self, mode):
        def invoke(_):
            self.assertTrue((self.root / "create_started.json").exists())
            if mode == "timeout":
                raise subprocess.TimeoutExpired("fake", 1)
            return subprocess.CompletedProcess([], 0, b"", b"")

        with self.assertRaises(G.Unresolved):
            G.issue(
                self.root,
                self.ref,
                dict(state="ARMED", intent_sha256=self.ref.sha256),
                invoke,
                now=lambda: NOW,
            )


class WatchTests(GuardFixture):
    def test_allocated_empty_response_recovered_with_empty_ledger(self):
        result = self.run_watch()
        self.assertEqual(result["id"], 42)
        self.assertTrue(result["absence_verified"])
        self.assertIn(("destroy", 42), self.events)

    def test_allocated_timeout_parent_death_recovered(self):
        result = self.run_watch(mode="timeout")
        self.assertTrue(result["absence_verified"])
        self.assertEqual([e for e in self.events if e[0] == "destroy"], [("destroy", 42)])

    def test_capture_precedes_recovered_destroy_empty(self):
        self.run_watch()
        self.assertLess(self.events.index(("capture", 42)), self.events.index(("destroy", 42)))

    def test_capture_precedes_recovered_destroy_timeout(self):
        self.run_watch(mode="timeout")
        self.assertLess(self.events.index(("capture", 42)), self.events.index(("destroy", 42)))

    def test_inventory_error_keeps_liability_empty_response(self):
        self.unresolved(error=True)

    def test_inventory_error_keeps_liability_timeout(self):
        self.unresolved(error=True, mode="timeout")

    def test_ambiguous_inventory_keeps_liability(self):
        self.unresolved(rows=[self.row(), self.row(id=43)])

    def test_same_label_wrong_host_keeps_liability(self):
        self.unresolved(rows=[self.row(host_id=99)])

    def test_empty_inventory_after_issue_is_not_proven_no_allocation(self):
        self.unresolved(rows=[])

    def test_empty_inventory_after_timeout_is_not_proven_no_allocation(self):
        self.unresolved(rows=[], mode="timeout")

    def unresolved(self, *, error=False, rows=None, mode="empty"):
        with self.assertRaises(StopFake) as caught:
            self.run_watch(mode=mode, rows=rows, inventory_error=error)
        result = caught.exception.args[0]
        self.assertEqual(result["state"], "UNRESOLVED")
        self.assertEqual(result["liability_usd"], 1.23)
        self.assertFalse(result["retry_allowed"])
        self.assertFalse(any(e[0] == "destroy" for e in self.events))
        self.assertEqual(sum(e[0] == "create" for e in self.events), 1)


class BoundDeadlineTests(GuardFixture):
    def test_bound_then_inventory_fails_cannot_suppress_deadline_destroy(self):
        self.known_failure("inventory")

    def test_bound_then_credit_fails_cannot_suppress_deadline_destroy(self):
        self.known_failure("credit")

    def test_bound_then_rate_metadata_fails_cannot_suppress_deadline_destroy(self):
        self.known_failure("metadata")

    def known_failure(self, mode):
        self.root = self.root / "short"
        self.root.mkdir()
        plan = dict(self.plan, seconds=60, capture_lead_seconds=20,
                    deadline_utc=(NOW + dt.timedelta(seconds=60)).isoformat())
        self.ref = G.prepare(self.root, self.offer, plan, 71, self.snapshot(), SOURCE)
        self.intent = G.load(self.ref)
        self.issue("success")
        clock = [NOW]
        calls = [0]
        events = []
        states = []
        def inventory():
            calls[0] += 1
            if calls[0] > 1 and mode == "inventory": raise OSError(SECRET)
            row = self.row()
            if calls[0] > 1 and mode == "metadata": row['dph_total'] = None
            return self.snapshot([row])
        def credit():
            if calls[0] > 1 and mode == "credit": raise ValueError(SECRET)
            return 40
        def capture(iid):
            events.append(("capture", iid, clock[0]))
            raise OSError("fake unavailable capture")
        def publish(value):
            states.append(value)
            if clock[0] >= NOW + dt.timedelta(seconds=100): raise StopFake()
        def pause(seconds): clock[0] += dt.timedelta(seconds=seconds)
        try:
            G.watch(self.root, self.ref, inventory=inventory, credit=credit,
                    now=lambda:clock[0], pause=pause, parent_alive=lambda _: True,
                    ready=lambda _: None, publish=publish, capture=capture,
                    destroy=lambda iid: events.append(("destroy", iid, clock[0])))
        except StopFake:
            pass
        destroyed = [e for e in events if e[0] == "destroy"]
        self.assertTrue(destroyed, "known owned ID survived the original fixed deadline")
        self.assertEqual({e[1] for e in destroyed}, {42})
        self.assertLessEqual(destroyed[0][2], NOW + dt.timedelta(seconds=60))
        self.assertEqual(events[0][0], "capture")
        self.assertEqual(states[-1]['state'], "UNRESOLVED")
        self.assertEqual(states[-1]['liability_usd'], 1.23)
        self.assertFalse(states[-1].get('absence_verified', False))


if __name__ == "__main__":
    unittest.main()
