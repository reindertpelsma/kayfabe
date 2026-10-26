# Default Vast check selection

CODE READ from source at rev 9a995ad97 plus the selection change in this branch: an ordinary
`check` request considers every configured `CHECK_DIES` class that has eligible offers. It
keeps the configured country preference and effective-price ordering within each class,
then chooses the class whose selected offer has the lowest effective hourly price. Equal
prices retain configured die order. `DEFAULT_ORDER` remains the compatible protocol label
for this default policy. An explicit ordered die preference or sweep rotation keeps its
existing order.

Every verification, quality, country deny, machine exclusion and spending check still
applies. Availability lists eligible per-die prices; the runner prints the actual rental's
machine, host, instance, GPU and offer hourly price. The offer price excludes the broker's
transfer allowance; admission uses the full reservation estimate.
