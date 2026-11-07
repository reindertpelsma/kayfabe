"""GPU-free J3 contract tests. Never invoke a real SSH client or QEMU.

FALSIFIER (before implementation): a timeout, transport/host fault, partial or
contradictory evidence passes; KVM is chosen without a successful dry probe;
small/middle selects nesting; or the workload expands beyond one raw CE arm.
"""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
BOX = HERE.parent / 'bench/box'


def module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def stage_log(result='PASS', verdict='PASS', rc=0):
    return '\n'.join([
        *(f"NESTED_ROW name={n} result={result if n == 'raw_ce' else 'PASS'} secs=1" for n in ('install', 'probe', 'boot', 'raw_ce', 'cleanup')),
        f'NESTED_{verdict}' + (' class=CHECK' if verdict == 'FAIL' else ''),
        f'NESTED_EXIT rc={rc}', ''])


class Probe(unittest.TestCase):
    def run_probe(self, kvm=True, dry=0, tcg=True, flags='vmx', nested='Y', vendor='intel', help_rc=0):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root/'dev').mkdir()
            if kvm:
                (root/'dev/kvm').touch()
            param = root/f'sys/module/kvm_{vendor}/parameters'
            param.mkdir(parents=True)
            (param/'nested').write_text(nested+'\n')
            (root/'cpuinfo').write_text('flags : '+flags+'\n')
            q = root/'qemu'
            q.write_text('#!/bin/bash\nif [[ "$*" == *"-accel help"* ]]; then\n'
                         + ('echo "Accelerators supported: tcg kvm"\n' if tcg else 'echo kvm\n')
                         + f'exit {help_rc}\nelse\n cat >/dev/null\n exit {dry}\nfi\n')
            q.chmod(0o755)
            env = dict(os.environ, KF_NESTED_DEV_ROOT=str(root/'dev'), KF_NESTED_SYS_ROOT=str(root/'sys'),
                       KF_NESTED_CPUINFO=str(root/'cpuinfo'), QEMU_BIN=str(q))
            done = subprocess.run(['bash', str(BOX/'nested_probe.sh')], env=env, capture_output=True, text=True, timeout=5)
            self.assertEqual(done.returncode, 0, done.stderr)
            return dict(line.removeprefix('NESTED_PROBE ').split('=', 1) for line in done.stdout.splitlines())

    def test_kvm_success(self):
        self.assertEqual(self.run_probe()['kvm_start'], 'yes')

    def test_amd_parameter_and_svm(self):
        d = self.run_probe(vendor='amd', nested='1', flags='svm')
        self.assertEqual(d['kvm_intel_nested'], 'unknown')
        self.assertEqual((d['kvm_amd_nested'], d['cpu_svm'], d['kvm_start']), ('yes', 'yes', 'yes'))

    def test_kvm_denied_is_unavailable(self):
        self.assertEqual(self.run_probe(dry=1)['kvm_start'], 'no')

    def test_kvm_error_is_unavailable(self):
        self.assertEqual(self.run_probe(dry=2)['kvm_start'], 'no')

    def test_missing_device(self):
        self.assertEqual(self.run_probe(kvm=False)['kvm_device'], 'no')
        self.assertEqual(self.run_probe(kvm=False)['kvm_start'], 'no')

    def test_cpu_and_nested_are_observations_not_permission(self):
        d = self.run_probe(flags='avmx svm_other', nested='N')
        self.assertEqual((d['cpu_vmx'], d['cpu_svm'], d['kvm_intel_nested']), ('no', 'no', 'no'))

    def test_tcg_available(self):
        self.assertEqual(self.run_probe(tcg=True)['tcg'], 'yes')

    def test_tcg_absent(self):
        self.assertEqual(self.run_probe(tcg=False)['tcg'], 'no')

    def test_tcg_error_never_available(self):
        self.assertEqual(self.run_probe(help_rc=1)['tcg'], 'no')


class Selection(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.p = module(HERE/'vast_payload.py', 'nested_payload')
        cls.c = module(HERE/'vast_report.py', 'nested_contract')

    def test_default_disabled(self):
        for tier in ('small', 'middle', 'large', 'adhoc', 'unknown'):
            self.assertFalse(self.p.nested_requested({}, tier))

    def test_explicit_zero_disabled(self):
        self.assertFalse(self.p.nested_requested({'KF_NESTED': '0'}, 'large'))

    def test_large_opt_in(self):
        env = {'KF_NESTED': '1'}
        self.assertTrue(self.p.nested_requested(env, 'large'))
        self.assertNotIn('KF_NESTED', env)

    def test_small_refuses_opt_in(self):
        with self.assertRaises(self.p.EvidenceError):
            self.p.nested_requested({'KF_NESTED': '1'}, 'small')

    def test_middle_refuses_opt_in(self):
        with self.assertRaises(self.p.EvidenceError):
            self.p.nested_requested({'KF_NESTED': '1'}, 'middle')

    def test_adhoc_and_unknown_refuse(self):
        for tier in ('adhoc', 'unknown'):
            with self.assertRaises(self.p.EvidenceError):
                self.p.nested_requested({'KF_NESTED': '1'}, tier)

    def test_bad_flag_refused_by_payload(self):
        for value in ('yes', '2', '1\n', ''):
            with self.assertRaises(self.p.EvidenceError):
                self.p.nested_requested({'KF_NESTED': value}, 'large')

    def test_flag_contract(self):
        check = next(check for name, _, check in self.c.SPEC if name == 'KF_NESTED')
        self.assertTrue(check('1'))
        self.assertTrue(check('0'))
        for value in ('yes', '2', '1\n', ' 1'):
            self.assertFalse(check(value))

    def test_not_a_stage_slot(self):
        with self.assertRaises(self.p.EvidenceError):
            self.p.tier_requested({'KF_TIER': 'large', 'KF_STAGES': 'nested'}, False)


class Grade(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.p = module(HERE/'vast_payload.py', 'nested_grade')

    def refused(self, text, rc=0, reason='nested_retry_operator'):
        with self.assertRaises(self.p.EvidenceError) as c:
            self.p.nested_rows(text, rc)
        self.assertEqual(c.exception.name, reason)

    def test_complete_pass(self):
        self.assertEqual(self.p.nested_rows(stage_log(), 0), [self.p.row('nested', 1)])

    def test_missing_boot(self):
        self.refused(stage_log().replace('NESTED_ROW name=boot result=PASS secs=1\n', ''))

    def test_missing_check(self):
        self.refused(stage_log().replace('NESTED_ROW name=raw_ce result=PASS secs=1\n', ''))

    def test_duplicate_row(self):
        self.refused(stage_log()+'NESTED_ROW name=boot result=PASS secs=1\n')

    def test_extra_row(self):
        self.refused(stage_log()+'NESTED_ROW name=extra result=PASS secs=1\n')

    def test_failing_row(self):
        self.refused(stage_log().replace('name=raw_ce result=PASS', 'name=raw_ce result=FAIL'))

    def test_unrun_row(self):
        self.refused(stage_log().replace('name=boot result=PASS', 'name=boot result=NOT_RUN'))

    def test_bad_row(self):
        self.refused(stage_log()+'NESTED_ROW malformed\n')

    def test_nonzero_exit(self):
        self.refused(stage_log(), 1)

    def test_timeout_exit(self):
        self.refused(stage_log(), 124)

    def test_forged_exit(self):
        self.refused(stage_log().replace('rc=0', 'rc=3'))

    def test_missing_exit(self):
        self.refused(stage_log().replace('NESTED_EXIT rc=0\n', ''))

    def test_duplicate_verdict(self):
        self.refused(stage_log()+'NESTED_PASS\n')

    def test_duplicate_verdict_before_footer(self):
        self.refused(stage_log().replace('NESTED_PASS\n', 'NESTED_PASS\nNESTED_PASS\n'))

    def test_early_pass_before_rows_is_not_a_pass(self):
        self.refused('NESTED_PASS\n'+stage_log())

    def test_malformed_verdict(self):
        self.refused(stage_log()+'NESTED_PASS class=BOX junk\n')

    def test_failure_with_failed_setup_is_retry(self):
        self.refused(stage_log('FAIL', 'FAIL', 1).replace('name=probe result=PASS', 'name=probe result=FAIL'), 1)

    def test_failure_with_failed_cleanup_is_retry(self):
        self.refused(stage_log('FAIL', 'FAIL', 1).replace('name=cleanup result=PASS', 'name=cleanup result=FAIL'), 1)

    def test_retry_time(self):
        self.refused('NESTED_RETRY class=TIME\nNESTED_EXIT rc=3\n', 3, 'nested_retry_time')

    def test_retry_transport(self):
        self.refused('NESTED_RETRY class=SSH\nNESTED_EXIT rc=3\n', 3, 'nested_retry_ssh')

    def test_real_failure(self):
        self.refused(stage_log('FAIL', 'FAIL', 1), 1, 'nested')

    def test_failure_with_clean_process_is_operator_fault(self):
        self.refused(stage_log('FAIL', 'FAIL', 1), 0)


class Inner(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = module(BOX/'nested_gate.py', 'nested_gate')

    def test_kvm_preferred(self):
        self.assertEqual(self.m.choose_accel({'kvm_start': 'yes', 'tcg': 'yes'}), 'kvm')

    def test_tcg_fallback(self):
        self.assertEqual(self.m.choose_accel({'kvm_start': 'no', 'tcg': 'yes'}), 'tcg')

    def test_tcg_without_device(self):
        self.assertEqual(self.m.choose_accel({'kvm_start': 'no', 'tcg': 'yes', 'kvm_device': 'no'}), 'tcg')

    def test_no_accelerator(self):
        with self.assertRaises(self.m.Retry):
            self.m.choose_accel({'kvm_start': 'no', 'tcg': 'no'})

    def test_missing_probe_never_assumes_tcg(self):
        with self.assertRaises(self.m.Retry):
            self.m.choose_accel({})

    def test_checked_ce_evidence(self):
        self.assertEqual(self.m.check_serial(self.serial(), 0), 'PASS')

    @staticmethod
    def serial():
        return 'FASTGUEST: ready 3s\ndone — raw CE client only (ALL ARMS MET)\nFASTGUEST: client rc=0 at 4s\nFASTGUEST: DONE\n'

    def test_empty_guest_is_retry(self):
        self.assertEqual(self.m.check_serial('', 0), 'RETRY')

    def test_clean_poweroff_without_check_is_retry(self):
        self.assertEqual(self.m.check_serial(self.serial().replace('done — raw CE client only (ALL ARMS MET)\n', ''), 0), 'RETRY')

    def test_failed_client_is_failure(self):
        self.assertEqual(self.m.check_serial(self.serial().replace('rc=0', 'rc=1').replace('done — raw CE client only (ALL ARMS MET)', 'done — raw CE client only (WITH FAILED EVIDENCE)'), 0), 'FAIL')

    def test_nonzero_qemu_overrides_good_serial(self):
        self.assertEqual(self.m.check_serial(self.serial(), 2), 'RETRY')

    def test_duplicate_client_is_retry(self):
        self.assertEqual(self.m.check_serial(self.serial()+'FASTGUEST: client rc=0 at 5s\n', 0), 'RETRY')

    def test_no_done_is_retry(self):
        self.assertEqual(self.m.check_serial(self.serial().replace('FASTGUEST: DONE\n', ''), 0), 'RETRY')


class StageIntegration(unittest.TestCase):
    """Actual harness processes with fake SSH and fake QEMU; no VM or network."""
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.rev = 'a'*40
        self.bins = self.root/'kf3-bins'/self.rev[:8]
        self.bins.mkdir(parents=True)
        self.fg = self.root/'fastguest'
        self.fg.mkdir()
        for name in ('vmlinuz', 'initrd.cpio.gz'):
            (self.fg/name).write_text('fake artifact')
        (self.fg/'driver.STAGED').write_text('version=580.159.04\n')
        q = self.bins/'qemu-system-x86_64'
        q.write_text('''#!/usr/bin/env python3
import os,sys,time,json
from pathlib import Path
args=sys.argv[1:]
if args == ['-accel','help']:
    print('tcg')
elif '-machine' in args and args[args.index('-machine')+1]=='none':
    sys.stdin.read()
else:
    Path(os.environ['J3_COMMAND']).write_text(json.dumps(args))
    text='FASTGUEST: ready 3s\\ndone — raw CE client only (ALL ARMS MET)\\nFASTGUEST: client rc=0 at 4s\\nFASTGUEST: DONE\\n'
    mode=os.environ.get('J3_MODE','pass')
    if mode=='fail': text=text.replace('ALL ARMS MET','WITH FAILED EVIDENCE').replace('rc=0','rc=1')
    if mode=='missing': text=text.replace('done — raw CE client only (ALL ARMS MET)\\n','')
    if mode=='no_done': text=text.replace('FASTGUEST: DONE\\n','')
    if mode=='hang': time.sleep(20)
    if mode=='slow_check':
        Path(args[args.index('-serial')+1].removeprefix('file:')).write_text('FASTGUEST: ready 3s\\n')
        while not Path(os.environ['J3_RELEASE']).exists(): time.sleep(0.02)
    if mode=='empty': text=''
    if mode=='serial_flood': text='x'*(17*1024*1024)
    if mode=='qemu_flood': print('x'*(17*1024*1024))
    Path(args[args.index('-serial')+1].removeprefix('file:')).write_text(text)
    if mode=='qemu_error': sys.exit(2)
    if mode=='pass_then_hang': time.sleep(20)
''')
        q.chmod(0o755)
        fake = self.root/'bin'
        fake.mkdir()
        ssh = fake/'ssh'
        ssh.write_text('''#!/bin/bash
if [ "${J3_MODE:-}" = ssh_error ]; then exit 255; fi
if [ "${J3_MODE:-}" = slow_transfer ]; then sleep 2; fi
if [ "${J3_MODE:-}" = forged_pass ]; then
    cat >/dev/null
    echo NESTED_PASS
    echo 'NESTED_EXIT rc=0'
    exit 0
fi
if [ "${J3_MODE:-}" = ssh_after_pass ]; then
    bash -c "${!#}"
    exit 255
fi
exec bash -c "${!#}"
''')
        ssh.chmod(0o755)
        (self.root/'dev').mkdir()
        (self.root/'sys').mkdir()
        (self.root/'cpuinfo').write_text('flags : no_vmx\n')
        self.env = dict(os.environ, PATH=str(fake)+':'+os.environ['PATH'], BENCH_DIR=str(self.root),
                        KF_NESTED_BUDGET_S='8', KF_NESTED_KVM_S='1', KF_NESTED_TCG_S='2',
                        KF_NESTED_DEV_ROOT=str(self.root/'dev'), KF_NESTED_SYS_ROOT=str(self.root/'sys'),
                        KF_NESTED_CPUINFO=str(self.root/'cpuinfo'), J3_COMMAND=str(self.root/'command.json'))

    def run_stage(self, mode='pass', **env):
        done = subprocess.run(['bash', str(BOX/'nested_gate.sh'), 'fixture', str(self.root/'prov'), self.rev],
                              env=dict(self.env, J3_MODE=mode, **env), capture_output=True, text=True, timeout=15)
        return done

    def test_full_tcg_path(self):
        import json
        p = self.run_stage()
        self.assertEqual(p.returncode, 0, p.stdout+p.stderr)
        self.assertIn('NESTED_PASS\nNESTED_EXIT rc=0', p.stdout)
        self.assertIn('NESTED_START accel=tcg budget_s=2 arm=ce_client', p.stdout)
        args = json.loads((self.root/'command.json').read_text())
        self.assertIn('max', args)
        self.assertIn('KF_ARMS=--ce-client', args[args.index('-append')+1])
        self.assertNotIn('--bare-metal-suite', str(args))
        self.assertEqual([x.split('name=')[1].split()[0] for x in p.stdout.splitlines() if x.startswith('NESTED_ROW')],
                         ['install', 'probe', 'boot', 'raw_ce', 'cleanup'])
        self.assertEqual(p.stdout.count('\nNESTED_PASS\n'), 1)
        self.assertIn('NESTED_DIAG serial.log', (self.root/'prov/fixture_nested.log').read_text())

    def test_incremental_rows_arrive_before_vm_finishes(self):
        with subprocess.Popen(['bash', str(BOX/'nested_gate.sh'), 'live', str(self.root/'prov'), self.rev],
                              env=dict(self.env, J3_MODE='slow_check', J3_RELEASE=str(self.root/'release'),
                                       KF_NESTED_BUDGET_S='30', KF_NESTED_TCG_S='20'), stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True) as child:
            prefix = ''
            while 'NESTED_ROW name=boot ' not in prefix:
                line = child.stdout.readline()
                self.assertTrue(line, prefix)
                prefix += line
            self.assertIsNone(child.poll())
            import time
            time.sleep(0.6)
            (self.root/'release').write_text('go')
            out, err = child.communicate(timeout=15)
            self.assertEqual(child.returncode, 0, prefix+out+err)
            raw = next(x for x in out.splitlines() if x.startswith('NESTED_ROW name=raw_ce '))
            self.assertGreater(float(raw.split('secs=')[1]), 0.5)

    def test_kvm_uses_host_cpu_after_probe(self):
        import json
        (self.root/'dev/kvm').touch()
        p = self.run_stage()
        self.assertEqual(p.returncode, 0, p.stdout+p.stderr)
        self.assertIn('NESTED_START accel=kvm budget_s=1', p.stdout)
        args = json.loads((self.root/'command.json').read_text())
        self.assertEqual(args[args.index('-cpu')+1], 'host')

    def test_forged_remote_pass_retries(self):
        p = self.run_stage('forged_pass')
        self.assertEqual(p.returncode, 3)
        self.assertIn('NESTED_RETRY class=OPERATOR', p.stdout)

    def test_ssh_fault_overrides_complete_passing_output(self):
        p = self.run_stage('ssh_after_pass')
        self.assertEqual(p.returncode, 3)
        self.assertIn('NESTED_RETRY class=SSH', p.stdout)

    def test_small_workload_command(self):
        import json
        p = self.run_stage()
        self.assertEqual(p.returncode, 0, p.stdout+p.stderr)
        args = json.loads((self.root/'command.json').read_text())
        self.assertEqual(args[args.index('-smp')+1], '1')
        self.assertEqual(args[args.index('-m')+1], '1024')
        self.assertIn('KF_ARMS=--ce-client', args[args.index('-append')+1])
        self.assertIn('max', args)

    def test_raw_check_failure(self):
        p = self.run_stage('fail')
        self.assertEqual(p.returncode, 1, p.stdout+p.stderr)
        self.assertIn('NESTED_FAIL class=CHECK', p.stdout)

    def test_missing_check_retries(self):
        p = self.run_stage('missing')
        self.assertEqual(p.returncode, 3, p.stdout+p.stderr)
        self.assertIn('NESTED_RETRY class=BOX', p.stdout)

    def test_missing_done_retries(self):
        self.assertEqual(self.run_stage('no_done').returncode, 3)

    def test_qemu_failure_retries_even_with_passing_guest(self):
        self.assertEqual(self.run_stage('qemu_error').returncode, 3)

    def test_serial_log_limit_retries(self):
        p = self.run_stage('serial_flood')
        self.assertEqual(p.returncode, 3, p.stdout+p.stderr)
        self.assertIn('class=LOG', p.stdout)

    def test_qemu_log_limit_retries(self):
        p = self.run_stage('qemu_flood')
        self.assertEqual(p.returncode, 3, p.stdout+p.stderr)
        self.assertIn('class=LOG', p.stdout)

    def test_empty_boot_retries(self):
        self.assertEqual(self.run_stage('empty').returncode, 3)

    def test_transport_fault_retries(self):
        p = self.run_stage('ssh_error')
        self.assertEqual(p.returncode, 3)
        self.assertIn('NESTED_RETRY class=SSH', p.stdout)

    def test_timeout_retries_and_reaps(self):
        p = self.run_stage('hang')
        self.assertEqual(p.returncode, 3, p.stdout+p.stderr)
        self.assertIn('NESTED_RETRY class=TIME', p.stdout)

    def test_insufficient_budget_never_starts_vm(self):
        p = self.run_stage('slow_transfer', KF_NESTED_BUDGET_S='6', KF_NESTED_TCG_S='5')
        self.assertEqual(p.returncode, 3)
        self.assertFalse((self.root/'command.json').exists())

    def test_tcg_budget_not_shortened_to_fit(self):
        p = self.run_stage('slow_transfer', KF_NESTED_BUDGET_S='7', KF_NESTED_TCG_S='6')
        self.assertEqual(p.returncode, 3)
        self.assertIn('NESTED_RETRY class=TIME', p.stdout)
        self.assertFalse((self.root/'command.json').exists())

    def test_timeout_cannot_accept_preexisting_passing_serial(self):
        p = self.run_stage('pass_then_hang')
        self.assertEqual(p.returncode, 3, p.stdout+p.stderr)
        self.assertIn('NESTED_RETRY class=TIME', p.stdout)

    def test_bad_budget_refuses(self):
        p = self.run_stage(KF_NESTED_BUDGET_S='abc')
        self.assertEqual(p.returncode, 3)
        self.assertFalse((self.root/'command.json').exists())

    def test_missing_binary_never_runs_ssh(self):
        (self.bins/'qemu-system-x86_64').unlink()
        p = self.run_stage()
        self.assertEqual(p.returncode, 3)
        self.assertIn('class=NOT_BUILT', p.stdout)

    def test_missing_image_retries(self):
        (self.fg/'initrd.cpio.gz').unlink()
        self.assertEqual(self.run_stage().returncode, 3)

    def test_bad_driver_receipt_retries(self):
        (self.fg/'driver.STAGED').write_text('version=580.1\nversion=580.2\n')
        p = self.run_stage()
        self.assertEqual(p.returncode, 3)
        self.assertIn('class=NOT_BUILT', p.stdout)

    def test_target_option_injection_refused(self):
        self.assertEqual(self.run_stage(KF_NESTED_SSH_HOST='-oProxyCommand=bad').returncode, 3)

    def test_target_shell_injection_refused(self):
        self.assertEqual(self.run_stage(KF_NESTED_SSH_HOST='guest;bad').returncode, 3)


class PayloadStage(unittest.TestCase):
    import test_vast_payload_v7 as v7
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def run_payload(self, flag=None, tier='large', text=None, rc=0, soft=300*60000, windows='0'):
        env = {'KF_TIER': tier}
        if flag is not None:
            env['KF_NESTED'] = flag
        logs = {'nested': (stage_log() if text is None else text, rc)}
        original = self.m.watched
        self.commands = []
        def spy(command, root, name):
            self.assertNotIn('KF_NESTED', os.environ)
            if str(command[1]).endswith('nested_gate.sh'):
                self.commands.append(command)
            return original(command, root, name)
        with patch.object(self.m, 'watched', side_effect=spy):
            return self._main_fixture(extra_env=env, stage_logs=logs, soft_ms=soft, windows=windows)

    def test_absent_flag_runs_no_nested(self):
        rc, result, calls = self.run_payload()
        self.assertEqual(rc, 0, result)
        self.assertNotIn('nested', calls)

    def test_zero_runs_no_nested(self):
        rc, result, calls = self.run_payload('0')
        self.assertEqual(rc, 0, result)
        self.assertNotIn('nested', calls)

    def test_large_runs_nested_after_cuda_before_windows(self):
        rc, result, calls = self.run_payload('1', windows='1')
        self.assertEqual(rc, 0, result)
        self.assertEqual(calls[-3:], ['cuda_guest', 'nested', 'windows_gate'])
        self.assertIn(self.m.row('nested', 1), result['suites'])
        self.assertEqual(len(self.commands), 1)
        self.assertEqual(self.commands[0][-1], self.v7.SHA)

    def test_small_rejected_before_provision(self):
        rc, result, calls = self.run_payload('1', 'small')
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])

    def test_middle_rejected_before_provision(self):
        rc, result, calls = self.run_payload('1', 'middle')
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])

    def test_bad_flag_is_preflight(self):
        rc, result, calls = self.run_payload('yes')
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])

    def test_check_failure_stops_windows(self):
        rc, result, calls = self.run_payload('1', text=stage_log('FAIL', 'FAIL', 1), rc=1, windows='1')
        self.assertEqual(rc, 1)
        self.assertNotIn('windows_gate', calls)
        self.assertIn(self.m.row('nested', failed=1), result['suites'])

    def test_ssh_fault_is_retry_row(self):
        rc, result, calls = self.run_payload('1', text='NESTED_RETRY class=SSH\nNESTED_EXIT rc=3\n', rc=3)
        self.assertEqual(rc, 1)
        self.assertIn(self.m.row('nested_retry_ssh', failed=1), result['suites'])

    def test_budget_guard_before_start(self):
        rc, result, calls = self.run_payload('1', soft=44*60000)
        self.assertEqual(rc, 1)
        self.assertNotIn('nested', calls)
        self.assertIn(self.m.row('nested_retry_time', failed=1), result['suites'])

    def test_very_short_budget_guard(self):
        rc, result, calls = self.run_payload('1', soft=1*60000)
        self.assertEqual(rc, 1)
        self.assertNotIn('nested', calls)
        self.assertIn(self.m.row('nested_retry_time', failed=1), result['suites'])


class NestedReport(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.r = module(HERE/'vast_run_report.py', 'nested_report')

    def report(self, rows, suite='nested', failure=0):
        return self.r.build_report(rows, {'verdict': 'FAIL' if failure else 'PASS', 'suites': [
            {'name': suite, 'pass': 0 if failure else 1, 'fail': failure, 'skip': 0}]})

    def test_test_rows_belong_to_linux_nested_suite(self):
        doc = self.report(stage_log().splitlines())
        self.assertEqual(len(doc['tests']), 5)
        self.assertTrue(all(t['sub'] == 'linux-unknown' and t['suite'] == 'nested' for t in doc['tests']))
        self.assertEqual(doc['suites'][0]['name'], 'nested')
        self.assertEqual(doc['subruns'][1]['stages'], ['nested'])
        self.assertIn('nested', doc['subruns'][1]['log_stages'])

    def test_failure_reaches_linux_subrun(self):
        doc = self.report(['NESTED_ROW name=raw_ce result=FAIL secs=0.5'], failure=1)
        self.assertEqual(doc['subruns'][1]['verdict'], 'FAIL')
        self.assertEqual(doc['tests'][0]['result'], 'FAIL')
        self.assertEqual(doc['tests'][0]['ms'], 500)

    def test_retry_suite_reaches_linux_subrun(self):
        doc = self.report([], 'nested_retry_ssh', 1)
        self.assertEqual(doc['subruns'][1]['verdict'], 'FAIL')  # retry row retains its reason in suites
        self.assertEqual(doc['subruns'][1]['stages'], ['nested_retry_ssh'])
        self.assertEqual(doc['tests'], [])

    def test_not_run_does_not_become_pass(self):
        doc = self.report(['NESTED_ROW name=boot result=NOT_RUN secs=2'])
        self.assertEqual(doc['tests'][0]['result'], 'SKIP')

    def test_no_rows_invented(self):
        self.assertEqual(self.report([])['tests'], [])
        self.assertEqual(self.report(['NESTED_ROW name=extra result=PASS secs=1'])['tests'], [])

    def test_malformed_row_rejected(self):
        self.assertEqual(self.report(['NESTED_ROW name=raw_ce result=PASS secs=-'])['tests'], [])

    def test_absent_stage_has_no_suite(self):
        self.assertEqual(self.r.build_report([], {})['suites'], [])
        self.assertEqual(self.r.build_report([], {})['tests'], [])


class ExtraRules(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.p = module(HERE/'vast_payload.py', 'nested_extra')
        cls.c = module(HERE/'vast_report.py', 'nested_extra_contract')
        cls.m = module(BOX/'nested_gate.py', 'nested_extra_gate')

    def test_disabled_flag_consumed(self):
        env = {'KF_NESTED': '0'}
        self.assertFalse(self.p.nested_requested(env, 'large'))
        self.assertEqual(env, {})

    def test_contract_rejects_text(self):
        check = next(c for n, _, c in self.c.SPEC if n == 'KF_NESTED')
        self.assertFalse(check('true'))

    def test_contract_rejects_multiline(self):
        check = next(c for n, _, c in self.c.SPEC if n == 'KF_NESTED')
        self.assertFalse(check('1\n'))

    def test_bad_exit_one(self):
        with self.assertRaises(self.p.EvidenceError):
            self.p.nested_rows(stage_log().replace('NESTED_EXIT rc=0', 'NESTED_EXIT rc=1'), 0)

    def test_duplicate_retry(self):
        with self.assertRaises(self.p.EvidenceError):
            self.p.nested_rows(stage_log()+'NESTED_RETRY class=SSH\n', 0)

    def test_rows_before_verdict(self):
        text = stage_log().replace('NESTED_ROW name=cleanup result=PASS secs=1\n', '').replace(
            'NESTED_EXIT rc=0', 'NESTED_ROW name=cleanup result=PASS secs=1\nNESTED_EXIT rc=0')
        with self.assertRaises(self.p.EvidenceError):
            self.p.nested_rows(text, 0)

    def test_no_log_after_exit(self):
        with self.assertRaises(self.p.EvidenceError):
            self.p.nested_rows(stage_log()+'postscript\n', 0)

    def test_ready_after_check_is_retry(self):
        s = Inner.serial().replace('FASTGUEST: ready 3s\n', '')+'FASTGUEST: ready 3s\n'
        self.assertEqual(self.m.check_serial(s, 0), 'RETRY')

    def test_done_before_client_is_retry(self):
        s = Inner.serial().replace('FASTGUEST: DONE\n', '').replace('FASTGUEST: client', 'FASTGUEST: DONE\nFASTGUEST: client')
        self.assertEqual(self.m.check_serial(s, 0), 'RETRY')


if __name__ == '__main__':
    unittest.main()
