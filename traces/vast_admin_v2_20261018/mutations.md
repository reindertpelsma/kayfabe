| Mutant | Kills | Two witnesses |
|---|---:|---|
| L01 internal origin bypass | 2 | admin log rejects non-internal origin https://broker.example; admin log rejects non-internal origin http://internal.invalid |
| L02 public log route exposed | 2 | admin log is absent from public gateway /admin/log?run=123-1; admin log is absent from public gateway /pppppppppppppppppppppppppppppppp/admin/log?run=123-1 |
| L03 record capability disclosure | 3 | admin log traverses exact bounded scalar pages, ended=null; admin log traverses exact bounded scalar pages, ended=123 |
| L04 four chunks per page | 4 | admin log traverses exact bounded scalar pages, ended=null; admin log traverses exact bounded scalar pages, ended=123 |
| L05 byte limit removed | 2 | admin log refuses oversized stored page 16385; admin log refuses oversized stored page 65536 |
| L06 traversal truncated | 2 | admin log traverses exact bounded scalar pages, ended=null; admin log traverses exact bounded scalar pages, ended=123 |
| L07 phantom final page | 3 | admin log traverses exact bounded scalar pages, ended=null; admin log traverses exact bounded scalar pages, ended=123 |
| L08 final summary hidden | 2 | admin log traverses exact bounded scalar pages, ended=null; admin log traverses exact bounded scalar pages, ended=123 |
| L09 whole tail used as summary | 3 | admin log traverses exact bounded scalar pages, ended=null; admin log traverses exact bounded scalar pages, ended=123 |
| L10 missing log reported available | 2 | admin log absent/pruned record is expired ; admin log absent/pruned record is expired &cursor=0 |
| L11 cursor range bypass | 2 | admin log rejects out-of-range cursor on empty relay 1; admin log rejects out-of-range cursor on empty relay 2 |
| L12 HTML escape bypass | 2 | log page escapes summary/text and proxies once /log?run=123-1; log page escapes summary/text and proxies once /log?run=456-2&cursor=1 |
| L13 Next loses run binding | 2 | log page escapes summary/text and proxies once /log?run=123-1; log page escapes summary/text and proxies once /log?run=456-2&cursor=1 |
| L14 Next on exhausted page | 2 | last log page has no Next or fabricated summary 123-1; last log page has no Next or fabricated summary 456-2 |
| L15 log links omitted | 4 | terminal row has log link and no Stop button, ended=1800000000000; terminal row has log link and no Stop button, ended=1799999999000 |
| L16 Stop requires provider ID | 5 | open run gets a Stop button before provider ID, phase=CREATING; open run gets a Stop button before provider ID, phase=BOOTING |
| L17 terminal rows get Stop | 2 | terminal row has log link and no Stop button, ended=1800000000000; terminal row has log link and no Stop button, ended=1799999999000 |
| L18 log owner check bypass | 2 | log page rejects another owner /log?run=123-1; log page rejects another owner /log?run=456-2&cursor=1 |
| L19 log error body disclosed | 2 | log broker outage is fixed error text 123-1; log broker outage is fixed error text 456-2 |
| L20 missing log loses expiry message | 2 | expired log renders readable fixed text /log?run=123-1; expired log renders readable fixed text /log?run=456-2&cursor=1 |
