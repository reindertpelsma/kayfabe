"""Design-only RUSD model; no device, host RM, transport or runtime consumer.

The initial refusal models the current production contract. Positive tests specify
future publication obligations, not an instruction to return success without them.
Public layout: ogkm 580.65.06 cl00de.h and ctrl2080internal.h, not a capture.
"""
from dataclasses import dataclass

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
        return False

    def poll(self, mask):
        return False

    def publish(self, sample, generation=None):
        return False

    def destroy(self):
        self.registration = None
        self.mask = 0
        return True

    def reset(self):
        self.destroy()
        self.generation += 1

    def read_perf(self):
        return None
