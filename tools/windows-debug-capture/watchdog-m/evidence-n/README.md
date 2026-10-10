# N reaches the later four-entry descriptor constructor

**STATUS: RESEARCH, 2026-10-05.** Saved offline evidence; Windows still reports
NVIDIA Code 43 and nvidia-smi exit 9. N product revision
`9312854d0f67e2db9c6c5dffc7c467955970b816` adds only the generated TMO SFCLOAD
declaration relative to M. All display methods remain refused.

[The labelled L/M/N comparison](comparison.json) validates 24 assertions in each
dump. M→N retains the first 17 and last six; only ordinal 17 changes, from driver
RVA `0x16e97c6` to `0x16e9967`. The generic bugcheck remains `0x1b0` with reviewed
scalar parameters `2`, `0xffffffffc000009a`, `0x100`. Assertion level is not an
NV_STATUS.

Bounded inspection of the same hash-pinned retail 580.88 driver establishes:

| RVA | Meaning in this path |
|---|---|
| `0x16e9755..0x16e976c` | Earlier window-constructor loop increments and tests its count. |
| `0x16e9791..0x16e97a1` | A subsequent `0x8000`-byte CPU allocation is tested; only a nonzero result reaches the later loop. |
| `0x16e9830` | Later loop selects an enabled entry through vtable offset `0xab8`. |
| `0x16e98d9` | Calls constructor `0x16f03c0` with descriptor `rsp+0x40+index*0xb0`. |
| `0x16e98ed..0x16e98f2` | Tests its success byte; zero branches to the saved N assertion. |
| `0x16e9962..0x16e9967` | Assertion helper call and saved return address found in N. |
| `0x16e9908..0x16e990f` | Loop advances through a maximum of four entries. |

N therefore progresses past the earlier ILUT/TMO constructor loop and allocation.
The dump does not identify the failing entry index or inner constructor guard.
Source mapping of this later descriptor is a separate follow-up; no capability
change or working display processing is established by this evidence.

The fresh 343710-byte dump SHA256 is
`4daf4bebe29c6fcf657e0b62cb8816b101fdfabded5ab75f6e3136fee13c7e23`.
The parent recovered it from stopped Windows storage using read-only NBD/NTFS
after a verified controller guest shutdown; cleanup and supervisor success were
recorded. KD was not executed in N. [Collection metadata](collection-status.json)
retains revision, build, recovery and source-receipt hashes.

The comparator discovers the module and NVCD afresh, matches four PE identity
fields to the pinned driver, and requires L to agree with its independent KD
module range and journal. Every input retains the same one-byte-short outer
NVCD envelope: the complete protobuf journal is decoded, but whole-NVCD checksum
verification remains unavailable. This checked dump profile is not a universal
Windows layout. No raw dump, absolute loaded address, private journal or driver
bytes are included here.

Reproduce with the labelled-manifest command in [the parent recipe](../README.md#labelled-follow-up-comparisons-including-n),
whose dependency materialization commands pin exact Git commits. For the bounded
local caller inspection:

```sh
python3 tools/windows-debug-capture/watchdog-m/inspect-n.py \
  --driver /TRUSTED/580.88/Display.Driver/nvlddmkm.sys --disassemble
```

Keep disassembly output private. The source-hash manifest identifies this
inspection script and the strengthened comparator used for the saved N result.
