# Native AD102 display oracle

STATUS: LIVE, 2026-10-19. MEASURED on own rental56766467/source24906bf33,
stock open595.91.07, host-derived AD102/arch190/impl2/PCI2684.

Read-only root tool read_display_caps_page.py uses PROT_READ,32-bit loads,
BAR0 640000+1000 only. No register write, driver unbind or captured-table import.
The complete native sparse page has99nonzero words,00c=100 and even-window
CAPA01df21d0. This is evidence only; existing exact die+branch policy stays.

MEASURED (2026-10-19) strict caps_oracle580.65.06/190/2 against that complete native page
returns1, EXACT_NATIVE_PAGE_EQUAL=false,differing_words=100. Tool built/run at
frozen24906bf33; all offsets/header names/bases are in native_caps_strict_ad102.txt.
This confirms a page difference, not causation for a driver failure. The expected
complete-byte equality remains RED; no root-oracle value becomes a product value.
Windows generic installation was running with standard VGA, not a timed GPU
workload; no kayfabe guest used the native GPU during this read/CPU comparison.
