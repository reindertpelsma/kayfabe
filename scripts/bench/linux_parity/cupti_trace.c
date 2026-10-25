/* Optional CUPTI injection collector. Use only in separate matched diagnostic passes. */
#define _POSIX_C_SOURCE 200809L
#include <cupti.h>
#include <pthread.h>
#include <stdint.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

static FILE *out;
static pthread_mutex_t log_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_once_t init_once = PTHREAD_ONCE_INIT;
static _Atomic int ok = 1;
static int check(CUptiResult result, const char *operation) {
    if (result == CUPTI_SUCCESS) return 1;
    const char *message = "unknown";
    cuptiGetResultString(result, &message);
    fprintf(out, "CUPTI_FAIL operation=%s status=%d message=%s\n", operation,
            (int)result, message);
    fflush(out);
    ok = 0;
    return 0;
}
static void CUPTIAPI requested(uint8_t **buffer, size_t *size, size_t *max_records) {
    *size = 4 * 1024 * 1024;
    *buffer = malloc(*size);
    *max_records = 0;
    if (!*buffer) {
        pthread_mutex_lock(&log_lock);
        fprintf(out, "CUPTI_FAIL allocation\n");
        fflush(out);
        ok = 0;
        pthread_mutex_unlock(&log_lock);
        *size = 0;
    }
}
static void CUPTIAPI completed(CUcontext context, uint32_t stream, uint8_t *buffer,
                               size_t size, size_t valid) {
    (void)size;
    CUpti_Activity *record;
    pthread_mutex_lock(&log_lock);
    CUptiResult result;
    while ((result = cuptiActivityGetNextRecord(buffer, valid, &record)) == CUPTI_SUCCESS) {
        if (record->kind == CUPTI_ACTIVITY_KIND_CONCURRENT_KERNEL) {
            const CUpti_ActivityKernel9 *k = (const void *)record;
            fprintf(out,
                    "CUPTI_KERNEL name=%s start=%llu end=%llu context=%u stream=%u "
                    "correlation=%u grid=%d,%d,%d block=%d,%d,%d registers=%u\n",
                    k->name, (unsigned long long)k->start, (unsigned long long)k->end,
                    k->contextId, k->streamId, k->correlationId,
                    k->gridX, k->gridY, k->gridZ, k->blockX, k->blockY, k->blockZ,
                    k->registersPerThread);
            if (!k->start || k->end < k->start) {
                fprintf(out, "CUPTI_FAIL missing_or_inverted_kernel_timestamp\n");
                ok = 0;
            }
        } else if (record->kind == CUPTI_ACTIVITY_KIND_DRIVER) {
            const CUpti_ActivityAPI *a = (const void *)record;
            fprintf(out, "CUPTI_DRIVER cbid=%u start=%llu end=%llu correlation=%u\n",
                    a->cbid, (unsigned long long)a->start, (unsigned long long)a->end,
                    a->correlationId);
        } else {
            fprintf(out, "CUPTI_ACTIVITY kind=%u\n", (unsigned)record->kind);
        }
    }
    if (result != CUPTI_ERROR_MAX_LIMIT_REACHED) check(result, "get-next-record");
    size_t dropped = 0;
    if (check(cuptiActivityGetNumDroppedRecords(context, stream, &dropped), "drops")) {
        fprintf(out, "CUPTI_DROPPED context=%p stream=%u count=%zu\n",
                (void *)context, stream, dropped);
        if (dropped) ok = 0;
    }
    fflush(out);
    pthread_mutex_unlock(&log_lock);
    free(buffer);
}
static void finish(void) {
    check(cuptiActivityFlushAll(CUPTI_ACTIVITY_FLAG_FLUSH_FORCED), "final-flush");
    pthread_mutex_lock(&log_lock);
    fprintf(out, "CUPTI_DONE valid=%d\n", ok);
    fflush(out);
    pthread_mutex_unlock(&log_lock);
    /* CUPTI may still own callbacks during shared-library teardown; leave FILE open. */
}
static void initialize(void) {
    const char *path = getenv("LX2_CUPTI_OUT");
    out = path ? fopen(path, "wx") : NULL;
    if (!out) {
        fputs("CUPTI_FAIL missing_or_existing_output\n", stderr);
        ok = 0;
        return;
    }
    uint64_t timestamp = 0;
    struct timespec m, r;
    clock_gettime(CLOCK_MONOTONIC, &m);
    clock_gettime(CLOCK_REALTIME, &r);
    check(cuptiGetTimestamp(&timestamp), "timestamp");
    fprintf(out, "CUPTI_CLOCK gpu=%llu mono=%lld.%09ld real=%lld.%09ld\n",
            (unsigned long long)timestamp, (long long)m.tv_sec, m.tv_nsec,
            (long long)r.tv_sec, r.tv_nsec);
    if (!check(cuptiActivityRegisterCallbacks(requested, completed), "register")) return;
    if (!check(cuptiActivityEnable(CUPTI_ACTIVITY_KIND_DRIVER), "enable-driver")) return;
    if (!check(cuptiActivityEnable(CUPTI_ACTIVITY_KIND_CONCURRENT_KERNEL), "enable-kernel"))
        return;
    atexit(finish);
    fflush(out);
}
int InitializeInjection(void) {
    pthread_once(&init_once, initialize);
    return ok;
}
