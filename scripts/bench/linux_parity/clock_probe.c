#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <time.h>
int main(int argc, char **argv) {
    struct timespec m, r;
    clock_gettime(CLOCK_MONOTONIC, &m);
    clock_gettime(CLOCK_REALTIME, &r);
    printf("LX2_CLOCK %s mono=%lld.%09ld real=%lld.%09ld\n",
           argc > 1 ? argv[1] : "probe", (long long)m.tv_sec, m.tv_nsec,
           (long long)r.tv_sec, r.tv_nsec);
    return 0;
}
