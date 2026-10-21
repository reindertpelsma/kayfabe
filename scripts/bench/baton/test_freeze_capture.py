import importlib.util
from pathlib import Path
import unittest


class TriggerInput(unittest.TestCase):
    def parse(self, text):
        spec = importlib.util.spec_from_file_location("host_freeze_capture", Path(__file__).with_name("host_freeze_capture.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod.parse_trigger(text)

    def test_heaven_trigger_preserves_app_and_gap(self):
        self.assertEqual(self.parse("02:34:28.390 TRIGGER Heaven.exe silent 4s after 3194 presents/30s"), ("Heaven.exe", 4.0))

    def test_desktop_trigger_is_also_captured(self):
        self.assertEqual(self.parse("02:34:50.757 TRIGGER dwm.exe silent 5.2s after 1637 presents/30s"), ("dwm.exe", 5.2))

    def test_nontrigger_and_short_gap_are_ignored(self):
        for text in ["START qpc=193936754", "TRIGGER Heaven.exe silent 1s", "x" * 5000, "TRIGGER bad\0name silent 4s"]:
            self.assertIsNone(self.parse(text))


if __name__ == "__main__":
    unittest.main()
