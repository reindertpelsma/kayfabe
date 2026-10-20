import importlib.util
from pathlib import Path
import unittest


def record(name, data):
    name = name.encode() + b"\0"
    fields = [1, 0o100755, 0, 0, 1, 0, len(data), 0, 0, 0, 0, len(name), 0]
    header = b"070701" + b"".join(f"{n:08x}".encode() for n in fields)
    return header + name + b"\0" * (-(110 + len(name)) % 4) + data + b"\0" * (-len(data) % 4)


class ReplaceClient(unittest.TestCase):
    def check_payload(self, new):
        spec = importlib.util.spec_from_file_location("replace_initrd_client", Path(__file__).with_name("replace_initrd_client.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        prefix = record("init", b"unmodified init")
        suffix = record("lib/libcuda.so", b"unmodified driver") + record("TRAILER!!!", b"") + b"\0" * 512
        old = prefix + record("bin/kftool", b"old client") + suffix
        self.assertEqual(mod.replace_client(old, new), prefix + record("bin/kftool", new) + suffix)

    def test_larger_client_preserves_neighbors_and_header(self):
        self.check_payload(b"new client" * 31)

    def test_smaller_client_preserves_neighbors_and_padding(self):
        self.check_payload(b"new")


if __name__ == "__main__":
    unittest.main()
