# USER relay family admission result

STATUS: LIVE, 2026-10-19. Approved narrow topic implemented and verified GPU-free.

CODE READ (commit 007ba1eb0:5): quoted historical statement, evidence not revalidated by BW9. MEASURED: unchanged predicate extraction + eight tests committed alone as
EXPECTED RED 2fb8aa503 (five pass, three runtime failures). Fix 871c38fad passes
full release kf-qemu: 312/0/0 before -> 320/0/0 after; all 312 previous test
names remain. Four mutants killed by 3/2/4/2 tests respectively (see JSON).
Clippy --all-targets --release: 451 diagnostics before and after, added=0;
both runs inherit the same 89 compile-denied clippy errors. Rustfmt verifies
all added regions, retaining ten inherited unrelated formatting hunks.

CODE READ / TESTED: Ada accepts exactly c56f across the complete 16-bit class
space plus high aliases/maximum. The generated Ada extras c36f and c46f remain
REFUSED; native birth/permit behavior is unchanged and no widening is claimed.
Other families admit only their generated ChannelGpfifo classes with unchanged
entry/power-of-two/alignment/overflow limits. No protected code outside the
helper, call site and new tests changed. Current hardware remains UNRUN.

Raw text logs remain on the development machine under sweep-userrelay-*;
small hashes/counts, test retention, mutations and clippy comparison are committed.
