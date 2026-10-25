"""Synthetic admission, budget and operator wiring checks; no provider contact."""

import datetime as dt
import math
import unittest

try:
    from execution import fit_plan, require_offer, ready_body, wire_arm, wire_controller
except ImportError:
    def missing(*args, **kwargs):
        raise ValueError("execution wiring absent")
    fit_plan = require_offer = ready_body = wire_arm = wire_controller = missing

NOW = dt.datetime(2026, 10, 25, 16, tzinfo=dt.timezone.utc)
END = NOW + dt.timedelta(hours=3)
ARM = 'for f in verify_stage580.ps1 "$enable_script" bw8_enable_observe.ps1 diag_min.ps1 init_state.ps1; do\n done\nenable_cmd="existing once command"\n'
CONTROLLER = 'mapfile -t plan < <(list_plan)\nbash /fixture/arm.sh on "$run" "$binary" "$source" || arm_rc=$?\n bash /fixture/collect_bw.sh "$run"\n'
READY = 'if mode == "rent":\n    create()\nelif mode == "ready":\n    C.settle_pin(rescan, console)\nelif mode == "destroy":\n    destroy()\n'


def offer():
    return dict(id=2, machine_id=3, host_id=4, gpu_name="RTX 5090", num_gpus=1,
                verification="verified", vms_enabled=True, is_vm_deverified=False,
                geolocation="Chile, CL", dph_base=.7333333333333334, storage_cost=.4,
                dph_total=5/6, inet_down_cost=.009114583333333334,
                inet_up_cost=.009114583333333334, disk_space=180,
                cpu_cores_effective=8, cpu_ram=32768, reliability=.99, inet_down=500)


class OfferAdmission(unittest.TestCase):
    def test_valid_offer(self):
        self.assertEqual(require_offer(offer(), set()), 5/6)

    def test_valid_second_country(self):
        data = offer(); data['geolocation'] = 'Spain, ES'
        self.assertEqual(require_offer(data, set()), 5/6)

    def bad(self, **changes):
        data = offer(); data.update(changes)
        with self.assertRaises(ValueError):
            require_offer(data, set())

    def test_unverified(self): self.bad(verification='unverified')
    def test_missing_verification(self): self.bad(verification=None)
    def test_vm_false(self): self.bad(vms_enabled=False)
    def test_vm_unknown(self): self.bad(is_vm_deverified=None)
    def test_gpu_wrong(self): self.bad(gpu_name='RTX 5070')
    def test_gpu_unknown(self): self.bad(gpu_name=None)
    def test_zero_gpu(self): self.bad(num_gpus=0)
    def test_two_gpu(self): self.bad(num_gpus=2)
    def test_denied_country(self): self.bad(geolocation='China, CN')
    def test_unknown_country(self): self.bad(geolocation='unknown')
    def test_machine_blacklist(self):
        with self.assertRaises(ValueError): require_offer(offer(), {3})
    def test_host_blacklist(self):
        with self.assertRaises(ValueError): require_offer(offer(), {4})
    def test_compute_too_high(self): self.bad(dph_base=1.1)
    def test_storage_too_high(self): self.bad(storage_cost=2)
    def test_bad_download_price(self): self.bad(inet_down_cost=math.nan)
    def test_bad_upload_price(self): self.bad(inet_up_cost=-1)
    def test_cpu_floor(self): self.bad(cpu_cores_effective=7)
    def test_ram_floor(self): self.bad(cpu_ram=16384)
    def test_disk_floor(self): self.bad(disk_space=179)
    def test_reliability_floor(self): self.bad(reliability=.94)


class Budget(unittest.TestCase):
    def plan(self, **kw):
        args = dict(hourly=5/6, credit=23, now=NOW, transfer=.11849,
                    deadline=END, estimated_seconds=4931, shared_remaining=2.0223078)
        args.update(kw)
        return fit_plan(**args)

    def test_fit(self):
        plan = self.plan()
        self.assertLessEqual(plan['seconds'], 6300)
        self.assertLess(plan['reservation_usd'], 2)
        self.assertLessEqual(plan['cap_usd'], 2.0223078)
        self.assertGreaterEqual(plan['seconds'], 4931)

    def test_expired(self):
        with self.assertRaises(ValueError): self.plan(deadline=NOW)
    def test_short_wall(self):
        with self.assertRaises(ValueError): self.plan(deadline=NOW+dt.timedelta(minutes=80))
    def test_cost_too_short(self):
        with self.assertRaises(ValueError): self.plan(transfer=.8)
    def test_estimate_too_long(self):
        with self.assertRaises(ValueError): self.plan(estimated_seconds=7000)
    def test_reserve_low(self):
        with self.assertRaises(ValueError): self.plan(credit=9)
    def test_reserve_exact(self):
        with self.assertRaises(ValueError): self.plan(credit=8)
    def test_shared_too_low(self):
        with self.assertRaises(ValueError): self.plan(shared_remaining=1.99)
    def test_shared_at_stop(self):
        with self.assertRaises(ValueError): self.plan(shared_remaining=2)
    def test_hourly_too_high(self):
        with self.assertRaises(ValueError): self.plan(hourly=1.01)
    def test_hourly_invalid(self):
        with self.assertRaises(ValueError): self.plan(hourly=math.nan)


class Wiring(unittest.TestCase):
    def test_parser_staged(self):
        self.assertIn('bw8_parser.ps1', wire_arm(ARM))
    def test_parser_gates_enable(self):
        data = wire_arm(ARM)
        self.assertLess(data.index('BW8_PARSER_GREEN=1'), data.index('enable_cmd='))
    def test_parser_error_receipt_preserved(self):
        self.assertIn('parser.txt', wire_arm(ARM))
    def test_exact_qemu_digest_gates_arm(self):
        self.assertIn('05d3d46f4e8429f5ffebacbd7ca9f5156f7d6f1141b4b10ce84732b174347448',
                      wire_controller(CONTROLLER))
    def test_qemu_digest_checked_before_plan_launch(self):
        data=wire_controller(CONTROLLER)
        self.assertLess(data.index('sha256'),data.index('mapfile -t plan'))
    def test_completion_overlay_selected(self):
        self.assertIn('arm_bw8.sh on', wire_controller(CONTROLLER))
    def test_no_collection_boot(self):
        self.assertNotIn('collect_bw.sh', wire_controller(CONTROLLER))
    def test_capture_preserved(self):
        self.assertIn('capture_pack.py', wire_controller(CONTROLLER))
    def test_missing_arm_anchor(self):
        with self.assertRaises(ValueError): wire_arm('enable_cmd="x"\n')
    def test_missing_parser_gate_anchor(self):
        with self.assertRaises(ValueError): wire_arm(ARM.replace('enable_cmd=', 'other='))
    def test_missing_controller_arm(self):
        with self.assertRaises(ValueError): wire_controller(CONTROLLER.replace('arm.sh', 'other.sh'))
    def test_missing_controller_collect(self):
        with self.assertRaises(ValueError): wire_controller(CONTROLLER.replace('collect_bw.sh', 'other.sh'))
    def test_ready_body_unchanged(self):
        self.assertIn('C.settle_pin', ready_body(READY))
    def test_ready_has_no_create(self):
        self.assertNotIn('create()', ready_body(READY))
    def test_ready_has_no_destroy(self):
        self.assertNotIn('destroy()', ready_body(READY))
    def test_missing_settled_admission(self):
        with self.assertRaises(ValueError): ready_body(READY.replace('C.settle_pin', 'C.wait_for_pin'))
    def test_removed_settled_admission(self):
        with self.assertRaises(ValueError): ready_body(READY.replace('C.settle_pin(rescan, console)', 'pass'))


if __name__ == '__main__':
    unittest.main()
