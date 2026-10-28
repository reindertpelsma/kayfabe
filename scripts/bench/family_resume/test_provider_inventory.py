"""Account-scoped complete inventory and exactly one provider create HTTP attempt."""

import subprocess
import unittest

try:
    import provider_inventory as P
except ImportError:
    P = None


class ProviderTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(P, "provider inventory adapter missing")

    def pages(self, *, changed_owner=False, incomplete=False, duplicate=False):
        events = []
        users = iter([dict(id=71, credit=40), dict(id=72 if changed_owner else 71, credit=40)])

        def query(args, **kwargs):
            events.append(args)
            if args[:2] == ["show", "user"]:
                return next(users)
            second = "--next-token" in args
            if second and incomplete:
                raise OSError("fake inventory page unavailable")
            row = dict(
                id=41 if not second or duplicate else 42,
                label="fixture",
                machine_id=21018,
                host_id=120071,
                dph_total=0.24,
                start_date=1234.125,
                instance_api_key="FAKE_MUST_NOT_RETURN",
            )
            return dict(
                success=True,
                instances=[row],
                instances_found=1,
                total_instances=2,
                next_token=None if second else "cursor",
            )

        return query, events

    def test_inventory_fetches_every_page(self):
        query, events = self.pages()
        result = P.snapshot(71, query=query)
        self.assertTrue(result["complete"])
        self.assertEqual([r["id"] for r in result["rows"]], [41, 42])
        self.assertEqual(sum("--next-token" in e for e in events), 1)

    def test_inventory_projects_no_provider_secrets(self):
        query, _ = self.pages()
        self.assertNotIn("FAKE_MUST_NOT_RETURN", str(P.snapshot(71, query=query)))

    def test_projection_preserves_creation_timestamp(self):
        query, _ = self.pages()
        self.assertEqual(P.snapshot(71, query=query)['rows'][0]['start_date'], 1234.125)

    def test_second_page_preserves_creation_timestamp(self):
        query, _ = self.pages()
        self.assertEqual(P.snapshot(71, query=query)['rows'][1]['start_date'], 1234.125)

    def test_changed_account_is_unresolved(self):
        query, _ = self.pages(changed_owner=True)
        with self.assertRaises(P.InventoryUnresolved):
            P.snapshot(71, query=query)

    def test_wrong_starting_account_is_unresolved(self):
        query, _ = self.pages()
        with self.assertRaises(P.InventoryUnresolved):
            P.snapshot(72, query=query)

    def test_failed_second_page_cannot_claim_complete(self):
        query, _ = self.pages(incomplete=True)
        with self.assertRaises(P.InventoryUnresolved):
            P.snapshot(71, query=query)

    def test_duplicate_cross_page_ids_cannot_claim_complete(self):
        query, _ = self.pages(duplicate=True)
        with self.assertRaises(P.InventoryUnresolved):
            P.snapshot(71, query=query)

    def create_command(self, *, timeout=False):
        events = []

        def run(args, **kwargs):
            events.append(args)
            if timeout:
                raise subprocess.TimeoutExpired(args, 45)
            return subprocess.CompletedProcess(args, 0, b"", b"")

        try:
            P.create_once(900, "codex-sw5-c-" + "a" * 32, run=run)
        except subprocess.TimeoutExpired:
            pass
        self.assertEqual(len(events), 1)
        args = events[0]
        self.assertEqual(args[args.index("--retry") + 1], "1")
        self.assertEqual(args.count("create"), 1)
        self.assertNotIn("--api-key", args)
        self.assertNotIn("--explain", args)
        self.assertNotIn("--curl", args)

    def test_empty_create_transport_has_one_http_attempt(self):
        self.create_command()

    def test_timeout_create_transport_has_one_http_attempt(self):
        self.create_command(timeout=True)


class SnapshotBudgetTests(unittest.TestCase):
    def run_budget(self, increments, budget):
        clock = [0.0]
        calls = []

        def query(args, *, timeout):
            calls.append((list(args), timeout))
            clock[0] += increments
            if args[:2] == ["show", "user"]:
                return dict(id=71, credit=40)
            count = len([v for v, _ in calls if v[:2] != ["show", "user"]])
            return dict(
                success=True,
                instances=[dict(id=40 + count)],
                instances_found=1,
                total_instances=400,
                next_token="page" + str(count),
            )

        with self.assertRaises(P.InventoryUnresolved):
            P.snapshot(71, query=query, budget_seconds=budget, monotonic=lambda: clock[0])
        return calls

    def test_slow_pages_share_one_total_budget(self):
        calls = self.run_budget(20, 30)
        self.assertLessEqual(len(calls), 2)
        self.assertEqual([t for _, t in calls], [30, 10])

    def test_many_pages_stop_at_total_budget_without_partial_complete(self):
        calls = self.run_budget(0.1, 0.35)
        self.assertLessEqual(len(calls), 4)
        self.assertLess(calls[-1][1], calls[0][1])

    def test_expired_budget_issues_no_api_call(self):
        calls = []
        with self.assertRaises(P.InventoryUnresolved):
            P.snapshot(71, query=lambda *a, **k: calls.append(a), budget_seconds=0)
        self.assertEqual(calls, [])

    def test_clock_expired_before_first_request_issues_no_api_call(self):
        calls = []
        clock = iter([0, 31])
        with self.assertRaises(P.InventoryUnresolved):
            P.snapshot(
                71,
                query=lambda *a, **k: calls.append(a),
                budget_seconds=30,
                monotonic=lambda: next(clock),
            )
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
