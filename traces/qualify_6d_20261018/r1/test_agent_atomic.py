"""Deterministic publication controls for the actual lab-agent POST handler."""
import argparse
import io
import json
import os
from pathlib import Path
import tempfile
import threading
import unittest

parser = argparse.ArgumentParser(add_help=False)
parser.add_argument('--agent-source', required=True, type=Path)
args, remaining = parser.parse_known_args()
SOURCE = args.agent_source.read_text().split('\nif len(sys.argv)', 1)[0]


class BodyGate:
    def __init__(self, data):
        self.data = data
        self.read_started = threading.Event()
        self.release = threading.Event()

    def read(self, length):
        self.read_started.set()
        if not self.release.wait(5):
            raise RuntimeError('CONTROL_GATE_NOT_RELEASED')
        return self.data[:length]


class AtomicPublication(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        os.environ['Q4_AGENT_STATE'] = self.directory.name
        namespace = {'__name__': 'q4_agent_control'}
        exec(compile(SOURCE, 'lab-agent-under-test.py', 'exec'), namespace)
        self.destination = Path(namespace['R']) / 'control.txt'
        self.body = BodyGate(b'{"complete":true,"payload":"hello"}')
        self.handler = namespace['H'].__new__(namespace['H'])
        self.handler.path = '/result/control'
        self.handler.headers = {'Content-Length': str(len(self.body.data))}
        self.handler.rfile = self.body
        self.handler.wfile = io.BytesIO()
        self.handler.send_response = lambda *a: None
        self.handler.send_header = lambda *a: None
        self.handler.end_headers = lambda: None
        self.errors = []
        self.thread = None

    def start_writer(self):
        def invoke():
            try:
                self.handler.do_POST()
            except Exception as error:
                self.errors.append(error)
        self.thread = threading.Thread(target=invoke)
        self.thread.start()
        self.assertTrue(self.body.read_started.wait(5), 'HANDLER_DID_NOT_REACH_READ')

    def tearDown(self):
        self.body.release.set()
        if self.thread:
            self.thread.join(5)
            self.assertFalse(self.thread.is_alive(), 'HANDLER_DID_NOT_FINISH')
        self.directory.cleanup()

    def test_new_result_is_invisible_until_body_is_complete(self):
        self.start_writer()
        self.assertFalse(self.destination.exists(), 'EARLY_EMPTY_RESULT_PUBLICATION')

    def test_previous_complete_result_survives_until_replacement_is_complete(self):
        previous = b'{"complete":"previous"}'
        self.destination.write_bytes(previous)
        self.start_writer()
        self.assertEqual(self.destination.read_bytes(), previous,
                         'PREVIOUS_COMPLETE_RESULT_TRUNCATED_EARLY')

    def test_completed_result_is_published_as_valid_json(self):
        self.start_writer()
        self.body.release.set()
        self.thread.join(5)
        self.assertFalse(self.errors)
        self.assertEqual(json.loads(self.destination.read_text()),
                         {'complete': True, 'payload': 'hello'})


if __name__ == '__main__':
    unittest.main(argv=['test_agent_atomic.py', *remaining])
