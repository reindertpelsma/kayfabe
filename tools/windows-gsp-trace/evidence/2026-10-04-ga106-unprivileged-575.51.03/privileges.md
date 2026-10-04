# Ordinary-user reachability of the Windows control inventory

**STATUS: RESEARCH, 2026-10-04. Linux reachability; no Windows caller-privilege claim.**

See [method and limits](README.md). Full record numbers, ioctl intervals, credentials and source flags: [privileges.json](privileges.json).

Of 129 Windows IDs, **48** were sent to native GSP by matching ordinary-user control ioctls; **8** were sent internally during ordinary-user ioctls (**55** distinct IDs combined). A send does not imply firmware success or safe forwarding.

Source column: exported-method privilege gate in OGKM 580.65.06. This alone does not prove live emission. `unresolved` means no matching ordinary export was found; generic GSS/BinAPI routes may apply. Source data for the exact tested 575.51.03 release is also retained.

| Control | Source gate | Native GSP direct / indirect sends | Accepted direct ioctl: open / closed 575 |
|---|---|---|---|
| `0x00730102` | user | 2 / 0 | 2 / 1 |
| `0x00730108` | user | 1 / 0 | 1 / 1 |
| `0x0073010a` | user | 0 / 0 | 0 / 0 |
| `0x0073010c` | user | 4 / 0 | 4 / 4 |
| `0x00730122` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x00730128` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x0073012c` | admin | 0 / 0 | 0 / 0 |
| `0x00730250` | user | 0 / 0 | 0 / 0 |
| `0x00730280` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x00730282` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x0073028b` | admin | 0 / 0 | 0 / 0 |
| `0x007302a4` | kernel | 0 / 0 | 0 / 0 |
| `0x007302a5` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x00731140` | user | 0 / 0 | 0 / 0 |
| `0x00731142` | user | 0 / 0 | 0 / 0 |
| `0x00731144` | admin | 0 / 0 | 0 / 0 |
| `0x0073114e` | admin | 0 / 0 | 0 / 0 |
| `0x00731150` | kernel | 0 / 0 | 0 / 0 |
| `0x00731152` | admin | 0 / 0 | 0 / 0 |
| `0x00731341` | admin | 0 / 0 | 0 / 0 |
| `0x00731343` | admin | 0 / 0 | 0 / 0 |
| `0x00731359` | admin | 0 / 0 | 0 / 0 |
| `0x00731360` | admin | 0 / 0 | 0 / 0 |
| `0x00731362` | admin | 0 / 0 | 0 / 0 |
| `0x00731368` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x00731378` | admin | 0 / 0 | 0 / 0 |
| `0x00731381` | admin | 0 / 0 | 0 / 0 |
| `0x00800294` | user | 2 / 0 | 2 / 1 |
| `0x0080170e` | user | 1 / 0 | 1 / 0 |
| `0x0080170f` | user | 0 / 0 | 0 / 0 |
| `0x00801812` | kernel | 0 / 0 | 0 / 0 |
| `0x20800102` | user | 3 / 0 | 10 / 10 |
| `0x2080012b` | admin | 0 / 18 | 0 / 0 |
| `0x2080012d` (additional) | admin | 0 / 0 | 0 / 0 |
| `0x2080012f` | user | 3 / 2 | 0 / 0 |
| `0x2080014b` | user | 5 / 0 | 1 / 1 |
| `0x20800156` | user | 1 / 0 | 1 / 1 |
| `0x20800157` | user | 1 / 0 | 0 / 0 |
| `0x208001a4` | user | 1 / 0 | 1 / 1 |
| `0x20800a38` | internal | 0 / 0 | 0 / 0 |
| `0x20800a9a` | internal | 0 / 3 | 0 / 0 |
| `0x20800af0` | internal | 0 / 0 | 0 / 0 |
| `0x20800af1` | internal | 0 / 0 | 0 / 0 |
| `0x20800af2` | internal | 0 / 0 | 0 / 0 |
| `0x20800aff` | internal | 0 / 7 | 0 / 0 |
| `0x2080110b` | user | 0 / 0 | 0 / 0 |
| `0x20801111` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x20801208` | user | 0 / 0 | 0 / 0 |
| `0x20801211` | user | 0 / 0 | 0 / 0 |
| `0x2080121f` (additional) | kernel | 0 / 0 | 0 / 0 |
| `0x20801303` | user | 4 / 0 | 11 / 11 |
| `0x20801322` | user | 1 / 0 | 0 / 0 |
| `0x20801344` | user | 1 / 0 | 0 / 0 |
| `0x20801347` | user | 1 / 0 | 0 / 0 |
| `0x20801357` | user | 1 / 0 | 0 / 0 |
| `0x20801813` | user | 2 / 0 | 2 / 2 |
| `0x20801819` | user | 4 / 0 | 4 / 4 |
| `0x20801823` | user | 2 / 0 | 4 / 4 |
| `0x20801829` | user | 1 / 0 | 1 / 1 |
| `0x20801830` | user | 1 / 0 | 1 / 1 |
| `0x20802a08` | kernel | 0 / 10 | 0 / 0 |
| `0x20803083` | user | 1 / 0 | 1 / 1 |
| `0x20803400` | user | 1 / 0 | 1 / 1 |
| `0x20803401` | user | 1 / 0 | 1 / 1 |
| `0x20803404` | user | 0 / 0 | 0 / 0 |
| `0x20808159` | unresolved ordinary export | 2 / 0 | 2 / 2 |
| `0x2080852a` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x2080852b` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080852c` | unresolved ordinary export | 2 / 0 | 2 / 2 |
| `0x2080852e` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x2080852f` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x20808530` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x20808536` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x20808537` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x20808539` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080853a` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x20808542` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x20808546` | unresolved ordinary export | 18 / 0 | 6 / 6 |
| `0x2080880f` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x20809001` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x20809004` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x20809019` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x2080901b` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x20809029` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080902a` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080902b` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080902c` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x20809037` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x20809038` | unresolved ordinary export | 1 / 0 | 0 / 0 |
| `0x20809063` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a026` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a028` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a079` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a080` | unresolved ordinary export | 2 / 0 | 2 / 2 |
| `0x2080a081` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a084` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x2080a088` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a095` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a0a4` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x2080a0a7` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x2080a0a8` | unresolved ordinary export | 4 / 0 | 4 / 4 |
| `0x2080a0c4` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a0c5` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a0c8` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a0cc` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a0d1` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a0f2` | unresolved ordinary export | 1 / 0 | 0 / 0 |
| `0x2080a612` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x2080a618` | unresolved ordinary export | 1 / 0 | 1 / 1 |
| `0x2080a630` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a637` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080a63c` | unresolved ordinary export | 1 / 0 | 0 / 0 |
| `0x2080a801` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080b201` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080b202` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080b209` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080b210` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080b216` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080d02d` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x2080e0af` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x20810108` | unresolved ordinary export | 4 / 0 | 4 / 4 |
| `0x2081010d` | unresolved ordinary export | 0 / 0 | 0 / 0 |
| `0x50800101` | user | 0 / 0 | 0 / 0 |
| `0x90e70113` | user | 1 / 0 | 0 / 0 |
| `0x90f10106` | admin | 0 / 4 | 0 / 0 |
| `0xa06c0101` | user | 6 / 0 | 6 / 6 |
| `0xa06c0103` | user | 2 / 0 | 2 / 2 |
| `0xa06c010a` | kernel | 0 / 10 | 0 / 0 |
| `0xa06f0103` | user | 0 / 8 | 0 / 0 |
| `0xc3700104` | kernel | 0 / 0 | 0 / 0 |
| `0xc3720101` | user | 0 / 0 | 0 / 0 |
