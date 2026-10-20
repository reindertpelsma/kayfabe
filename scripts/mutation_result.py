#!/usr/bin/env python3
"""Classify a completed cargo test run without counting compiler failures as kills."""
import re


def classify(returncode, output, minimum=2):
    """Only completed, named test failures count; build/harness faults never count."""
    tests = re.findall(r'^test (.+) \.\.\. FAILED$', output, re.MULTILINE)
    completed = bool(re.search(r'^test result: (?:ok|FAILED)\.', output, re.MULTILINE))
    errors = re.findall(r'^error[^\n]*', output, re.MULTILINE)
    unexpected = [line for line in errors if not line.startswith('error: test failed, to rerun pass ')]
    fault = bool(unexpected) or (returncode != 0 and not completed)
    status = 'fault' if fault else ('killed' if returncode != 0 and len(tests) >= minimum else 'survived')
    return {'status': status, 'kills': len(tests), 'tests': tests, 'errors': unexpected}
