#!/usr/bin/env python3
"""Offline console redaction checks; no provider or guest requests."""
import importlib.util
from pathlib import Path
import unittest


class ConsoleRedactionTests(unittest.TestCase):
    def setUp(self):
        path = Path(__file__).with_name('console_redact.py')
        spec = importlib.util.spec_from_file_location('console_redact', path)
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)

    def test_boot_phase_retained_without_identifiers(self):
        text = ('Windows Boot Manager 12345678-1234-1234-1234-123456789abc '
                'at 203.0.113.17 fe80::1 serial=BOARD123')
        result = self.module.redact(text)
        self.assertIn('Windows Boot Manager', result)
        for value in ['12345678-1234', '203.0.113.17', 'fe80::1', 'BOARD123']:
            self.assertNotIn(value, result)

    def test_error_retained_without_identifiers(self):
        text = ('Failed disk serial=BOARD456 mac=00:11:22:33:44:55 2001:db8::2 '
                'uuid=12345678-1234-1234-1234-123456789abc')
        result = self.module.redact(text)
        self.assertIn('Failed disk', result)
        for value in ['BOARD456', '00:11:22:33:44:55', '12345678-1234', '2001:db8::2']:
            self.assertNotIn(value, result)

    def test_download_phase_retained_without_secrets(self):
        text = ('Download complete https://example.invalid/file?token=PRIVATE '
                'ssh-ed25519 QUJDREVGRw== secret=HIDDEN')
        result = self.module.redact(text)
        self.assertIn('Download complete', result)
        for value in ['example.invalid', 'PRIVATE', 'QUJDREVGRw==', 'HIDDEN']:
            self.assertNotIn(value, result)

    def test_retry_error_retained_without_secrets(self):
        text = ('Decryption has failed https://example.invalid/private '
                'ssh-ed25519 SEVDUkVU token=HIDDEN')
        result = self.module.redact(text)
        self.assertIn('Decryption has failed', result)
        for value in ['example.invalid', 'SEVDUkVU', 'HIDDEN']:
            self.assertNotIn(value, result)


if __name__ == '__main__':
    unittest.main()
