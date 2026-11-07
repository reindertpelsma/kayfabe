# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Optional prestart contract against a fake guest channel, never a VM."""
import json
from types import SimpleNamespace
import unittest
from unittest.mock import Mock
import helpers as H
import gen_apps
import winapps


class Prestart(unittest.TestCase):
    def app(self, config=None):
        a=dict(id="app_a",prestart=config if config is not None else {})
        return a

    def session(self, outputs):
        g=object.__new__(winapps.GuestSession)
        g.q=Mock()
        g.q.powershell.side_effect=[SimpleNamespace(exitcode=code,text=lambda text=text:text) for code,text in outputs]
        g.vm=Mock(); g.vm.alive.return_value=True
        g.log=Mock()
        g._run_app=Mock(return_value=dict(restart="",verdict="PASS",line="APPRES PASS"))
        g._prestart_failure=Mock(return_value=dict(restart="prestart-failed",verdict="NOTRUN"))
        return g

    def test_default_app_has_no_prestart_calls(self):
        g=self.session([]); a={"id":"app_a"}
        self.assertEqual(g.run_app(a)["verdict"],"PASS")
        g.q.powershell.assert_not_called()

    def test_success_starts_before_app_and_resets_after(self):
        g=self.session([(0,"APP_PRESTART_DONE app=app_a action=Start"),(0,"APP_PRESTART_DONE app=app_a action=Reset")])
        self.assertEqual(g.run_app(self.app())["verdict"],"PASS")
        calls=g.q.powershell.call_args_list
        self.assertIn("Start",calls[0].args[1]); self.assertIn("Reset",calls[1].args[1])
        g._run_app.assert_called_once()

    def test_failed_start_never_launches_workload(self):
        g=self.session([(1,"APP_PRESTART_FAIL app=app_a action=Start"),(0,"APP_PRESTART_DONE app=app_a action=Reset")])
        self.assertEqual(g.run_app(self.app())["verdict"],"NOTRUN")
        g._run_app.assert_not_called()

    def test_missing_marker_never_launches_workload(self):
        g=self.session([(0,"unexpected"),(0,"APP_PRESTART_DONE app=app_a action=Reset")])
        self.assertEqual(g.run_app(self.app())["verdict"],"NOTRUN")
        g._run_app.assert_not_called()

    def test_cleanup_failure_forces_fresh_guest(self):
        g=self.session([(0,"APP_PRESTART_DONE app=app_a action=Start"),(1,"APP_PRESTART_FAIL app=app_a action=Reset")])
        self.assertEqual(g.run_app(self.app())["restart"],"lab-reset-failed")

    def test_exception_still_resets_environment(self):
        g=self.session([(0,"APP_PRESTART_DONE app=app_a action=Start"),(0,"APP_PRESTART_DONE app=app_a action=Reset")])
        g._run_app.side_effect=RuntimeError("fixture")
        with self.assertRaises(RuntimeError): g.run_app(self.app())
        self.assertEqual(g.q.powershell.call_count,2)

    def test_unknown_prestart_field_is_rejected(self):
        g=self.session([])
        with self.assertRaises(ValueError):g.run_app(self.app({"command":"arbitrary"}))
        g._run_app.assert_not_called()

    def test_task_override_is_rejected(self):
        with self.assertRaises(ValueError): self.session([]).run_app(self.app({'tasks':['\\Vendor\\AppTask']}))

    def test_generator_omits_prestart_by_default(self):
        self.assertTrue(all("prestart" not in a for a in gen_apps.build()["apps"]))

    def test_guest_tools_are_pushed_at_runtime(self):
        for name in ("app_prestart.ps1","app_autostart_common.ps1"):
            self.assertIn("C:\\kf\\"+name,winapps.PS_PUSH)

    def test_nonzero_with_success_marker_never_launches(self):
        g=self.session([(1,'APP_PRESTART_DONE app=app_a action=Start'),(0,'APP_PRESTART_DONE app=app_a action=Reset')])
        self.assertEqual(g.run_app(self.app())['verdict'],'NOTRUN')
        g._run_app.assert_not_called()

    def test_wrong_app_marker_never_launches(self):
        g=self.session([(0,'APP_PRESTART_DONE app=app_b action=Start'),(0,'APP_PRESTART_DONE app=app_a action=Reset')])
        self.assertEqual(g.run_app(self.app())['verdict'],'NOTRUN')
        g._run_app.assert_not_called()

    def test_cleanup_missing_marker_forces_fresh_guest(self):
        g=self.session([(0,'APP_PRESTART_DONE app=app_a action=Start'),(0,'unexpected')])
        self.assertEqual(g.run_app(self.app())['restart'],'lab-reset-failed')

    def test_cleanup_nonzero_with_marker_forces_fresh_guest(self):
        g=self.session([(0,'APP_PRESTART_DONE app=app_a action=Start'),(1,'APP_PRESTART_DONE app=app_a action=Reset')])
        self.assertEqual(g.run_app(self.app())['restart'],'lab-reset-failed')

    def test_second_unknown_field_rejected(self):
        with self.assertRaises(ValueError): self.session([]).run_app(self.app({'script':'arbitrary.ps1'}))

    def test_timeout_bounds_and_boolean_rejected(self):
        for seconds in (0,121,True):
            with self.subTest(seconds=seconds), self.assertRaises(ValueError):
                self.session([]).run_app(self.app({'timeout_s':seconds}))

    def test_negative_and_string_timeout_rejected(self):
        for seconds in (-1,'30'):
            with self.subTest(seconds=seconds), self.assertRaises(ValueError):
                self.session([]).run_app(self.app({'timeout_s':seconds}))


if __name__=="__main__":unittest.main()
