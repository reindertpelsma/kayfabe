"""Account-scoped complete inventory and exactly one provider create HTTP attempt."""

import subprocess
import unittest
try:
    import provider_inventory as P
except ImportError:
    P = None


class ProviderTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(P, 'provider inventory adapter missing')

    def pages(self, *, changed_owner=False, incomplete=False, duplicate=False):
        events = []
        users = iter([dict(id=71, credit=40), dict(id=72 if changed_owner else 71, credit=40)])
        def query(args):
            events.append(args)
            if args[:2] == ['show', 'user']: return next(users)
            second = '--next-token' in args
            if second and incomplete: raise OSError('fake inventory page unavailable')
            row = dict(id=41 if not second or duplicate else 42, label='fixture',
                       machine_id=21018, host_id=120071, dph_total=.24,
                       instance_api_key='FAKE_MUST_NOT_RETURN')
            return dict(success=True, instances=[row], instances_found=1,
                        total_instances=2, next_token=None if second else 'cursor')
        return query, events

    def test_inventory_fetches_every_page(self):
        query, events = self.pages()
        result = P.snapshot(71, query=query)
        self.assertTrue(result['complete'])
        self.assertEqual([r['id'] for r in result['rows']], [41, 42])
        self.assertEqual(sum('--next-token' in e for e in events), 1)

    def test_inventory_projects_no_provider_secrets(self):
        query, _ = self.pages()
        self.assertNotIn('FAKE_MUST_NOT_RETURN', str(P.snapshot(71, query=query)))

    def test_changed_account_is_unresolved(self):
        query, _ = self.pages(changed_owner=True)
        with self.assertRaises(P.InventoryUnresolved): P.snapshot(71, query=query)

    def test_wrong_starting_account_is_unresolved(self):
        query, _ = self.pages()
        with self.assertRaises(P.InventoryUnresolved): P.snapshot(72, query=query)

    def test_failed_second_page_cannot_claim_complete(self):
        query, _ = self.pages(incomplete=True)
        with self.assertRaises(P.InventoryUnresolved): P.snapshot(71, query=query)

    def test_duplicate_cross_page_ids_cannot_claim_complete(self):
        query, _ = self.pages(duplicate=True)
        with self.assertRaises(P.InventoryUnresolved): P.snapshot(71, query=query)

    def create_command(self, *, timeout=False):
        events = []
        def run(args, **kwargs):
            events.append(args)
            if timeout: raise subprocess.TimeoutExpired(args, 45)
            return subprocess.CompletedProcess(args, 0, b'', b'')
        try: P.create_once(900, 'codex-sw5-c-' + 'a' * 32, run=run)
        except subprocess.TimeoutExpired: pass
        self.assertEqual(len(events), 1)
        args = events[0]
        self.assertEqual(args[args.index('--retry') + 1], '1')
        self.assertEqual(args.count('create'), 1)
        self.assertNotIn('--api-key', args)
        self.assertNotIn('--explain', args)
        self.assertNotIn('--curl', args)

    def test_empty_create_transport_has_one_http_attempt(self): self.create_command()
    def test_timeout_create_transport_has_one_http_attempt(self): self.create_command(timeout=True)


if __name__ == '__main__': unittest.main()
