"""A proposed publisher's obligations. This suite does not test production RUSD support."""
import struct
import unittest

from rusd_model import CompletionSample, Model, PERF, SIZE


def init_request(address=4096, enabled=1, padding=b"\xa5" * 7):
    return struct.pack("<B7sQ", enabled, padding, address)


def sample(vm="A", busy=25, period=100, timestamp=1000):
    return CompletionSample(vm, busy, period, timestamp, 0, ((0, period),) * 4)


class Contract(unittest.TestCase):
    def ready(self, vm="A", address=4096):
        model = Model(vm)
        self.assertTrue(model.initialize(init_request(address)))
        return model

    def test_init_registers_complete_guest_ram_span(self):
        model = self.ready()
        self.assertEqual(model.registration, (4096, SIZE, model.generation))
        self.assertEqual(model.writes, [])

    def test_second_vm_owns_its_registration(self):
        model = self.ready("B", 8192)
        self.assertEqual(model.registration, (8192, SIZE, model.generation))
        self.assertEqual(model.writes, [])

    def test_real_vm_sample_publishes_with_a_valid_timestamp(self):
        model = self.ready()
        self.assertTrue(model.poll(2))
        self.assertTrue(model.publish(sample()))
        self.assertEqual(model.read_perf(), (1000, 25))

    def test_real_vm_sample_with_other_load_is_not_a_constant(self):
        model = self.ready("B")
        self.assertTrue(model.poll(2))
        self.assertTrue(model.publish(sample("B", 75, 100, 2000)))
        self.assertEqual(model.read_perf(), (2000, 75))

    def test_no_poll_subscription_means_no_publication(self):
        model = self.ready()
        self.assertFalse(model.publish(sample()))
        self.assertEqual(model.writes, [])
        self.assertIsNone(model.read_perf())

    def test_unsupported_poll_bits_are_atomic_refusals(self):
        model = self.ready()
        self.assertTrue(model.poll(2))
        for mask in [1, 3, 4, 8, 16, 32, 64, 1 << 63]:
            self.assertFalse(model.poll(mask))
            self.assertEqual(model.mask, 2)

    def test_zero_poll_mask_stops_refresh(self):
        model = self.ready()
        self.assertTrue(model.poll(2))
        self.assertTrue(model.poll(0))
        self.assertFalse(model.publish(sample()))
        self.assertEqual(model.writes, [])

    def test_a_host_wide_sample_never_reaches_guest_A(self):
        model = self.ready()
        self.assertTrue(model.poll(2))
        self.assertFalse(model.publish(sample("HOST")))
        self.assertEqual(model.writes, [])

    def test_guest_B_cannot_supply_guest_A_statistics(self):
        model = self.ready()
        self.assertTrue(model.poll(2))
        self.assertFalse(model.publish(sample("B")))
        self.assertEqual(model.writes, [])

    def test_destroy_retires_region_before_success(self):
        model = self.ready()
        self.assertTrue(model.poll(2))
        self.assertTrue(model.destroy())
        self.assertIsNone(model.registration)
        self.assertFalse(model.publish(sample()))
        self.assertEqual(model.writes, [])

    def test_wire_destroy_does_not_register_zero(self):
        model = self.ready()
        self.assertTrue(model.initialize(init_request(0, 0)))
        self.assertIsNone(model.registration)
        self.assertFalse(model.publish(sample()))

    def test_reset_revokes_old_writer_generation(self):
        model = self.ready()
        old = model.generation
        model.reset()
        self.assertTrue(model.initialize(init_request(8192)))
        self.assertTrue(model.poll(2))
        self.assertFalse(model.publish(sample(), old))
        self.assertEqual(model.writes, [])

    def test_reinitialize_revokes_old_writer_generation(self):
        model = self.ready()
        old = model.generation
        self.assertTrue(model.initialize(init_request(8192)))
        self.assertTrue(model.poll(2))
        self.assertFalse(model.publish(sample(), old))
        self.assertEqual(model.writes, [])

    def test_bad_address_never_creates_registration(self):
        for address in [0, 1, 4097, 16384 - SIZE + 1, 16384, (1 << 64) - 4096]:
            model = Model("A")
            self.assertFalse(model.initialize(init_request(address)))
            self.assertIsNone(model.registration)
            self.assertEqual(model.writes, [])

    def test_malformed_request_never_creates_registration(self):
        for request in [b"", bytes(8), bytes(15), bytes(17), init_request(enabled=2)]:
            model = Model("A")
            self.assertFalse(model.initialize(request))
            self.assertIsNone(model.registration)
            self.assertEqual(model.writes, [])

    def test_bad_reinit_keeps_existing_registration_and_data(self):
        model = self.ready()
        before = model.registration
        self.assertFalse(model.initialize(init_request((1 << 64) - 4096)))
        self.assertEqual(model.registration, before)
        self.assertEqual(model.writes, [])

    def test_incoherent_completion_samples_are_not_published(self):
        model = self.ready()
        self.assertTrue(model.poll(2))
        for value in [sample(busy=-1), sample(busy=101), sample(period=0), sample(timestamp=0)]:
            self.assertFalse(model.publish(value))
            self.assertEqual(model.writes, [])

    def test_torn_snapshot_with_newer_timestamp_is_rejected(self):
        model = self.ready()
        self.assertTrue(model.poll(2))
        self.assertTrue(model.publish(sample()))
        self.assertIsNone(model.read_perf(after_timestamp=1001))

    def test_publication_stays_inside_validated_region(self):
        model = self.ready()
        self.assertTrue(model.poll(2))
        model.ram[:4096] = b"\xa7" * 4096
        model.ram[4096 + SIZE:] = b"\xb8" * (len(model.ram) - 4096 - SIZE)
        self.assertTrue(model.publish(sample()))
        self.assertEqual(model.ram[:4096], b"\xa7" * 4096)
        self.assertEqual(model.ram[4096 + SIZE:], b"\xb8" * (len(model.ram) - 4096 - SIZE))
        self.assertTrue(all(4096 <= at < 4096 + SIZE for at, _ in model.writes))

    def test_unimplemented_memory_and_video_metrics_stay_invalid(self):
        model = self.ready()
        self.assertTrue(model.poll(2))
        incomplete = CompletionSample("A", 25, 100, 1000, None, None)
        self.assertFalse(model.publish(incomplete))
        self.assertIsNone(model.read_perf())
        self.assertEqual(model.writes, [])


if __name__ == "__main__":
    unittest.main()
