import importlib.util
from pathlib import Path
import struct
import unittest


def load():
    spec = importlib.util.spec_from_file_location("probe", Path(__file__).with_name("translated_probe.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def birth(key="0x1:0x2", token="0x3", host="0x4", aperture="Sysmem"):
    return (f"kf3: act birth translated: chan {key} BORN Translated: token {token} -> host {host} "
            f"in VasKey(7) gpfifo=0x2000x16 userd=Some({aperture} {{ base: 4096, size: 512 }}) "
            "engine=0xb tsg=None kernel_by=PRIVILEGE=KERNEL ring_va=0xff00000000")


class TranslatedProbe(unittest.TestCase):
    def test_sysmem_reads_declared_guest_userd_and_reports_pending(self):
        state = load().TranslatedLog()
        state.line(birth())
        reads = []
        def read(at, n):
            reads.append((at, n))
            return struct.pack("<II", 2, 7)
        rows = state.sample(read)
        self.assertEqual(len(rows), 1)
        self.assertEqual(reads, [(4096 + 0x88, 8)])
        self.assertEqual((rows[0]["get"], rows[0]["put"], rows[0]["delta"]), (2, 7, 5))
        self.assertEqual(rows[0]["engine"], 11)

    def test_framebuffer_is_unobserved_and_exact_retirement_removes_channel(self):
        state = load().TranslatedLog()
        state.line(birth(aperture="Framebuffer"))
        rows = state.sample(lambda *_: self.fail("no guessed framebuffer mapping"))
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["status"], "unobserved_framebuffer")
        state.line("kf3: TSPACE-RETIRE tok=0x3 host=0x4 key=VasKey(7)")
        self.assertEqual(state.sample(lambda *_: self.fail("retired channel")), [])

    def test_old_retirement_cannot_remove_replacement(self):
        state = load().TranslatedLog()
        state.line(birth())
        state.line(birth(token="0x5"))
        state.line("kf3: TSPACE-RETIRE tok=0x3 host=0x4 key=VasKey(7)")
        rows = state.sample(lambda *_: struct.pack("<II", 15, 2))
        self.assertEqual(len(rows), 1)
        self.assertEqual((rows[0]["token"], rows[0]["delta"]), (5, 3))

    def test_unreadable_pair_stays_unknown(self):
        state = load().TranslatedLog()
        state.line(birth())
        rows = state.sample(lambda *_: None)
        self.assertEqual(rows[0]["status"], "unreadable")
        self.assertNotIn("put", rows[0])


if __name__ == "__main__":
    unittest.main()
