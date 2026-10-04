/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#define _GNU_SOURCE
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <unistd.h>

/* Run AFTER setpriv drops identity/groups/capabilities. Open no NVIDIA fd here.
 * exec preserves this PID and identity, allowing kernel trace correlation. */
int main(int argc, char **argv) {
    uid_t r, e, s;
    gid_t gr, ge, gs;
    char line[512];
    unsigned caps = 0, nnp = 0;
    if (argc < 2 || getresuid(&r, &e, &s) || getresgid(&gr, &ge, &gs)) return 2;
    if (r != 65534 || e != 65534 || s != 65534 || gr != 65534 || ge != 65534 || gs != 65534 || getgroups(0, NULL) != 0) {
        fputs("POSTURE_FAIL identity/groups\n", stderr); return 2;
    }
    FILE *f = fopen("/proc/self/status", "r");
    if (!f) return 2;
    fprintf(stderr, "POSTURE pid=%ld uid=%u euid=%u suid=%u gid=%u egid=%u sgid=%u\n", (long)getpid(), r, e, s, gr, ge, gs);
    while (fgets(line, sizeof line, f)) {
        if (!strncmp(line, "Cap", 3)) {
            unsigned long long value;
            if (sscanf(line, "%*[^:]: %llx", &value) != 1 || value != 0) {
                fputs("POSTURE_FAIL capabilities\n", stderr); fclose(f); return 2;
            }
            ++caps; fputs(line, stderr);
        }
        if (!strncmp(line, "NoNewPrivs:", 11)) {
            if (sscanf(line, "NoNewPrivs: %u", &nnp) != 1 || nnp != 1) {
                fputs("POSTURE_FAIL no_new_privs\n", stderr); fclose(f); return 2;
            }
            fputs(line, stderr);
        }
    }
    fclose(f);
    if (caps != 5 || nnp != 1) { fputs("POSTURE_FAIL incomplete\n", stderr); return 2; }
    const char *preload = getenv("AUDIT_PRELOAD");
    if (preload && setenv("LD_PRELOAD", preload, 1)) return 2;
    fputs("POSTURE_PASS exec\n", stderr);
    fflush(stderr);
    execvp(argv[1], argv + 1);
    perror("execvp");
    return errno ? 127 : 2;
}
