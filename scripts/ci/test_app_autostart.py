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
                                enabled=True, status="Ready", commands=["tpmmaint.exe"]),
                           dict(path="\\Microsoft\\Windows\\WindowsUpdate\\",name="Scheduled Start",xml=XML,enabled=True,status="Running",commands=["usoclient.exe"])],
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


    def test_app_automatic_services_become_manual(self):
        self.prep(); d=self.saved()
        for name in ('AppA','AppB','SharedSvc'):
            self.assertEqual(self.by_name(d,name)['start_type'],'Manual')
            self.assertEqual(self.by_name(d,name)['status'],'Stopped')
        self.assertFalse(any(a.get('mode')=='Disabled' and a['id'] in ('AppA','AppB','SharedSvc') for a in d['actions']))

    def test_delayed_automatic_service_is_demand_start(self):
        self.by_name(self.data,'AppB')['delayed_auto']=True
        self.prep(); d=self.saved()
        self.assertEqual(self.by_name(d,'AppB')['start_type'],'Manual')
        old=next(s for s in json.loads(self.report.read_text(encoding='utf-8-sig'))['services'] if s['name']=='AppB')
        self.assertEqual(old['start_type'],'Automatic'); self.assertTrue(old['delayed_auto'])

    def test_manual_services_do_not_receive_startup_writes(self):
        self.prep(); d=self.saved()
        self.assertEqual(self.by_name(d,'SharedSvc')['start_type'],'Manual')
        self.assertFalse(any(a['op']=='set_mode' and a['id']=='SharedSvc' for a in d['actions']))

    def test_preexisting_disabled_app_service_is_not_changed(self):
        self.by_name(self.data,'AppB')['start_type']='Disabled'
        self.by_name(self.data,'AppB')['status']='Stopped'
        self.prep(); d=self.saved()
        self.assertEqual(self.by_name(d,'AppB')['start_type'],'Disabled')
        self.assertFalse(any(a['id']=='AppB' for a in d['actions']))

    def test_only_explicit_updater_services_are_disabled(self):
        self.data['entries']['services'].append(service('UnknownVendorUpdater'))
        self.prep(); d=self.saved()
        for name in ('wuauserv','DiagTrack','WSearch'):
            self.assertEqual(self.by_name(d,name)['start_type'],'Disabled')
        self.assertEqual(self.by_name(d,'UnknownVendorUpdater')['start_type'],'Manual')

    def test_non_updater_never_receives_disabled_write(self):
        self.prep(); d=self.saved()
        writes=[a for a in d['actions'] if a['op']=='set_mode' and a['mode']=='Disabled']
        self.assertEqual({a['id'] for a in writes},{'wuauserv','DiagTrack','WSearch'})

    def policy(self, services=(),tasks=()):
        p=self.tmp/'policy.json'
        p.write_text(json.dumps(dict(schema_version=1,kind='autostart_policy',services=list(services),tasks=list(tasks))))
        return p

    def prep_policy(self,path,ok=True):
        self.footprint(task=True)
        return self.run_ps('lab_prep.ps1','-FootprintsDir',self.footprints,'-ReportPath',self.report,
                           '-QuarantineDir',self.tmp/'quarantine','-PolicyPath',path,ok=ok)

    def test_explicit_vendor_updater_is_disabled(self):
        self.prep_policy(self.policy(['AppB'],['\\Vendor\\AppTask']))
        self.assertEqual(self.by_name(self.saved(),'AppB')['start_type'],'Disabled')
        self.assertFalse(self.saved()['entries']['tasks'][0]['enabled'])
        r=json.loads(self.report.read_text(encoding='utf-8-sig'))
        self.assertIn('AppB',r['policy']['services'])

    def test_ordinary_scheduled_tasks_are_untouched(self):
        self.prep()
        self.assertEqual(self.saved()['entries']['tasks'][0],self.data['entries']['tasks'][0])

    def test_only_listed_update_task_is_disabled(self):
        self.prep(); tasks=self.saved()['entries']['tasks']
        self.assertFalse(tasks[2]['enabled']); self.assertEqual(tasks[2]['status'],'Ready')
        self.assertEqual(tasks[0],self.data['entries']['tasks'][0])
        self.assertFalse(any(a['op']=='disable_task' and a['id']=='\\vendor\\apptask' for a in self.saved()['actions']))

    def test_startup_entries_are_removed_without_disabling_ordinary_tasks(self):
        self.prep(); d=self.saved()
        self.assertEqual(d['entries']['run'],[]); self.assertEqual(d['entries']['startup'],[])
        self.assertTrue(d['entries']['tasks'][0]['enabled'])

    def test_previous_start_types_are_recorded(self):
        p=self.footprint(); self.prep(); r=json.loads(self.report.read_text(encoding='utf-8-sig'))
        self.assertEqual(r['schema_version'],2)
        self.assertEqual(next(s for s in r['services'] if s['name']=='AppA')['start_type'],'Automatic')
        self.assertEqual(next(s for s in r['services'] if s['name']=='SharedSvc')['start_type'],'Manual')
        self.assertEqual(r['footprints'][0]['sha256'],hashlib.sha256(p.read_bytes()).hexdigest())

    def test_prestart_is_only_a_manual_service_escape_hatch(self):
        d=self.prepared(); self.prestart(d); out=self.saved()
        for name in ('AppA','SharedSvc'):
            self.assertEqual(self.by_name(out,name)['start_type'],'Manual')
            self.assertEqual(self.by_name(out,name)['status'],'Running')
        self.assertEqual(self.by_name(out,'AppB')['status'],'Stopped')
        self.assertFalse(any(a['op']=='set_mode' for a in out['actions']))

    def test_prestart_does_not_restore_automatic_start(self):
        self.by_name(self.data,'AppA')['delayed_auto']=True
        d=self.prepared(); self.prestart(d)
        self.assertEqual(self.by_name(self.saved(),'AppA')['start_type'],'Manual')
        self.assertEqual(self.saved()['entries']['tasks'],d['entries']['tasks'])

    def test_prestart_reset_only_stops_selected_services(self):
        d=self.prepared(); self.prestart(d); active=self.saved()
        self.by_name(active,'AppB')['status']='Running'
        self.prestart(active,'app_a','-Action','Reset'); out=self.saved()
        self.assertEqual(self.by_name(out,'AppA')['status'],'Stopped')
        self.assertEqual(self.by_name(out,'AppB')['status'],'Running')
        self.assertFalse(any(a['op']=='set_mode' for a in out['actions']))

    def test_prestart_leaves_all_tasks_unchanged(self):
        d=self.prepared(); self.prestart(d)
        self.assertEqual(self.saved()['entries']['tasks'],d['entries']['tasks'])
        self.assertFalse(any(a['op'] in ('import_task','disable_task') for a in self.saved()['actions']))

    def test_updater_dependency_cannot_be_prestarted(self):
        self.by_name(self.data,'AppA')['dependencies']=['wuauserv']
        d=self.prepared(); self.by_name(d,'wuauserv')['start_type']='Manual'
        self.prestart(d,ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_indexer_dependency_cannot_be_prestarted(self):
        self.by_name(self.data,'AppA')['dependencies']=['WSearch']
        d=self.prepared(); self.by_name(d,'WSearch')['start_type']='Manual'
        self.prestart(d,ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_start_wait_failure_is_clear(self):
        d=self.prepared(); d['fail_on']=['wait:AppA']
        self.assertIn('APP_PRESTART_FAIL app=app_a',self.prestart(d,ok=False).stdout)

    def test_dependency_wait_failure_is_clear(self):
        d=self.prepared(); d['fail_on']=['wait:SharedSvc']
        self.assertIn('APP_PRESTART_FAIL app=app_a',self.prestart(d,ok=False).stdout)

    def test_policy_cannot_disable_protected_service(self):
        self.prep_policy(self.policy(['sshd']),ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_second_policy_protected_service_rejected(self):
        self.prep_policy(self.policy(['CryptSvc']),ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def reject_direct_disable(self,name):
        driver=self.tmp/'primitive.ps1'
        common=str(WIN/'app_autostart_common.ps1').replace("'","''")
        driver.write_text("param([string]$FixturePath,[string]$FixtureResultPath)\n. '"+common+"'\n"+
            "Initialize-KfBackend $FixturePath\n$c=Get-KfCatalog\n$script:KfProtected=Get-KfProtectedNames $c.entries\n"+
            "Initialize-KfPolicy $null $c.entries\ntry { Set-KfServiceMode (Get-KfServiceMap $c.entries.services)['"+name+"'] 'Disabled'; exit 0 }\n"+
            "catch { exit 1 } finally { Save-KfFixture $FixtureResultPath }\n")
        self.run_ps(driver,ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_primitive_rejects_disabling_app_service(self):
        self.reject_direct_disable('AppA')

    def test_primitive_rejects_disabling_manual_dependency(self):
        self.reject_direct_disable('SharedSvc')

    def test_policy_cannot_disable_protected_task(self):
        self.prep_policy(self.policy(tasks=['\\Microsoft\\Windows\\TPM\\ProtectedTask']),ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_wildcard_policy_service_is_rejected(self):
        self.prep_policy(self.policy(['App*']),ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_wildcard_policy_task_is_rejected(self):
        self.prep_policy(self.policy(tasks=['\\Vendor\\*']),ok=False)
        self.assertEqual(self.saved()['actions'],[])

    def test_silent_service_write_failure_is_audited(self):
        self.data['noop_on']=['set_mode:AppB']; self.prep(ok=False)
        self.assertFalse(json.loads(self.report.read_text(encoding='utf-8-sig'))['complete'])

    def test_silent_update_task_failure_is_audited(self):
        self.data['noop_on']=['disable_task:\\microsoft\\windows\\windowsupdate\\scheduled start']; self.prep(ok=False)
        self.assertFalse(json.loads(self.report.read_text(encoding='utf-8-sig'))['complete'])

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

    def test_unloaded_profile_refuses_before_mutation(self):
        self.data["unloaded_profiles"]=["S-1-5-21-2000"]
        self.prep(ok=False)
        self.assertEqual(self.saved()["actions"],[])

    def test_bad_footprint_refuses_before_mutation(self):
        p=self.footprint(); d=json.loads(p.read_text()); d["schema_version"]=2; p.write_text(json.dumps(d))
        self.prep(ok=False)
        self.assertEqual(self.saved()["actions"],[])

    def test_changed_footprint_hash_is_refused(self):
        d=self.prepared(); p=self.footprints/"app_a.json"; p.write_text(p.read_text()+" ")
        self.prestart(d,ok=False)
        self.assertEqual(self.saved()["actions"],[])

    def test_failed_lab_report_is_refused(self):
        d=self.prepared(); r=json.loads(self.report.read_text(encoding="utf-8-sig")); r["complete"]=False
        self.report.write_text(json.dumps(r)); self.prestart(d,ok=False)
        self.assertEqual(self.saved()["actions"],[])

    def test_changed_service_identity_refuses(self):
        d=self.prepared(); self.by_name(d,"AppA")["binary_path"]="C:\\Unexpected.exe"
        self.prestart(d,ok=False)
        self.assertEqual(self.saved()["actions"],[])

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
        for x in ("Assert-KfWindows","Win32NT","WaitForStatus","TimeoutSec"):
            self.assertIn(x,s)


if __name__=="__main__":
    unittest.main()
