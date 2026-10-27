"""PF2 reporting seams; no runtime/device behavior and no results at this stage."""

REVISIONS = {
    "A": "2108fb38d7288c82dfb7f5b257228e28fb36a13a",
    "B": "2108fb38d7288c82dfb7f5b257228e28fb36a13a",
    "C": "714ec29d3fdca0a013d9d478755346b0f1af6fa7",
}
APIS = ("direct3d11", "opengl")


def validate(row):
    return "UNRUN"


def summarize(rows):
    return {
        f"{arm}/{api}": {"status": "UNRUN", "n": 0, "median_fps": None}
        for arm in REVISIONS
        for api in APIS
    }
