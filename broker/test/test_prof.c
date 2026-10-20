/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#include "nb_prof.h"
#include <assert.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
static char dir[] = "/tmp/kf-prof-c-XXXXXX", mp[256], jp[256];
static struct nb_prof p;
static void start(void) {
  const char *base = getenv("KF_PROF_C_KEEP_DIR");
  if (!base) {
    assert(mkdtemp(dir));
    base = dir;
  }
  assert(snprintf(mp, sizeof mp, "%s/prof.mmap", base) > 0);
  assert(snprintf(jp, sizeof jp, "%s/prof.jsonl", base) > 0);
  assert(nb_prof_open(&p, mp, jp, 100) == 0);
}
static void done(void) {
  nb_prof_close(&p);
  if (!getenv("KF_PROF_C_KEEP_DIR")) {
    unlink(mp);
    unlink(jp);
    rmdir(dir);
  }
}
static void check_line(const char *needle) {
  char b[65536];
  int f = open(jp, O_RDONLY);
  assert(f >= 0);
  ssize_t n = read(f, b, sizeof b - 1);
  assert(n > 0);
  b[n] = 0;
  assert(strstr(b, needle));
  close(f);
}
int main(int argc, char **argv) {
  assert(argc == 2);
  start();
  if (!strcmp(argv[1], "content")) {
    nb_prof_present(&p, 1000000, true, 16666667);
    nb_prof_present(&p, 21000000, false, 16666667);
    nb_prof_present(&p, 51000000, true, 16666667);
    assert(p.new_frames == 2 && p.repeats == 1);
    assert(nb_prof_record(&p, 1000000100) == 0);
    check_line("\"broker_presented_new_frames\":2");
    check_line("\"max_gap_ns\":50000000");
  } else if (!strcmp(argv[1], "repeat-gap")) {
    nb_prof_present(&p, 1000, true, 0);
    nb_prof_present(&p, 2000, false, 0);
    nb_prof_present(&p, 101000, true, 0);
    nb_prof_present(&p, 102000, false, 0);
    assert(nb_prof_record(&p, 1000000100) == 0);
    check_line("\"max_gap_ns\":100000");
    check_line("\"repeated\":2");
  } else if (!strcmp(argv[1], "gap-threshold")) {
    nb_prof_present(&p, 1000, true, 0);
    nb_prof_present(&p, 100001000, true, 0);
    nb_prof_present(&p, 1100001001, true, 0);
    assert(nb_prof_record(&p, 2100000100) == 0);
    check_line("\"gaps_gt_100ms\":1");
    check_line("\"gaps_gt_1s\":1");
  } else if (!strcmp(argv[1], "quantile")) {
    uint64_t t = 1000;
    nb_prof_present(&p, t, true, 0);
    for (unsigned i = 0; i < 99; i++) {
      t += 1000000;
      nb_prof_present(&p, t, true, 0);
    }
    t += 100000000;
    nb_prof_present(&p, t, true, 0);
    assert(nb_prof_record(&p, 1000000100) == 0);
    check_line("\"p99_gap_ns\":1000000");
    check_line("\"p999_gap_ns\":100000000");
  } else if (!strcmp(argv[1], "overflow")) {
    for (unsigned i = 0; i < NB_PROF_FRAMES + 3; i++)
      nb_prof_present(&p, 1000 + i, true, 0);
    assert(p.frame_dropped == 3);
    assert(nb_prof_record(&p, 1000000100) == 0);
    check_line("\"frame_timestamps_dropped\":3");
  } else if (!strcmp(argv[1], "overflow-next")) {
    for (unsigned i = 0; i < NB_PROF_FRAMES + 9; i++)
      nb_prof_present(&p, 1000 + i, true, 0);
    assert(p.frame_dropped == 9);
    assert(nb_prof_record(&p, 1000000100) == 0);
    assert(p.frame_dropped == 0);
    nb_prof_present(&p, 1000000200, true, 0);
    assert(p.frame_n == 1 && p.frame_dropped == 0);
  } else if (!strcmp(argv[1], "quantile-tail")) {
    uint64_t t = 1000;
    nb_prof_present(&p, t, true, 0);
    for (unsigned i = 0; i < 999; i++) {
      t += 1000000;
      nb_prof_present(&p, t, true, 0);
    }
    t += 2000000000;
    nb_prof_present(&p, t, true, 0);
    assert(nb_prof_record(&p, 4000000100) == 0);
    check_line("\"p99_gap_ns\":1000000");
    check_line("\"p999_gap_ns\":1000000");
  } else if (!strcmp(argv[1], "second-window")) {
    nb_prof_present(&p, 1000, true, 0);
    assert(nb_prof_record(&p, 1000000100) == 0);
    nb_prof_present(&p, 2000001000, true, 0);
    assert(nb_prof_record(&p, 2100000100) == 0);
    assert(p.last_new_ns == 2000001000);
    assert(p.new_frames == 0);
  } else if (!strcmp(argv[1], "exclusive")) {
    struct nb_prof other;
    struct stat before, after;
    assert(stat(mp, &before) == 0);
    assert(nb_prof_open(&other, mp, jp, 900) != 0);
    assert(stat(mp, &after) == 0);
    assert(before.st_ino == after.st_ino && before.st_size == after.st_size);
  } else if (!strcmp(argv[1], "wire")) {
    nb_prof_receive(&p);
    nb_prof_release(&p);
    assert(nb_prof_record(&p, 1000000100) == 0);
    unsigned char h[64];
    int f = open(mp, O_RDONLY);
    assert(f >= 0);
    assert(read(f, h, sizeof h) == sizeof h);
    assert(!memcmp(h, "KFPROF01", 8));
    assert(h[8] == 1);
    close(f);
    check_line("\"received\":1");
    check_line("\"released\":1");
  } else
    abort();
  done();
  puts("PASS");
  return 0;
}
