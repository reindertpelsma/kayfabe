# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""VM SSH endpoints may use relay fields without a container ports map."""
import importlib
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'windows'))


class EndpointTests(unittest.TestCase):
    def module(self):
        return importlib.import_module('readiness')

    def relay(self, **changes):
        row = dict(cur_state='running', ssh_host='ssh3.vast.ai', ssh_port=20046)
        row.update(changes)
        return row

    def test_relay_fields_nominate_running_vm(self):
        self.assertTrue(self.module().ssh_route_is_ready(self.relay()))

    def test_null_ports_do_not_hide_relay(self):
        self.assertTrue(self.module().ssh_route_is_ready(self.relay(ports=None)))

    def test_relay_endpoint_preserves_exact_host_and_port(self):
        self.assertEqual(self.module().provider_ssh_endpoint(self.relay()),
                         ('ssh3.vast.ai', 20046))

    def test_null_map_endpoint_preserves_exact_host_and_port(self):
        self.assertEqual(self.module().provider_ssh_endpoint(self.relay(ports=None)),
                         ('ssh3.vast.ai', 20046))

    def test_direct_mapped_endpoint_is_preserved(self):
        row = self.relay(public_ipaddr='192.0.2.10',
                         ports={'22/tcp': [{'HostPort': '12345'}]})
        self.assertEqual(self.module().provider_ssh_endpoint(row), ('192.0.2.10', 12345))

    def test_incomplete_relay_is_not_a_ready_route(self):
        for changes in ({'ssh_host': None}, {'ssh_port': None}, {'ssh_port': 0}):
            with self.subTest(changes=changes):
                self.assertFalse(self.module().ssh_route_is_ready(self.relay(**changes)))

    def test_bad_host_or_port_never_becomes_an_endpoint(self):
        for changes in ({'ssh_host': '-bad'}, {'ssh_host': 'bad host'},
                        {'ssh_port': 65536}, {'ssh_port': 'bad'}):
            with self.subTest(changes=changes):
                self.assertIsNone(self.module().provider_ssh_endpoint(self.relay(**changes)))

    def test_not_running_still_is_not_ready(self):
        self.assertFalse(self.module().ssh_route_is_ready(self.relay(cur_state='loading')))
