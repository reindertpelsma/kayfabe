"""Preparation-only wait seam; media remains checksum-verified by the fetcher."""
class MediaRefused(RuntimeError):pass

def await_media(ready, alive, clock, sleep, deadline):
    for _ in range(120):
        if ready():return True
        if not alive():raise MediaRefused('media job exited without success')
        sleep(5)
    if ready():return True
    raise MediaRefused('ten-minute preparation wait expired')
