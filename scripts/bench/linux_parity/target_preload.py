"""Place measurement injection at the intended executable, after affinity wrappers."""


def target_preload(argv, environment, target_index, assignments):
    if not 0 <= target_index < len(argv):
        raise ValueError("missing target executable")
    clean = dict(environment)
    for key in assignments:
        if not key.replace("_", "a").isalnum() or key[0].isdigit():
            raise ValueError("invalid environment variable name")
        clean.pop(key, None)
    args = list(argv[:target_index]) + ["env", "--"]
    args += [key + "=" + str(value) for key, value in sorted(assignments.items())]
    args += list(argv[target_index:])
    return args, clean
