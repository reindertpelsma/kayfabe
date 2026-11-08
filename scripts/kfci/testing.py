"""Deterministic fakes for stage authors. Never selected by the production catalog."""

from .step import Step, StepResult, Verdict


class FakeClock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def advance(self, seconds):
        self.now += seconds


class FakeStep(Step):
    def __init__(
        self,
        id,
        *,
        needs=(),
        produces=(),
        estimated_minutes=1,
        timeout_seconds=600,
        result=None,
        action=None
    ):
        self.id, self.needs, self.produces = id, needs, produces
        self.estimated_minutes, self.timeout_seconds = (
            estimated_minutes,
            timeout_seconds,
        )
        self.result = result if result is not None else StepResult(Verdict.PASS)
        self.action = action
        self.calls = []

    def run(self, ctx):
        self.calls.append(ctx)
        if self.action:
            self.action(ctx)
        return self.result


class FakeProcess:
    """Callable replacement for proc.run; fails if a transcript is exhausted."""

    def __init__(self, results):
        self.results = iter(results)
        self.calls = []

    def __call__(self, argv, **kwargs):
        self.calls.append((list(argv), kwargs))
        return next(self.results)
