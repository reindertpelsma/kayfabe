"""Coordinator admission contract, candidate 6564dd597; no hardware access."""
import unittest
import fault_admission as policy

VAS = (0x6000010000, 0x6200010000, 0x6400010000)
PID = 4321


def record(va=VAS[0], pid=PID, xid=31, engine="CE0", client="HUBCLIENT_CE1",
           kind="FAULT_PTE", access="VIRT_READ", channel="0x0000001d"):
    return (f"NVRM: Xid (PCI:0000:01:00): {xid}, pid={pid}, name=qemu-system-x86, "
            f"channel {channel}, intr 00000000. MMU Fault: ENGINE {engine} {client} "
            f"faulted @ {va:#x}. Fault is of type {kind} ACCESS_TYPE_{access}")


def markers(vas=VAS, name="defer_liveness_prime"):
    return "\n".join(f"FAULT_MARK={name} arm=fixture va={va:#x}" for va in vas)


def evaluate(lines, **changes):
    args = dict(arm="defer-liveness", pid=PID, allowed_vas=VAS,
                prime_limit=3, serial=markers())
    args.update(changes)
    return policy.audit(lines, **args)


class AdmissionTests(unittest.TestCase):
    def test_empty_kernel_is_accepted(self):
        self.assertEqual(evaluate([]), [])

    def test_own_read_prime_is_accepted(self):
        self.assertEqual(evaluate([record()]), [])

    def test_own_write_prime_is_accepted(self):
        self.assertEqual(evaluate([record(access="VIRT_WRITE")]), [])

    def test_three_distinct_primes_are_accepted(self):
        self.assertEqual(evaluate([record(va=va) for va in VAS]), [])

    def test_repeat_va_is_one_prime(self):
        self.assertEqual(evaluate([record(va=va) for va in (*VAS, VAS[1])]), [])

    def test_repeat_va_on_another_channel_is_one_prime(self):
        self.assertEqual(evaluate([record(), record(channel="0x0000001e")]), [])

    def test_real_kernel_split_hex_va_is_accepted(self):
        self.assertEqual(evaluate([record().replace("0x6000010000", "0x60_00010000")]), [])

    def test_budget_exit_is_124(self):
        self.assertEqual(policy.exit_code("budget", 0), 124)

    def test_normal_exit_is_preserved(self):
        self.assertEqual(policy.exit_code(None, 7), 7)

    def test_host_fault_exit_is_distinct(self):
        self.assertEqual(policy.exit_code("host-fault", 0), 3)

    def test_audit_failure_exit_is_distinct(self):
        self.assertEqual(policy.exit_code("audit-error", 0), 3)


def reject_case(name, lines, **changes):
    def test(self):
        self.assertTrue(evaluate(lines, **changes), name)
    setattr(AdmissionTests, "test_reject_" + name, test)


for name, lines, changes in [
    ("other_pid", [record(pid=999)], {}),
    ("zero_pid", [record(pid=0)], {}),
    ("missing_pid", [record().replace(f"pid={PID}, ", "")], {}),
    ("unparsed_xid", ["NVRM: Xid: malformed"], {}),
    ("xid13", [record(xid=13)], {}),
    ("xid69", [record(xid=69)], {}),
    ("graphics_engine", [record(engine="GRAPHICS")], {}),
    ("different_ce_engine", [record(engine="CE1")], {}),
    ("different_client", [record(client="HUBCLIENT_ESC")], {}),
    ("missing_client", [record(client="")], {}),
    ("wrong_ce_client", [record(client="HUBCLIENT_CE0")], {}),
    ("pde_fault", [record(kind="FAULT_PDE")], {}),
    ("permission_fault", [record(kind="FAULT_PRIV_VIOLATION")], {}),
    ("physical_read", [record(access="PHYS_READ")], {}),
    ("unknown_access", [record(access="VIRT_ATOMIC")], {}),
    ("unnamed_va", [record(va=0x6600010000)], {}),
    ("adjacent_va", [record(va=VAS[0]+4)], {}),
    ("declared_but_not_preregistered", [record()], {"allowed_vas": VAS[1:]}),
    ("second_declared_not_preregistered", [record(va=VAS[1])],
     {"allowed_vas": (VAS[0], VAS[2])}),
    ("empty_marker", [record()], {"serial": ""}),
    ("wrong_marker", [record()], {"serial": markers(name="other_prime")}),
    ("too_many_for_one_prime", [record(va=va) for va in VAS[:2]], {"prime_limit": 1}),
    ("too_many_for_two_primes", [record(va=va) for va in VAS], {"prime_limit": 2}),
    ("noninducing_timer", [record()], {"arm": "timer"}),
    ("noninducing_concurrency", [record()], {"arm": "concurrency"}),
    ("oom", ["Out of memory: cannot allocate"], {}),
    ("oom_kill", ["oom-kill: task=qemu"], {}),
    ("killed_process", ["Killed process 4321 (qemu)"], {}),
]:
    reject_case(name, lines, **changes)


if __name__ == "__main__":
    unittest.main()
