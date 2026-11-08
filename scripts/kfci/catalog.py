"""Registry and small local dependency resolver; Worker assignments keep their order."""

from .assignment import identifier, number


class CatalogError(ValueError):
    pass


class Catalog:
    def __init__(self):
        self.steps = {}

    def register(self, step):
        if not identifier(step.id) or step.id in self.steps:
            raise CatalogError("invalid or duplicate step id: " + str(step.id))
        if not number(step.estimated_minutes) or not number(
            step.timeout_seconds, positive=True
        ):
            raise CatalogError("step must declare finite estimate and positive timeout")
        for names in (step.needs, step.produces):
            if not all(identifier(n) for n in names) or len(names) != len(set(names)):
                raise CatalogError("invalid or duplicate artifact name")
        self.steps[step.id] = step
        return step

    def get(self, name):
        if name not in self.steps:
            raise CatalogError("unknown step: " + str(name))
        return self.steps[name]

    def ordered(self, names, inputs=()):
        available, selected, seen = set(inputs), [], set()
        for name in names:
            step = self.get(name)
            if name in seen:
                raise CatalogError("duplicate step: " + name)
            missing = set(step.needs) - available
            if missing:
                raise CatalogError(
                    "step %s needs unavailable artifacts: %s"
                    % (name, ", ".join(sorted(missing)))
                )
            available.update(step.produces)
            selected.append(step)
            seen.add(name)
        return selected

    def assigned(self, entries):
        """Validate J2's scoped artifact order against registered action metadata."""
        available, selected = set(), []
        for entry in entries:
            step = self.get(entry["run_id"])
            needs = {name.split("@", 1)[0] for name in entry["needs"]}
            produces = {name.split("@", 1)[0] for name in entry["produces"]}
            if needs != set(step.needs) or produces != set(step.produces):
                raise CatalogError(
                    "assigned artifact contract differs for " + entry["id"]
                )
            missing = set(entry["needs"]) - available
            if missing:
                raise CatalogError(
                    "assigned step needs unavailable artifacts: "
                    + ", ".join(sorted(missing))
                )
            available.update(entry["produces"])
            selected.append(step)
        return selected

    def resolve(self, names, inputs=()):
        """Only local runs request dependencies. Ambiguity is never guessed away."""
        names = list(names)
        for name in names:
            self.get(name)
        available, visiting, done, order = set(inputs), set(), set(), []

        def visit(name):
            if name in visiting:
                raise CatalogError("dependency cycle at " + name)
            if name in done:
                return
            visiting.add(name)
            step = self.get(name)
            for need in step.needs:
                if need in available:
                    continue
                candidates = [s.id for s in self.steps.values() if need in s.produces]
                explicit = [n for n in names if n in candidates]
                choices = explicit if explicit else candidates
                if not choices:
                    raise CatalogError("no producer for " + need)
                if len(choices) != 1:
                    raise CatalogError(
                        "ambiguous producers for " + need + ": " + ", ".join(choices)
                    )
                visit(choices[0])
            visiting.remove(name)
            done.add(name)
            order.append(name)
            available.update(step.produces)

        for name in names:
            visit(name)
        return order
