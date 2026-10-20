# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import base64
import gzip
import hashlib
import importlib.util
from pathlib import Path
import unittest
s=importlib.util.spec_from_file_location('archive',Path(__file__).with_name('archive.py'))
archive=importlib.util.module_from_spec(s)
if Path(s.origin).exists(): s.loader.exec_module(archive)


class Archive(unittest.TestCase):
    def packet(self):
        b=b'own captured trace'; return {'name':'t_010203.etl','sha256':hashlib.sha256(b).hexdigest(),'gzip':base64.b64encode(gzip.compress(b)).decode()}
    def test_valid_capture(self):
        self.assertEqual(archive.verify(self.packet()),b'own captured trace')
    def test_owned_pm_tail_is_verifiable(self):
        p=self.packet(); p['name']='pm_tail.bin'
        self.assertEqual(archive.verify(p),b'own captured trace')
    def test_empty_owned_pm_tail_is_verifiable(self):
        p={'name':'pm_tail.bin','gzip':base64.b64encode(gzip.compress(b'')).decode(),'sha256':hashlib.sha256(b'').hexdigest()}
        self.assertEqual(archive.verify(p),b'')
    def test_corrupt_digest_cannot_authorize_delete(self):
        p=self.packet();p['sha256']='0'*64
        with self.assertRaises(ValueError): archive.verify(p)
    def test_wrong_digest_cannot_authorize_delete(self):
        p=self.packet();p['sha256']=hashlib.sha256(b'other').hexdigest()
        with self.assertRaises(ValueError): archive.verify(p)
    def test_path_escape_rejected(self):
        p=self.packet();p['name']='../t_010203.etl'
        with self.assertRaises(ValueError): archive.verify(p)
    def test_unowned_filename_rejected(self):
        p=self.packet();p['name']='owner.etl'
        with self.assertRaises(ValueError): archive.verify(p)


if __name__=='__main__': unittest.main()
