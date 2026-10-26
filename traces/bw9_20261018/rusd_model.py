"""Design-only RUSD model; no device, host RM, transport or runtime consumer.

The initial refusal models the current production contract. Positive tests specify
future publication obligations, not an instruction to return success without them.
Public layout: ogkm 580.65.06 cl00de.h and ctrl2080internal.h, not a capture.
"""
from dataclasses import dataclass
import struct

SIZE = 2600
PERF = 1976
POLL_PERF = 2


@dataclass(frozen=True)
class CompletionSample:
    vm: str
    busy_us: int
    period_us: int
    guest_timestamp_ns: int
    memory_busy_us: int | None
    video: tuple[tuple[int, int], ...] | None


class Model:
    def __init__(self, vm, ram_size=16384):
        self.vm = vm
        self.ram = bytearray(ram_size)
        self.generation = 1
        self.registration = None
        self.mask = 0
        self.writes = []

    def initialize(self, request):
        if len(request) != 16:
            return False
        enabled, address = struct.unpack("<B7xQ", request)
        if enabled == 0 and address == 0:
            return self.destroy()
        if enabled != 1 or address == 0 or address % 4096:
            return False
        if address > len(self.ram) or SIZE > len(self.ram) - address:
            return False
        self.generation += 1
        self.registration = (address, SIZE, self.generation)
        self.mask = 0
        return True

    def poll(self, mask):
        if self.registration is None or mask not in (0, POLL_PERF):
            return False
        self.mask = mask
        return True

    def publish(self, sample, generation=None):
        if self.registration is None or self.mask != POLL_PERF:
            return False
        if generation is not None and generation != self.generation:
            return False
        if sample.vm != self.vm:
            return False
        if sample.memory_busy_us is None or sample.video is None or len(sample.video) != 4:
            return False
        if not (0 < sample.guest_timestamp_ns < 0xFF00000000000000):
            return False
        if not 0 <= sample.busy_us <= sample.period_us or sample.period_us <= 0:
            return False
        if not 0 <= sample.memory_busy_us <= sample.period_us:
            return False
        if any(not 0 <= busy <= period or not 0 < period <= 0xFFFFFFFF
               for busy, period in sample.video):
            return False
        words = [sample.busy_us * 100 // sample.period_us,
                 sample.memory_busy_us * 100 // sample.period_us]
        for busy, period in sample.video:
            words.extend((busy * 100 // period, period))
        at = self.registration[0] + PERF
        # Simulation of the public timestamp protocol; no actual shared-memory atomics.
        self._write(at, struct.pack("<Q", (1 << 64) - 1))
        self._write(at + 8, struct.pack("<10I", *words))
        self._write(at, struct.pack("<Q", sample.guest_timestamp_ns))
        return True

    def _write(self, at, data):
        base, size, generation = self.registration
        assert generation == self.generation
        assert base <= at and at + len(data) <= base + size
        self.ram[at:at + len(data)] = data
        self.writes.append((at, len(data)))

    def destroy(self):
        self.registration = None
        self.mask = 0
        self.generation += 1
        return True

    def reset(self):
        self.destroy()
        self.generation += 1

    def read_perf(self, after_timestamp=None):
        if self.registration is None:
            return None
        at = self.registration[0] + PERF
        before, busy = struct.unpack_from("<QI", self.ram, at)
        if before == 0 or before >= 0xFF00000000000000:
            return None
        after = struct.unpack_from("<Q", self.ram, at)[0]
        if after_timestamp is not None:
            after = after_timestamp
        return (before, busy) if before == after else None
