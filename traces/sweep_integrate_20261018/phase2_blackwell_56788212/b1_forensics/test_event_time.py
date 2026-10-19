import datetime,unittest
from event_time import event_time
class WindowsEventTime(unittest.TestCase):
 def test_windows_seven_digit_nonzero_fraction(self):
  self.assertEqual(event_time('2026-10-19T22:27:00.1234567Z'),datetime.datetime(2026,10,19,22,27,0,123456,tzinfo=datetime.timezone.utc))
 def test_windows_seven_digit_zero_fraction(self):
  self.assertEqual(event_time('2026-10-19T22:27:00.0000000Z'),datetime.datetime(2026,10,19,22,27,tzinfo=datetime.timezone.utc))
 def test_six_digit_fraction(self):
  self.assertEqual(event_time('2026-10-19T22:27:00.123456Z').microsecond,123456)
 def test_no_fraction(self):
  self.assertEqual(event_time('2026-10-19T22:27:00Z').minute,27)
 def test_invalid_calendar_is_not_coerced(self):
  with self.assertRaises(ValueError):event_time('2026-99-19T22:27:00.0000000Z')
if __name__=='__main__':unittest.main()
