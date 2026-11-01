"""Exercise the exact collector regex and capture selection with WER fixtures."""
from pathlib import Path
import re
import unittest

SOURCE = Path(__file__).with_name('wer_times.ps1')

def value(line):
    source = SOURCE.read_text()
    expression = re.search(r"if\(\$line -match '([^']+)'",source)[1]
    index = int(re.search(r'\$map\[\$matches\[1\]\]=\$matches\[(\d+)\]',source)[1])
    match = re.match(expression,line)
    return match[index]

class WerFields(unittest.TestCase):
    def test_event_time_preserves_the_entire_numeric_value(self):
        self.assertEqual(value('EventTime=134370000000000000'),'134370000000000000')

    def test_signature_value_preserves_code_instead_of_field_suffix(self):
        self.assertEqual(value('Sig[0].Value=141'),'141')

if __name__ == '__main__':
    unittest.main()
