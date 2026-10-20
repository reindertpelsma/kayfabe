/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#ifndef NB_PROF_H
#define NB_PROF_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#define NB_PROF_FRAMES 4096u
struct nb_prof {
  int fd, json_fd;
  unsigned char *map;
  bool failed, feedback_known;
  uint64_t start, index, received, new_frames, repeats, released, period,
      last_new_ns, last_present_ns;
  uint64_t frame_n, frame_dropped, clock_only, raw_feedback_ns;
  uint64_t stamps[NB_PROF_FRAMES], gaps[NB_PROF_FRAMES];
};
int nb_prof_open(struct nb_prof *, const char *, const char *, uint64_t);
void nb_prof_close(struct nb_prof *);
void nb_prof_receive(struct nb_prof *);
void nb_prof_release(struct nb_prof *);
void nb_prof_present(struct nb_prof *, uint64_t, bool, uint64_t);
int nb_prof_record(struct nb_prof *, uint64_t);
#ifdef NB_PROF
bool nb_prof_on(void);
void nb_prof_boot(void);
void nb_prof_poll(void);
void nb_prof_received(void);
void nb_prof_released(void);
void nb_prof_content(uint64_t, bool);
void nb_prof_feedback(uint64_t, uint64_t, uint64_t, bool);
#else
static inline bool nb_prof_on(void) { return false; }
#define nb_prof_boot() ((void)0)
#define nb_prof_poll() ((void)0)
#define nb_prof_received() ((void)0)
#define nb_prof_released() ((void)0)
#define nb_prof_content(ts, is_new) ((void)0)
#define nb_prof_feedback(now, ts, period, clock_only) ((void)0)
#endif
#endif
