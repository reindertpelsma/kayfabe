# Exact image range producer

**STATUS: ANSWERED, 2026-10-15.** Relay producer compatibility fix; full AD102
image integrity still must be checked after its transfer completes.

Before the fix, the trusted source host's dd identifies as uutils coreutils0.8.0.
A one-MiB read of the regular compressed baseline into a pipe emits zero bytes,
reports "failed to seek in output file: Illegal seek", and fails. On the rental,
GNU dd8.32 writes/seeks a regular scratch output correctly. Falsifier before the
probe: if the same bounded source dd-to-pipe succeeds, investigate the receiver.
The source probe fails and the receiver scratch seek passes. No GPU inference.

Use a fixed-budget pread producer instead: regular file only, exact range checked
before emitting, count at most1GiB, read blocks at most128KiB, early EOF/error
propagates through the existing relay pipefail guard. Never seek on stdout.
The last chunk's count is its exact remaining size. Quote the source path and
reader as shell arguments. Keep mandatory full source/destination SHA256.

Three range tests cover multiblock/last-byte reads, bounds/truncation, and a
nonregular source; all19 harness tests pass, shell syntax passes. The streaming
failure tests still require no completed marker on a failed sender or decoder.
