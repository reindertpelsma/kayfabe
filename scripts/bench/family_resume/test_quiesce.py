"""Stop only a pinned controller and its authenticated descendant snapshot."""
import signal
import unittest

import quiesce as Q


class QuiesceTests(unittest.TestCase):
    def test_pid_reuse_refuses_before_any_signal(self):
        signals = []
        with self.assertRaises(ValueError):
            Q.quiesce(10, 100, snapshot=lambda: {10: (1, 101)},
                      send=lambda *args: signals.append(args), pause=lambda _: None)
        self.assertEqual(signals, [])

    def test_unrelated_process_is_never_signalled(self):
        signals = []
        tables = [{10: (1, 100), 11: (10, 200), 12: (11, 300), 99: (1, 400)}]
        def send(pid, sig):
            signals.append((pid, sig))
            if sig == signal.SIGKILL:
                tables[0].pop(pid, None)
        Q.quiesce(10, 100, snapshot=lambda: dict(tables[0]), send=send, pause=lambda _: None)
        self.assertEqual({pid for pid, _ in signals}, {10, 11, 12})
        self.assertEqual(signals[0], (10, signal.SIGSTOP))

    def test_reused_child_is_not_signalled_again(self):
        signals = []
        table = {10: (1, 100), 11: (10, 200)}
        def send(pid, sig):
            signals.append((pid, sig))
            if (pid, sig) == (11, signal.SIGSTOP):
                table[11] = (1, 201)
            if sig == signal.SIGKILL:
                table.pop(pid, None)
        Q.quiesce(10, 100, snapshot=lambda: dict(table), send=send, pause=lambda _: None)
        self.assertEqual([sig for pid, sig in signals if pid == 11], [signal.SIGSTOP])

    def test_already_exited_controller_requires_no_signals(self):
        signals = []
        Q.quiesce(10, 100, snapshot=lambda: {99: (1, 400)},
                  send=lambda *args: signals.append(args), pause=lambda _: None)
        self.assertEqual(signals, [])


if __name__ == "__main__":
    unittest.main()
