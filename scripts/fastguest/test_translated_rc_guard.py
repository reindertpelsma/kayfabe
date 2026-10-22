#!/usr/bin/env python3
"""Synthetic captures test the evidence gate; these are not hardware evidence."""
import json
import unittest
from translated_rc_guard import Refused, check


def fixture():
    manifest = dict(run="linux-rc-1", mode="on", source="a" * 40, pid=123,
                    pci="0000:01:00", start_ns=1000000000, end_ns=3000000000,
                    detail="intr 00000010")
    qemu = """RC_NEGCTL_CONFIG run=linux-rc-1 source=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
chan 0xc1e00001:0x42 BORN Translated: token 0x5 -> host 0x19 in VasKey gpfifo=0x10000x64 userd=Sysmem engine=0x9
FAULT_MARK=translated_rc_invalid_pb run=linux-rc-1 pid=123 host=0x19 channel=0x19 guest=0x5 engine=0x9 word=0xc0000000 unix_ns=2000000000
RC_NEGCTL_SUBMITTED run=linux-rc-1 host=0x19
kf3: RC Translated host 0x19 guest chid 0x5 engine 0x9 Xid 32 — channel in error
kf3: RC_TRIGGERED posted: guest chid 0x5 engine 0x9 except_type 0x20 (host 0x19)
"""
    guest = """KF_RC_PROBE_BEGIN run=linux-rc-1 mode=on chid=0x5 engine=0x9 class=0xc56f
KF_RC_BYSTANDER run=linux-rc-1 phase=before value=1
KF_RC_EVENT chid=0x5 engine=0x9 xid=32 scope=0
KF_RC_NOTIFIER run=linux-rc-1 chid=0x5 timestamp=0x1234 info32=32 info16=9 status=0xffff get=0
KF_RC_BYSTANDER run=linux-rc-1 phase=after value=2
KF_RC_PROBE_END run=linux-rc-1 result=PASS
"""
    journal = json.dumps({"__REALTIME_TIMESTAMP": "2100000", "MESSAGE": "NVRM: Xid (PCI:0000:01:00): 32, pid=123, name=qemu-system-x86, channel 0x00000019 intr 00000010"})+"\n"
    return manifest, qemu, guest, journal


class GuardTests(unittest.TestCase):
    def reject(self, *, m=None, q=None, g=None, j=None):
        a,b,c,d=fixture()
        with self.assertRaises(Refused): check(a if m is None else m,b if q is None else q,c if g is None else g,d if j is None else j)

    def test_exact_control_accepts(self): self.assertEqual(check(*fixture())["verdict"],"PASS")
    def test_capture_with_noise_accepts(self):
        m,q,g,j=fixture();self.assertEqual(check(m,"boot\n"+q,g+"poweroff\n",j)["verdict"],"PASS")
    def test_no_notifier_rejected(self): self.reject(g=fixture()[2].replace("KF_RC_NOTIFIER", "NO_NOTIFIER"))
    def test_no_guest_event_rejected(self): self.reject(g=fixture()[2].replace("KF_RC_EVENT", "NO_EVENT"))
    def test_host_post_alone_cannot_prove_guest_receipt(self): self.reject(g=fixture()[2].replace("KF_RC_EVENT", "NO_EVENT").replace("KF_RC_NOTIFIER", "NO_NOTIFIER"))
    def test_duplicate_guest_events_rejected(self): self.reject(g=fixture()[2]+"KF_RC_EVENT chid=0x5 engine=0x9 xid=32 scope=0\n")
    def test_wrong_guest_chid_rejected(self): self.reject(g=fixture()[2].replace("KF_RC_EVENT chid=0x5", "KF_RC_EVENT chid=0x6"))
    def test_wrong_engine_rejected(self): self.reject(g=fixture()[2].replace("info16=9", "info16=10"))
    def test_failed_work_did_not_retire(self): self.reject(g=fixture()[2].replace("get=0", "get=1"))
    def test_missing_get_evidence_rejected(self): self.reject(g=fixture()[2].replace(" get=0", ""))
    def test_incomplete_notifier_rejected(self): self.reject(g=fixture()[2].replace("status=0xffff", "status=0x0"))
    def test_wrong_xid_in_notifier_rejected(self): self.reject(g=fixture()[2].replace("info32=32", "info32=45"))
    def test_passthrough_does_not_qualify(self): self.reject(q=fixture()[1].replace("BORN Translated", "BORN Passthrough"))
    def test_wrong_twin_mapping_rejected(self): self.reject(q=fixture()[1].replace("-> host 0x19", "-> host 0x20"))
    def test_missing_fault_marker_rejected(self): self.reject(q=fixture()[1].replace("FAULT_MARK=", "NO_MARK="))
    def test_duplicate_fault_marker_rejected(self): self.reject(q=fixture()[1]+fixture()[1].splitlines()[2]+"\n")
    def test_missing_native_submission_rejected(self): self.reject(q=fixture()[1].replace("RC_NEGCTL_SUBMITTED", "NO_SUBMIT"))
    def test_wrong_source_rejected(self): self.reject(q=fixture()[1].replace("source="+"a"*40, "source="+"b"*40))
    def test_wrong_host_pid_rejected(self): self.reject(j=fixture()[3].replace("pid=123", "pid=124"))
    def test_wrong_host_channel_rejected(self): self.reject(j=fixture()[3].replace("channel 0x00000019", "channel 0x00000020"))
    def test_wrong_host_signature_rejected(self): self.reject(j=fixture()[3].replace("intr 00000010", "intr 00200000"))
    def test_unrelated_xid_rejected(self): self.reject(j=fixture()[3]+fixture()[3].replace(": 32,", ": 31,"))
    def test_duplicate_host_xid_rejected(self): self.reject(j=fixture()[3]*2)
    def test_missing_host_xid_rejected(self): self.reject(j="")
    def test_fault_outside_interval_rejected(self): self.reject(j=fixture()[3].replace("2100000", "4000000"))
    def test_fault_preceding_marker_rejected(self): self.reject(j=fixture()[3].replace("2100000", "1900000"))
    def test_bystander_missing_rejected(self): self.reject(g=fixture()[2].replace("phase=after", "phase=missing"))
    def test_bystander_did_not_progress_rejected(self): self.reject(g=fixture()[2].replace("phase=after value=2", "phase=after value=1"))
    def test_guest_failure_rejected(self): self.reject(g=fixture()[2].replace("result=PASS", "result=FAIL"))
    def test_oversized_input_rejected(self): self.reject(q="x"*(2*1024*1024+1))
    def test_malformed_journal_rejected(self): self.reject(j="not json\n")
    def test_off_control_accepts(self):
        m,q,g,j=fixture();m["mode"]="off";q=q.split("FAULT_MARK")[0];g=g.replace("mode=on","mode=off");g="\n".join(l for l in g.splitlines() if "KF_RC_EVENT" not in l and "KF_RC_NOTIFIER" not in l)+"\nKF_RC_CLEAN run=linux-rc-1 status=0x0\n";self.assertEqual(check(m,q,g,"")["verdict"],"PASS")


if __name__ == "__main__": unittest.main()
