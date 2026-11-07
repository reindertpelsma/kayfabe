#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Guest autostart fixtures only: PowerShell on Linux, never a Windows provider."""
from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
WIN = ROOT / "scripts/bench/windows"
PW = os.environ.get("KF_PWSH") or shutil.which("pwsh") or "/root/am/pwsh/pwsh"
HASH = "a" * 64
XML = '<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><BootTrigger><Enabled>true</Enabled></BootTrigger></Triggers><Principals><Principal><UserId>S-1-5-18</UserId><LogonType>ServiceAccount</LogonType></Principal></Principals><Settings><Enabled>true</Enabled></Settings><Actions><Exec><Command>C:\\Apps\\helper.exe</Command></Exec></Actions></Task>'


def service(name, mode="Automatic", deps=(), kind="Own Process"):
    return dict(name=name, display_name=name, binary_path="C:\\Apps\\" + name + ".exe",
                start_type=mode, account="LocalSystem", service_type=kind,
                dependencies=list(deps), delayed_auto=False, status="Running")


def catalog():
    return dict(schema_version=1, kind="fixture", machine_id="fixture-machine", unloaded_profiles=[],
                fail_on=[], actions=[], entries=dict(
                    services=[service("sshd", deps=["RpcSs"]), service("RpcSs"), service("QEMU-GA"),
                              service("NVDisplay.ContainerLocalSystem"), service("nvlddmkm", "System", kind="Kernel Driver"),
                              service("viostor", "Boot", kind="Kernel Driver"), service("TBS"), service("CryptSvc"),
                              service("msiserver", "Manual"), service("SharedSvc", "Manual"),
                              service("AppA", deps=["SharedSvc"]), service("AppB"), service("wuauserv"),
                              service("DiagTrack"), service("WSearch")],
                    tasks=[dict(path="\\Vendor\\", name="AppTask", xml=XML, enabled=True, status="Running",
                                commands=["C:\\Apps\\helper.exe"]),
                           dict(path="\\Microsoft\\Windows\\TPM\\", name="ProtectedTask", xml=XML,
                                enabled=True, status="Ready", commands=["tpmmaint.exe"])],
                    run=[dict(key="HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run", name="AppA",
                              value="C:\\Apps\\AppA.exe", kind="String"),
                         dict(key="HKU:\\S-1-5-21-1000\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\RunOnce",
                              name="AppB", value="C:\\Apps\\AppB.exe", kind="String")],
                    startup=[dict(path="C:\\Users\\owner\\Startup\\AppA.lnk", sha256="b" * 64,
                                  bytes=32, target="C:\\Apps\\AppA.exe")]))


@unittest.skipUnless(Path(PW).is_file(), "PowerShell fixture runtime unavailable")
class AutostartFixtures(unittest.TestCase):
    def setUp(self):
        self.tmp_obj = tempfile.TemporaryDirectory(prefix="y1-autostart-")
        self.addCleanup(self.tmp_obj.cleanup)
        self.tmp = Path(self.tmp_obj.name)
        self.data = catalog()
        self.fixture = self.tmp / "fixture.json"
        self.result = self.tmp / "fixture-result.json"
        self.footprints = self.tmp / "footprints"
        self.footprints.mkdir()
        self.report = self.tmp / "lab_prep.json"
        self.env = dict(PATH=os.environ["PATH"], HOME=str(self.tmp), XDG_CONFIG_HOME=str(self.tmp / "config"),
                        XDG_CACHE_HOME=str(self.tmp / "cache"), DOTNET_CLI_HOME=str(self.tmp),
                        DOTNET_SKIP_FIRST_TIME_EXPERIENCE="1", POWERSHELL_TELEMETRY_OPTOUT="1")

    def run_ps(self, script, *args, data=None, ok=True):
        self.fixture.write_text(json.dumps(data if data is not None else self.data))
        r = subprocess.run([PW, "-NoLogo", "-NoProfile", "-NonInteractive", "-File", str(WIN / script),
                            *map(str, args), "-FixturePath", str(self.fixture), "-FixtureResultPath", str(self.result)],
                           env=self.env, text=True, capture_output=True, timeout=120)
        if ok:
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        else:
            self.assertNotEqual(r.returncode, 0)
        return r

    def footprint(self, app="app_a", names=("AppA",), task=False):
        rows = [deepcopy(s) for s in self.data["entries"]["services"] if s["name"] in names]
        doc = dict(schema_version=1, kind="app_footprint", machine_id=self.data["machine_id"],
                   created_utc="2026-11-07T10:00:00Z", app_id=app, installer_sha256=HASH,
                   before_sha256="c" * 64, after_sha256="d" * 64,
                   entries=dict(services=rows, tasks=deepcopy(self.data["entries"]["tasks"][:1]) if task else [],
                                run=[], startup=[]), changed_existing=[])
        path = self.footprints / (app + ".json")
        path.write_text(json.dumps(doc))
        return path

    def prep(self, ok=True):
        if not list(self.footprints.glob("*.json")):
            self.footprint(task=True)
        r = self.run_ps("lab_prep.ps1", "-FootprintsDir", self.footprints, "-ReportPath", self.report,
                        "-QuarantineDir", self.tmp / "quarantine", ok=ok)
        return r

    def prepared(self):
        self.prep()
        return json.loads(self.result.read_text(encoding="utf-8-sig"))

    def prestart(self, data, app="app_a", *args, ok=True):
        return self.run_ps("app_prestart.ps1", "--app", app, "-FootprintsDir", self.footprints,
                           "-LabReportPath", self.report, "-TimeoutSec", 2, *args, data=data, ok=ok)

    def saved(self):
        return json.loads(self.result.read_text(encoding="utf-8-sig"))

    def by_name(self, data, name):
        return next(s for s in data["entries"]["services"] if s["name"] == name)

    def test_snapshot_captures_all_surfaces(self):
        out = self.tmp / "snapshot.json"
        self.run_ps("app_footprint.ps1", "snapshot", "-OutputPath", out)
        d = json.loads(out.read_text(encoding="utf-8-sig"))
        self.assertEqual(d["kind"], "autostart_snapshot")
        for key in ("services", "tasks", "run", "startup"):
            self.assertEqual(d["entries"][key], self.data["entries"][key])

    def make_diff(self, before=None, after=None, **change):
        before = before or deepcopy(self.data)
        after = after or deepcopy(self.data)
        a, b = self.tmp / "before.json", self.tmp / "after.json"
        self.run_ps("app_footprint.ps1", "snapshot", "-OutputPath", a, data=before)
        self.run_ps("app_footprint.ps1", "snapshot", "-OutputPath", b, data=after)
        if change:
            d = json.loads(b.read_text(encoding="utf-8-sig")); d.update(change); b.write_text(json.dumps(d))
        out = self.tmp / "diff.json"
        return a, b, out

    def test_diff_only_new_service_is_owned(self):
        before = deepcopy(self.data); before["entries"]["services"] = []
        a,b,out = self.make_diff(before, self.data)
        self.run_ps("app_footprint.ps1", "diff", "-BeforePath", a, "-AfterPath", b,
                    "-App", "app_a", "-InstallerSha256", HASH, "-OutputPath", out)
        d=json.loads(out.read_text(encoding="utf-8-sig"))
        self.assertEqual(d["app_id"], "app_a")
        self.assertEqual(d["installer_sha256"], HASH)
        self.assertEqual(len(d["entries"]["services"]), len(self.data["entries"]["services"]))
        self.assertEqual(d["before_sha256"], hashlib.sha256(a.read_bytes()).hexdigest())

    def test_diff_does_not_claim_existing_entries(self):
        a,b,out = self.make_diff()
        self.run_ps("app_footprint.ps1", "diff", "-BeforePath", a, "-AfterPath", b,
                    "-App", "app_a", "-InstallerSha256", HASH, "-OutputPath", out)
        d=json.loads(out.read_text(encoding="utf-8-sig"))
        self.assertTrue(all(not rows for rows in d["entries"].values()))

    def test_diff_rejects_cross_machine_snapshots(self):
        a,b,out = self.make_diff(machine_id="another")
        self.run_ps("app_footprint.ps1", "diff", "-BeforePath", a, "-AfterPath", b,
                    "-App", "app_a", "-InstallerSha256", HASH, "-OutputPath", out, ok=False)
        self.assertFalse(out.exists())

    def test_lab_disables_services_missing_from_footprint(self):
        self.prep(); d=self.saved()
        for name in ("AppA", "AppB", "SharedSvc", "wuauserv", "DiagTrack", "WSearch"):
            self.assertEqual(self.by_name(d,name)["start_type"], "Disabled")
            self.assertEqual(self.by_name(d,name)["status"], "Stopped")

    def test_lab_disables_tasks_and_removes_all_startup(self):
        self.prep(); d=self.saved()
        self.assertFalse(d["entries"]["tasks"][0]["enabled"])
        self.assertEqual(d["entries"]["tasks"][0]["status"], "Ready")
        self.assertEqual(d["entries"]["run"], [])
        self.assertEqual(d["entries"]["startup"], [])

    def test_protected_services_are_never_changed(self):
        self.prep(); d=self.saved()
        names=("sshd","RpcSs","QEMU-GA","NVDisplay.ContainerLocalSystem","nvlddmkm","viostor","TBS","CryptSvc","msiserver")
        for name in names:
            self.assertEqual(self.by_name(d,name),self.by_name(self.data,name))
            self.assertFalse(any(a["id"]==name for a in d["actions"]))

    def test_protection_follows_live_dependencies(self):
        self.by_name(self.data,"sshd")["dependencies"]=["AppB"]
        self.prep(); d=self.saved()
        self.assertEqual(self.by_name(d,"AppB")["start_type"],"Automatic")
        self.assertFalse(any(a["id"]=="AppB" for a in d["actions"]))

    def test_protected_tasks_untouched(self):
        self.prep(); d=self.saved()
        self.assertEqual(d["entries"]["tasks"][1],self.data["entries"]["tasks"][1])

    def test_driver_guard_is_not_just_name_matching(self):
        self.data["entries"]["services"].append(service("UnfamiliarDriver","Boot",kind="File System Driver"))
        self.prep()
        self.assertEqual(self.by_name(self.saved(),"UnfamiliarDriver")["start_type"],"Boot")

    def test_lab_report_records_previous_types_and_hashes(self):
        p=self.footprint()
        self.prep(); r=json.loads(self.report.read_text(encoding="utf-8-sig"))
        self.assertTrue(r["complete"])
        self.assertEqual(next(s for s in r["services"] if s["name"]=="SharedSvc")["start_type"],"Manual")
        self.assertEqual(r["footprints"][0]["sha256"],hashlib.sha256(p.read_bytes()).hexdigest())

    def test_failed_mutation_never_publishes_complete_report(self):
        self.data["fail_on"]=["set_mode:AppB"]
        r=self.prep(ok=False)
        self.assertIn("LAB_PREP_FAIL",r.stdout)
        self.assertFalse(json.loads(self.report.read_text(encoding="utf-8-sig"))["complete"])

    def test_unloaded_profile_refuses_before_mutation(self):
        self.data["unloaded_profiles"]=["S-1-5-21-2000"]
        self.prep(ok=False)
        self.assertEqual(self.saved()["actions"],[])

    def test_bad_footprint_refuses_before_mutation(self):
        p=self.footprint(); d=json.loads(p.read_text()); d["schema_version"]=2; p.write_text(json.dumps(d))
        self.prep(ok=False)
        self.assertEqual(self.saved()["actions"],[])

    def test_start_selected_app_and_its_dependency_only(self):
        d=self.prepared(); self.prestart(d); out=self.saved()
        self.assertEqual(self.by_name(out,"AppA")["start_type"],"Automatic")
        self.assertEqual(self.by_name(out,"AppA")["status"],"Running")
        self.assertEqual(self.by_name(out,"SharedSvc")["start_type"],"Manual")
        self.assertEqual(self.by_name(out,"SharedSvc")["status"],"Running")
        self.assertEqual(self.by_name(out,"AppB")["start_type"],"Disabled")

    def test_start_does_not_enable_tasks_by_default(self):
        d=self.prepared(); self.prestart(d)
        self.assertFalse(self.saved()["entries"]["tasks"][0]["enabled"])

    def test_explicit_task_import_has_no_triggers(self):
        d=self.prepared(); self.prestart(d,"app_a","-TaskIds","\\Vendor\\AppTask")
        t=self.saved()["entries"]["tasks"][0]
        self.assertTrue(t["enabled"])
        import xml.etree.ElementTree as E
        xml=E.fromstring(t["xml"])
        self.assertEqual(len(xml.find('{http://schemas.microsoft.com/windows/2004/02/mit/task}Triggers')),0)

    def test_unknown_task_is_refused(self):
        d=self.prepared()
        r=self.prestart(d,"app_a","-TaskIds","\\Other\\NotOwned",ok=False)
        self.assertIn("APP_PRESTART_FAIL app=app_a",r.stdout)

    def test_changed_footprint_hash_is_refused(self):
        d=self.prepared(); p=self.footprints/"app_a.json"; p.write_text(p.read_text()+" ")
        self.prestart(d,ok=False)
        self.assertEqual(self.saved()["actions"],[])

    def test_failed_lab_report_is_refused(self):
        d=self.prepared(); r=json.loads(self.report.read_text(encoding="utf-8-sig")); r["complete"]=False
        self.report.write_text(json.dumps(r)); self.prestart(d,ok=False)
        self.assertEqual(self.saved()["actions"],[])

    def test_reset_removes_previous_apps_service_state(self):
        d=self.prepared(); self.prestart(d); active=self.saved()
        self.prestart(active,"app_a","-Action","Reset")
        self.assertEqual(self.by_name(self.saved(),"AppA")["status"],"Stopped")
        self.assertEqual(self.by_name(self.saved(),"AppA")["start_type"],"Disabled")

    def test_app_b_never_inherits_app_a_background(self):
        self.footprint(task=True); self.footprint("app_b",("AppB",)); d=self.prepared(); self.prestart(d); active=self.saved()
        self.prestart(active,"app_b")
        self.assertEqual(self.by_name(self.saved(),"AppA")["start_type"],"Disabled")
        self.assertEqual(self.by_name(self.saved(),"AppA")["status"],"Stopped")
        self.assertEqual(self.by_name(self.saved(),"AppB")["status"],"Running")

    def test_changed_service_identity_refuses(self):
        d=self.prepared(); self.by_name(d,"AppA")["binary_path"]="C:\\Unexpected.exe"
        self.prestart(d,ok=False)
        self.assertEqual(self.saved()["actions"],[])

    def test_start_wait_failure_is_clear(self):
        d=self.prepared(); d["fail_on"]=["wait:AppA"]
        r=self.prestart(d,ok=False)
        self.assertIn("APP_PRESTART_FAIL app=app_a",r.stdout)

    def test_always_off_dependency_is_refused(self):
        self.by_name(self.data,"AppA")["dependencies"]=["wuauserv"]
        d=self.prepared(); self.prestart(d,ok=False)
        self.assertEqual(self.by_name(self.saved(),"wuauserv")["start_type"],"Disabled")

    def test_unowned_updater_and_task_cannot_escape_sweep(self):
        self.data['entries']['services'].append(service('UnlistedUpdater'))
        self.prep(); d=self.saved()
        self.assertEqual(self.by_name(d,'UnlistedUpdater')['start_type'],'Disabled')
        self.assertFalse(d['entries']['tasks'][0]['enabled'])
        self.assertFalse(d['entries']['run']); self.assertFalse(d['entries']['startup'])

    def test_second_protected_dependency_and_driver(self):
        self.by_name(self.data,'QEMU-GA')['dependencies']=['SharedSvc']
        self.data['entries']['services'].append(service('RandomKernel','System',kind='Kernel Driver'))
        self.prep(); d=self.saved()
        for name in ('SharedSvc','RandomKernel','sshd','CryptSvc'):
            self.assertEqual(self.by_name(d,name),self.by_name(self.data,name))

    def test_protected_task_command_is_kept(self):
        self.data['entries']['tasks'][0]['commands']=['C:\\Windows\\sshd.exe']
        self.prep()
        self.assertEqual(self.saved()['entries']['tasks'][0],self.data['entries']['tasks'][0])

    def test_protected_run_and_shortcut_are_kept(self):
        self.data['entries']['run'][0]['value']='C:\\Tools\\qemu-ga.exe'
        self.data['entries']['startup'][0]['target']='C:\\Tools\\nvcontainer.exe'
        self.prep()
        self.assertEqual(self.saved()['entries']['run'],self.data['entries']['run'][:1])
        self.assertEqual(self.saved()['entries']['startup'],self.data['entries']['startup'])

    def test_second_profile_refusal_and_failed_report(self):
        self.data['unloaded_profiles']=['S-1-5-21-3000']
        self.prep(ok=False)
        self.assertFalse(json.loads(self.report.read_text(encoding='utf-8-sig'))['complete'])
        self.assertEqual(self.saved()['actions'],[])

    def test_unknown_footprint_field_rejected(self):
        p=self.footprint(); d=json.loads(p.read_text()); d['extra']=True; p.write_text(json.dumps(d))
        self.prep(ok=False); self.assertEqual(self.saved()['actions'],[])

    def test_boolean_schema_rejected(self):
        p=self.footprint(); d=json.loads(p.read_text()); d['schema_version']=True; p.write_text(json.dumps(d))
        self.prep(ok=False); self.assertEqual(self.saved()['actions'],[])

    def test_second_unknown_footprint_field_rejected(self):
        p=self.footprint(); d=json.loads(p.read_text()); d['command']='ignored.exe'; p.write_text(json.dumps(d))
        self.prep(ok=False); self.assertEqual(self.saved()['actions'],[])

    def test_corrupt_installer_hash_rejected(self):
        p=self.footprint(); d=json.loads(p.read_text()); d['installer_sha256']='bogus'; p.write_text(json.dumps(d))
        self.prep(ok=False); self.assertEqual(self.saved()['actions'],[])

    def test_corrupt_snapshot_hash_rejected(self):
        p=self.footprint(); d=json.loads(p.read_text()); d['before_sha256']='z'*64; p.write_text(json.dumps(d))
        self.prep(ok=False); self.assertEqual(self.saved()['actions'],[])

    def test_stop_failure_has_incomplete_report(self):
        self.data['fail_on']=['stop:AppA']; self.prep(ok=False)
        self.assertFalse(json.loads(self.report.read_text(encoding='utf-8-sig'))['complete'])

    def test_second_report_hash_drift_rejected(self):
        d=self.prepared(); p=self.footprints/'app_a.json'
        doc=json.loads(p.read_text()); doc['created_utc']='2026-11-08T00:00:00Z'; p.write_text(json.dumps(doc))
        self.prestart(d,ok=False); self.assertEqual(self.saved()['actions'],[])

    def test_report_machine_mismatch_rejected(self):
        d=self.prepared(); r=json.loads(self.report.read_text(encoding='utf-8-sig')); r['machine_id']='elsewhere'
        self.report.write_text(json.dumps(r)); self.prestart(d,ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_service_account_drift_rejected(self):
        d=self.prepared(); self.by_name(d,'AppA')['account']='Unexpected'
        self.prestart(d,ok=False); self.assertEqual(self.saved()['actions'],[])

    def test_search_dependency_stays_off(self):
        self.by_name(self.data,'AppA')['dependencies']=['WSearch']
        d=self.prepared(); self.prestart(d,ok=False)
        self.assertEqual(self.by_name(self.saved(),'WSearch')['start_type'],'Disabled')

    def test_dependency_wait_failure_is_clear(self):
        d=self.prepared(); d['fail_on']=['wait:SharedSvc']
        self.assertIn('APP_PRESTART_FAIL app=app_a',self.prestart(d,ok=False).stdout)

    def test_task_json_explicit_import_has_no_boot_trigger(self):
        d=self.prepared(); self.prestart(d,'app_a','-TaskIdsJson',json.dumps(['\\Vendor\\AppTask']))
        task=self.saved()['entries']['tasks'][0]
        self.assertNotIn('BootTrigger',task['xml']); self.assertEqual(task['status'],'Ready')

    def test_task_credentials_are_not_recreated(self):
        self.data['entries']['tasks'][0]['xml']=XML.replace('ServiceAccount','Password')
        d=self.prepared(); self.prestart(d,'app_a','-TaskIds','\\Vendor\\AppTask',ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_interactive_password_task_is_not_recreated(self):
        self.data['entries']['tasks'][0]['xml']=XML.replace('ServiceAccount','InteractiveTokenOrPassword')
        d=self.prepared(); self.prestart(d,'app_a','-TaskIds','\\Vendor\\AppTask',ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_default_prestart_keeps_task_stopped(self):
        d=self.prepared(); self.prestart(d)
        self.assertFalse(any(a['op']=='import_task' for a in self.saved()['actions']))

    def test_selected_task_must_belong_to_app(self):
        d=self.prepared(); self.prestart(d,'app_a','-TaskIds','\\Microsoft\\Windows\\TPM\\ProtectedTask',ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_reset_also_stops_explicit_task(self):
        d=self.prepared(); self.prestart(d,'app_a','-TaskIds','\\Vendor\\AppTask'); d=self.saved()
        d['entries']['tasks'][0]['status']='Running'
        self.prestart(d,'app_a','-Action','Reset')
        self.assertFalse(self.saved()['entries']['tasks'][0]['enabled'])
        self.assertEqual(self.saved()['entries']['tasks'][0]['status'],'Ready')

    def test_changed_existing_service_is_audit_only(self):
        after=deepcopy(self.data); self.by_name(after,'AppA')['binary_path']='C:\\New.exe'
        a,b,out=self.make_diff(after=after)
        self.run_ps('app_footprint.ps1','diff','-BeforePath',a,'-AfterPath',b,'-App','app_a','-InstallerSha256',HASH,'-OutputPath',out)
        d=json.loads(out.read_text(encoding='utf-8-sig'))
        self.assertEqual(d['entries']['services'],[])
        self.assertEqual(d['changed_existing'][0]['kind'],'services')

    def test_second_machine_mismatch_diff_rejected(self):
        a,b,out=self.make_diff(machine_id='not-this-instance')
        self.run_ps('app_footprint.ps1','diff','-BeforePath',a,'-AfterPath',b,'-App','app_b','-InstallerSha256',HASH,'-OutputPath',out,ok=False)
        self.assertFalse(out.exists())

    def test_prep_cannot_replace_original_modes(self):
        d=self.prepared(); original=self.report.read_bytes()
        self.run_ps('lab_prep.ps1','-FootprintsDir',self.footprints,'-ReportPath',self.report,
                    '-QuarantineDir',self.tmp/'quarantine',data=d,ok=False)
        self.assertEqual(self.report.read_bytes(),original)
        self.assertEqual(self.saved()['actions'],[])

    def test_prep_preserves_failed_report_for_inspection(self):
        self.data['fail_on']=['stop:AppA']; self.prep(ok=False)
        original=self.report.read_bytes(); self.data['fail_on']=[]
        self.prep(ok=False)
        self.assertEqual(self.report.read_bytes(),original)
        self.assertEqual(self.saved()['actions'],[])

    def test_silent_service_disable_failure_is_audited(self):
        self.data['noop_on']=['set_mode:AppB']; self.prep(ok=False)
        self.assertFalse(json.loads(self.report.read_text(encoding='utf-8-sig'))['complete'])

    def test_silent_task_disable_failure_is_audited(self):
        self.data['noop_on']=['disable_task:\\vendor\\apptask']; self.prep(ok=False)
        self.assertFalse(json.loads(self.report.read_text(encoding='utf-8-sig'))['complete'])


class WindowsSourceContracts(unittest.TestCase):
    def common(self):
        return (WIN/"app_autostart_common.ps1").read_text()

    def test_windows_collectors_cover_all_surfaces(self):
        s=self.common()
        for x in ("Win32_Service","Win32_SystemDriver","Get-Service","Get-ScheduledTask","Export-ScheduledTask",
                  "WOW6432Node","RunOnce","Win32_UserProfile","Get-FileHash","ReparsePoint"):
            self.assertIn(x,s)

    def test_windows_actions_are_literal_and_nonrecursive(self):
        s=self.common()
        for x in ("Set-Service","Disable-ScheduledTask","Stop-ScheduledTask","DeleteValue(","Move-Item -LiteralPath"):
            self.assertIn(x,s)
        self.assertNotIn("Stop-Service -Force",s)
        self.assertNotIn("Remove-Item -Recurse",s)

    def test_tpm_gpu_and_harness_have_a_protection_floor(self):
        s=self.common()
        for x in ("NVDisplay.Container*","sshd","QEMU-GA","msiserver","TBS","CryptSvc","KeyIso","Driver"):
            self.assertIn(x,s)

    def test_xml_and_windows_boundary_are_explicit(self):
        s=self.common()
        for x in ("DtdProcessing","Prohibit","XmlResolver","Assert-KfWindows","Win32NT"):
            self.assertIn(x,s)


if __name__=="__main__":
    unittest.main()
