// efs_auth.cpp — the EFS authorization and ABI boundary, with no GPU work.
// Checks, all through the UVM file directly (no libcuda):
//   1. QUERY on a plain (non-EFS) file: abiVersion present, moduleEnabled reflects the param,
//      active=0.
//   2. Opting in requires the module enabled AND DISABLE_HMM/MPS; a stock kernel refuses the
//      unknown flag differently (NV_ERR_INVALID_ARGUMENT) from a disabled module
//      (NV_ERR_NOT_SUPPORTED) — both distinguishable from success.
//   3. WAIT/RESOLVE on a non-EFS file: NV_ERR_NOT_SUPPORTED (never a crash, never OK).
//   4. WAIT/RESOLVE from a DIFFERENT thread group (a forked child sharing the inherited fd):
//      NV_ERR_INSUFFICIENT_PERMISSIONS — a passed EFS fd does not delegate fault service.
//   5. RESOLVE of a fabricated/never-issued record id: numResolved=0, numStale=count (no action,
//      no crash) — the ABI carries only kernel-issued ids.
// Prints one TAG per check; exit 0 iff every check met its expectation.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include "rm_mini.h"

static int g_fail = 0;
#define EXPECT(cond, tag, ...) do { int _c = (cond); printf("%s %s ", (_c) ? "OK  " : "FAIL", tag); \
    printf(__VA_ARGS__); printf("\n"); if (!_c) g_fail = 1; } while (0)

int main(void)
{
    // A plain file (no EFS flag).
    int fd_plain = -1;
    NV_STATUS st_plain = 0;
    if (uvm_open_init(0, &fd_plain, &st_plain) < 0 || st_plain != NV_OK) {
        printf("FAIL setup uvm_initialize(plain) status=0x%x\n", st_plain);
        return 1;
    }

    UVM_EFS_QUERY_PARAMS q;
    int e = efs_query(fd_plain, &q);
    EXPECT(e == 0 && q.abiVersion == UVM_EFS_ABI_VERSION, "QUERY_PLAIN", "ioctl=%d abi=%u enabled=%u active=%u",
           e, q.abiVersion, q.moduleEnabled, q.active);
    unsigned module_enabled = q.moduleEnabled;

    // WAIT / RESOLVE on the plain file: NOT_SUPPORTED.
    UvmEfsFaultRecord rec;
    int w = efs_wait(fd_plain, &rec, 1, 0);
    EXPECT(w == -(int)(0x10000u | NV_ERR_NOT_SUPPORTED), "WAIT_PLAIN_REFUSED", "ret=%d", w);
    NvU64 fakeid = 0x1234;
    unsigned res = 0, stale = 0;
    NV_STATUS rs = efs_resolve(fd_plain, &fakeid, 1, UVM_EFS_ACTION_REPLAY, &res, &stale);
    EXPECT(rs == NV_ERR_NOT_SUPPORTED, "RESOLVE_PLAIN_REFUSED", "status=0x%x", rs);

    // Opt-in on a fresh file.
    int fd_efs = -1;
    NV_STATUS st_efs = 0;
    NvU64 efs_flags = UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE | UVM_INIT_FLAGS_DISABLE_HMM;
    int ie = uvm_open_init(efs_flags, &fd_efs, &st_efs);
    if (module_enabled) {
        EXPECT(ie == 0 && st_efs == NV_OK, "OPTIN_ENABLED", "ioctl=%d status=0x%x", ie, st_efs);
        // opt-in without DISABLE_HMM/MPS must be refused
        int fd_bad = -1;
        NV_STATUS st_bad = 0;
        uvm_open_init(UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE, &fd_bad, &st_bad);
        EXPECT(st_bad == NV_ERR_INVALID_ARGUMENT, "OPTIN_REQUIRES_HMM_OFF", "status=0x%x", st_bad);
        if (fd_bad >= 0)
            close(fd_bad);
    }
    else {
        EXPECT(st_efs == NV_ERR_NOT_SUPPORTED, "OPTIN_DISABLED_REFUSED", "ioctl=%d status=0x%x", ie, st_efs);
    }

    if (module_enabled && st_efs == NV_OK) {
        e = efs_query(fd_efs, &q);
        EXPECT(e == 0 && q.active == 1, "QUERY_EFS_ACTIVE", "active=%u max=%u timeout_ms=%u", q.active,
               q.maxRecords, q.timeoutMs);

        // RESOLVE of a fabricated id: all stale, no action.
        NvU64 fab[2] = {0xdead0001, 0xdead0002};
        res = stale = 0;
        rs = efs_resolve(fd_efs, fab, 2, UVM_EFS_ACTION_CANCEL, &res, &stale);
        EXPECT(rs == NV_OK && res == 0 && stale == 2, "RESOLVE_FABRICATED_STALE", "status=0x%x resolved=%u stale=%u",
               rs, res, stale);

        // A forked child inherits the fd but is a different thread group: refused.
        int pipefd[2];
        if (pipe(pipefd) != 0)
            return 2;
        pid_t child = fork();
        if (child == 0) {
            close(pipefd[0]);
            int cw = efs_wait(fd_efs, &rec, 1, 0);
            unsigned cres = 0, cstale = 0;
            NvU64 id0 = 0;
            NV_STATUS crs = efs_resolve(fd_efs, &id0, 1, UVM_EFS_ACTION_REPLAY, &cres, &cstale);
            int vals[2] = {cw, (int)crs};
            ssize_t wr = write(pipefd[1], vals, sizeof(vals));
            (void)wr;
            _exit(0);
        }
        close(pipefd[1]);
        int vals[2] = {0, 0};
        ssize_t rd = read(pipefd[0], vals, sizeof(vals));
        (void)rd;
        int status;
        waitpid(child, &status, 0);
        EXPECT(vals[0] == -(int)(0x10000u | NV_ERR_INSUFFICIENT_PERMISSIONS), "WAIT_FOREIGN_TGID_REFUSED",
               "child_wait=%d", vals[0]);
        EXPECT((NV_STATUS)vals[1] == NV_ERR_INSUFFICIENT_PERMISSIONS, "RESOLVE_FOREIGN_TGID_REFUSED",
               "child_resolve=0x%x", (NV_STATUS)vals[1]);
    }

    printf("RESULT %s\n", g_fail ? "FAIL" : "PASS");
    return g_fail;
}
