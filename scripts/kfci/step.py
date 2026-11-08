"""Step metadata, typed outcomes and an execution context with a monotonic deadline."""

from abc import ABC, abstractmethod
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
import time

from . import proc


class Cancelled(RuntimeError):
    """Supervisor cancellation; distinct from InterruptedError swallowed by selectors."""


class Verdict(str, Enum):
    PASS = "PASS"
    FAIL = "FAIL"
    RETRY = "RETRY"


class RetryClass(str, Enum):
    TIME = "TIME"
    OPERATOR = "OPERATOR"
    PROVIDER = "PROVIDER"
    MEDIA = "MEDIA"
    TRANSPORT = "TRANSPORT"
    HOSTREF = "HOSTREF"
    BAR1_EXCEPTION = "BAR1_EXCEPTION"
    NOT_BUILT = "NOT_BUILT"
    NO_LIST = "NO_LIST"
    BAD_LIST = "BAD_LIST"
    BOX = "BOX"
    USAGE = "USAGE"


@dataclass(frozen=True)
class StepResult:
    verdict: Verdict
    retry_class: RetryClass | None = None

    def __post_init__(self):
        if not isinstance(self.verdict, Verdict):
            raise ValueError("verdict must be a Verdict")
        if self.verdict is Verdict.RETRY:
            if not isinstance(self.retry_class, RetryClass):
                raise ValueError("RETRY requires a RetryClass")
        elif self.retry_class is not None:
            raise ValueError("only RETRY can carry a class")

    @classmethod
    def retry(cls, reason):
        return cls(Verdict.RETRY, reason)

    @property
    def rc(self):
        if self.retry_class is RetryClass.USAGE:
            return 2
        return {Verdict.PASS: 0, Verdict.FAIL: 1, Verdict.RETRY: 3}[self.verdict]


@dataclass
class Context:
    parameters: dict
    env: dict
    output: object
    deadline: float
    clock: object = time.monotonic
    root: Path = Path(__file__).resolve().parents[2]

    assignment_id: str | None = None
    instance_id: str | None = None
    drivers: dict | None = None

    def remaining(self):
        return max(0.0, self.deadline - self.clock())

    def emit(self, line):
        if not isinstance(line, str) or "\n" in line or "\r" in line:
            raise ValueError("structured output must be one line")
        self.output.write(line + "\n")
        self.output.flush()

    def process(self, argv, *, timeout, **kwargs):
        remaining = self.remaining()
        if remaining <= 0:
            raise TimeoutError("step deadline reached")
        return proc.run(argv, timeout=min(timeout, remaining), env=self.env, **kwargs)


class Step(ABC):
    """A registered action. Estimates guard admission; timeouts bound subprocesses.

    needs/produces are artifact names, not shell commands. A successful producer
    makes those names available to later steps. A step must use ctx.process for
    external work and return a StepResult; it must not retry itself into a PASS.
    """

    id: str
    needs: tuple[str, ...] = ()
    produces: tuple[str, ...] = ()
    estimated_minutes: float = 1
    timeout_seconds: float = 60

    @abstractmethod
    def run(self, ctx) -> StepResult:
        raise NotImplementedError
