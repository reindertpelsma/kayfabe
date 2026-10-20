/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
/* Output only: no mapped byte is ever read as control, geometry or a cursor. */
#include "nb_prof.h"
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <limits.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>
#define FILE_BYTES 16777216u
#define RECORD_BASE 13635584u
#define RECORD_BYTES 65536u
#define RECORDS 47u
static void put64(unsigned char *p, uint64_t v) {
  for (unsigned i = 0; i < 8; i++)
    p[i] = (unsigned char)(v >> (i * 8));
}
static void put32(unsigned char *p, uint32_t v) {
  for (unsigned i = 0; i < 4; i++)
    p[i] = (unsigned char)(v >> (i * 8));
}
static void seq(unsigned char *p, uint64_t v) {
  atomic_store_explicit((_Atomic uint64_t *)(void *)p, v, memory_order_release);
}
static uint64_t sat(uint64_t n) { return n == UINT64_MAX ? n : n + 1; }
int nb_prof_open(struct nb_prof *p, const char *mp, const char *jp,
                 uint64_t now) {
  memset(p, 0, sizeof *p);
  p->fd = p->json_fd = -1;
  p->start = now;
#if __BYTE_ORDER__ != __ORDER_LITTLE_ENDIAN__
  errno = ENOTSUP;
  return -1;
#endif
  p->fd = open(mp, O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0600);
  if (p->fd < 0)
    return -1;
  p->json_fd =
      open(jp, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0600);
  if (p->json_fd < 0) {
    close(p->fd);
    p->fd = -1;
    unlink(mp);
    return -1;
  }
  if (ftruncate(p->fd, FILE_BYTES))
    goto fail;
  int e = posix_fallocate(p->fd, 0, FILE_BYTES);
  if (e) {
    errno = e;
    goto fail;
  }
  p->map = mmap(NULL, FILE_BYTES, PROT_READ | PROT_WRITE, MAP_SHARED, p->fd, 0);
  if (p->map == MAP_FAILED) {
    p->map = NULL;
    goto fail;
  }
  memset(p->map, 0, FILE_BYTES);
  memcpy(p->map, "KFPROF01", 8);
  put32(p->map + 8, 1);
  put32(p->map + 12, 0x01020304);
  put64(p->map + 16, FILE_BYTES);
  put64(p->map + 32, (uint64_t)getpid());
  put64(p->map + 40, 4);
  put64(p->map + 48, now);
  put64(p->map + 64, 1);
  put64(p->map + 72, now);
  put64(p->map + 80, now);
  put64(p->map + 96, 1000000000);
  put64(p->map + 104, 1);
  struct timespec resolution;
  if (clock_getres(CLOCK_MONOTONIC, &resolution) == 0 &&
      resolution.tv_sec == 0 && resolution.tv_nsec >= 0)
    put64(p->map + 112, (uint64_t)resolution.tv_nsec);
#ifdef NB_PROF_REV
#define NB_PROF_STRING_1(x) #x
#define NB_PROF_STRING(x) NB_PROF_STRING_1(x)
  static const char rev[] = NB_PROF_STRING(NB_PROF_REV);
  if (sizeof rev == 41)
    memcpy(p->map + 128, rev, 40);
#endif
  seq(p->map + 24, 2);
  return 0;
fail:
  nb_prof_close(p);
  unlink(mp);
  unlink(jp);
  return -1;
}
void nb_prof_close(struct nb_prof *p) {
  if (p->map) {
    munmap(p->map, FILE_BYTES);
    p->map = NULL;
  }
  if (p->fd >= 0)
    close(p->fd);
  if (p->json_fd >= 0)
    close(p->json_fd);
  p->fd = p->json_fd = -1;
}
void nb_prof_receive(struct nb_prof *p) {
  if (p->fd >= 0)
    p->received = sat(p->received);
}
void nb_prof_release(struct nb_prof *p) {
  if (p->fd >= 0)
    p->released = sat(p->released);
}
void nb_prof_present(struct nb_prof *p, uint64_t ts, bool fresh,
                     uint64_t period) {
  if (p->fd < 0 || p->failed)
    return;
  p->feedback_known = true;
  p->period = period;
  p->last_present_ns = ts;
  if (!fresh) {
    p->repeats = sat(p->repeats);
    return;
  }
  p->new_frames = sat(p->new_frames);
  uint64_t gap =
      p->last_new_ns && ts > p->last_new_ns ? ts - p->last_new_ns : 0;
  if (!p->last_new_ns || ts > p->last_new_ns)
    p->last_new_ns = ts;
  if (p->frame_n < NB_PROF_FRAMES) {
    p->stamps[p->frame_n] = ts;
    p->gaps[p->frame_n++] = gap;
  } else
    p->frame_dropped = sat(p->frame_dropped);
}
static int cmp64(const void *a, const void *b) {
  uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b;
  return (x > y) - (x < y);
}
int nb_prof_record(struct nb_prof *p, uint64_t now) {
  if (p->fd < 0 || p->failed || now < p->start ||
      p->index >= UINT64_MAX / 2 - 2)
    return -1;
  uint64_t sorted[NB_PROF_FRAMES], n = 0, g33 = 0, g50 = 0, g100 = 0, g250 = 0,
                                   g1s = 0;
  for (uint64_t i = 0; i < p->frame_n; i++) {
    uint64_t g = p->gaps[i];
    if (!g)
      continue;
    sorted[n++] = g;
    g33 += g > 33000000;
    g50 += g > 50000000;
    g100 += g > 100000000;
    g250 += g > 250000000;
    g1s += g > 1000000000;
  }
  qsort(sorted, (size_t)n, sizeof sorted[0], cmp64);
  uint64_t max = n ? sorted[n - 1] : 0,
           p99 = n ? sorted[(n * 99 + 99) / 100 - 1] : 0,
           p999 = n ? sorted[(n * 999 + 999) / 1000 - 1] : 0;
  char count[32];
  if (p->feedback_known)
    snprintf(count, sizeof count, "%" PRIu64, p->new_frames);
  else
    strcpy(count, "null");
  char b[RECORD_BYTES - 64];
  int k = snprintf(
      b, sizeof b,
      "{\"schema\":1,\"source\":\"broker\",\"start_ns\":%" PRIu64
      ",\"end_ns\":%" PRIu64
      ",\"threads\":[],\"ledger\":{\"oldest\":[]},\"counters\":{\"rmcontrol_"
      "seen\":null,\"host_rm_calls\":null,\"display_new_frames\":null,\"broker_"
      "presented_new_frames\":%s},\"broker\":{\"received\":%" PRIu64
      ",\"released\":%" PRIu64 ",\"repeated\":%" PRIu64
      ",\"credits\":null,\"refresh_ns\":%" PRIu64
      ",\"last_feedback_ns\":%" PRIu64 ",\"raw_feedback_ns\":%" PRIu64
      ",\"clock_only\":%" PRIu64
      "},\"pacing\":{\"basis\":\"validated_monotonic_or_observation\",\"max_"
      "gap_ns\":%" PRIu64 ",\"p99_gap_ns\":%" PRIu64 ",\"p999_gap_ns\":%" PRIu64
      ",\"gaps_gt_33ms\":%" PRIu64 ",\"gaps_gt_50ms\":%" PRIu64
      ",\"gaps_gt_100ms\":%" PRIu64 ",\"gaps_gt_250ms\":%" PRIu64
      ",\"gaps_gt_1s\":%" PRIu64 ",\"gap_samples\":%" PRIu64
      "},\"coverage\":{\"production_sources\":false,\"frame_timestamps_"
      "dropped\":%" PRIu64 ",\"timestamp_rows_omitted\":%" PRIu64
      "},\"frame_timestamps_ns\":[",
      p->start, now, count, p->received, p->released, p->repeats, p->period,
      p->last_present_ns, p->raw_feedback_ns, p->clock_only, max, p99, p999,
      g33, g50, g100, g250, g1s, n, p->frame_dropped,
      p->frame_n > 512 ? p->frame_n - 512 : 0);
  if (k < 0 || (size_t)k >= sizeof b)
    goto fail;
  size_t used = (size_t)k;
  for (uint64_t i = 0; i < p->frame_n && i < 512; i++) {
    k = snprintf(b + used, sizeof b - used, "%s%" PRIu64, i ? "," : "",
                 p->stamps[i]);
    if (k < 0 || (size_t)k >= sizeof b - used)
      goto fail;
    used += (size_t)k;
  }
  if (used + 4 >= sizeof b)
    goto fail;
  memcpy(b + used, "]}", 2);
  used += 2;
  size_t off = RECORD_BASE + (size_t)(p->index % RECORDS) * RECORD_BYTES;
  uint64_t even = (p->index + 1) * 2;
  seq(p->map + off, even - 1);
  atomic_thread_fence(memory_order_release);
  put64(p->map + off + 8, p->start);
  put64(p->map + off + 16, now);
  put64(p->map + off + 24, used);
  put64(p->map + off + 32, p->index + 1);
  memcpy(p->map + off + 64, b, used);
  seq(p->map + off, even);
  seq(p->map + 24, even + 1);
  atomic_thread_fence(memory_order_release);
  put64(p->map + 56, now);
  put64(p->map + 176, p->index + 1);
  seq(p->map + 24, even + 2);
  struct stat st;
  if (fstat(p->json_fd, &st) || st.st_size < 0 || st.st_size > 32 * 1024 * 1024)
    goto fail;
  b[used++] = '\n';
  if (write(p->json_fd, b, used) != (ssize_t)used)
    goto fail;
  p->index++;
  p->start = now;
  p->received = p->released = p->new_frames = p->repeats = p->frame_n =
      p->frame_dropped = p->clock_only = 0;
  return 0;
fail:
  p->failed = true;
  return -1;
}
#ifdef NB_PROF
static struct nb_prof global = {.fd = -1, .json_fd = -1};
static bool pending, has_feedback;
static uint64_t mono(void) {
  struct timespec t;
  if (clock_gettime(CLOCK_MONOTONIC, &t))
    return 0;
  return (uint64_t)t.tv_sec * 1000000000u + (uint64_t)t.tv_nsec;
}
bool nb_prof_on(void) { return global.fd >= 0 && !global.failed; }
static void finish(void) {
  if (nb_prof_on())
    (void)nb_prof_record(&global, mono());
  nb_prof_close(&global);
}
void nb_prof_boot(void) {
  const char *path = getenv("KF3_BROKER_PROF_FILE");
  if (!path || !*path)
    return;
  char json[PATH_MAX];
  int n = snprintf(json, sizeof json, "%s.jsonl", path);
  if (n < 0 || (size_t)n >= sizeof json)
    return;
  if (nb_prof_open(&global, path, json, mono()) == 0)
    atexit(finish);
}
void nb_prof_poll(void) {
  if (!nb_prof_on())
    return;
  uint64_t now = mono();
  if (now >= global.start && now - global.start >= 1000000000)
    (void)nb_prof_record(&global, now);
}
void nb_prof_received(void) { nb_prof_receive(&global); }
void nb_prof_released(void) { nb_prof_release(&global); }
void nb_prof_content(uint64_t raw, bool fresh) {
  if (nb_prof_on()) {
    pending = fresh;
    has_feedback = true;
    global.raw_feedback_ns = raw;
  }
}
void nb_prof_feedback(uint64_t now, uint64_t ts, uint64_t period,
                      bool clock_only) {
  if (!nb_prof_on() || !has_feedback)
    return;
  if (clock_only)
    global.clock_only = sat(global.clock_only);
  nb_prof_present(&global, clock_only ? now : ts, pending, period);
  has_feedback = pending = false;
  nb_prof_poll();
}
#endif
