# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import helpers as H
from contextlib import contextmanager
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock,patch
import winapps as W
import official_runtime as O


class Harness(unittest.TestCase):
    def run_host(self,crash=False):
        paths=[]
        @contextmanager
        def media(*args):
            with tempfile.TemporaryDirectory(prefix='kf-official-harness-fixture-')as tmp:
                p=Path(tmp)/'official-run.iso';p.write_bytes(b'fixture');paths.append(p)
                yield p
        def matrix(cfg,*args,**kwargs):
            self.assertTrue(Path(cfg.official_iso).is_file());self.assertEqual(cfg.official_driver,'580.88')
            if crash:raise RuntimeError('fixture-run-error')
            return 0
        with tempfile.TemporaryDirectory()as tmp,patch.dict(W.os.environ,{},clear=True),patch.object(O,'transient',side_effect=media),patch.object(W,'WinVmVm'),patch.object(W,'run_matrix',side_effect=matrix):
            args=['--run-dir',tmp,'--vm','winvm','--iso','public.iso','--parent-overlay','gate','--flags-file','flags','--official-driver','580.88','--apps','vectorAdd_demo']
            if crash:
                with self.assertRaisesRegex(RuntimeError,'fixture-run-error'):W.main(args)
            else:self.assertEqual(W.main(args),0)
        self.assertTrue(paths);self.assertFalse(paths[0].parent.exists())

    def test_transient_survives_until_matrix_finishes_then_is_deleted(self):self.run_host()
    def test_transient_deleted_on_matrix_exception(self):self.run_host(True)

    def test_unpinned_driver_stops_before_boot(self):
        with tempfile.TemporaryDirectory()as tmp,patch.dict(W.os.environ,{},clear=True),patch.object(W,'WinVmVm')as vm,patch.object(O,'transient',side_effect=AssertionError('selection must fail before downloads')):
            rc=W.main(['--run-dir',tmp,'--vm','winvm','--iso','public.iso','--parent-overlay','gate','--flags-file','flags','--official-driver','999.99','--apps','vectorAdd_demo'])
            self.assertEqual(rc,8);vm.assert_not_called()

    def test_missing_driver_selection_stops_before_boot(self):
        with tempfile.TemporaryDirectory()as tmp,patch.dict(W.os.environ,{},clear=True),patch.object(W,'WinVmVm')as vm,patch.object(O,'transient',side_effect=AssertionError('selection must fail before downloads')):
            rc=W.main(['--run-dir',tmp,'--vm','winvm','--iso','public.iso','--parent-overlay','gate','--flags-file','flags','--apps','vectorAdd_demo'])
            self.assertEqual(rc,8);vm.assert_not_called()

    def driver_mismatch(self,observed):
        with tempfile.TemporaryDirectory()as tmp:
            app=next(a for a in H.load_apps()['apps']if a['id']=='vectorAdd_demo')
            vm=Mock();vm.gsp_cycles.return_value=0
            g=W.GuestSession(W.Cfg(official_driver='580.88'),vm,H.load_apps(),H.load_manifest(),tmp,lambda x:None,'fixture')
            g.official_attached=True;g.official_cd='P:';g.driver_version=observed
            g._stage=Mock(side_effect=AssertionError('wrong driver must not launch'))
            result=g.run_app(app);self.assertEqual(result['verdict'],'NOTRUN');self.assertIn('official-driver-version-mismatch',result['line']);g._stage.assert_not_called()

    def test_wrong_installed_driver_is_notrun(self):self.driver_mismatch('999.99')
    def test_unreported_installed_driver_is_notrun(self):self.driver_mismatch(None)
