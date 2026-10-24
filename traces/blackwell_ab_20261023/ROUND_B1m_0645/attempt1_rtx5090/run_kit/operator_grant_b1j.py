"""Controller 04:20 B1j selection: the qualifying offer with the most CPU cores on the default
profile (>= 16 effective cores). A sub-16-core machine (m58852) is only rented from a separately
wired 7-visible-core workspace, labelled as such."""

DEFAULT_MIN_CORES = 16


def select(eligible):
    default = [x for x in eligible if float(x.get("cpu_cores_effective", 0)) >= DEFAULT_MIN_CORES]
    if not default:
        return []
    top = max(float(x["cpu_cores_effective"]) for x in default)
    return [x for x in default if float(x["cpu_cores_effective"]) == top]
