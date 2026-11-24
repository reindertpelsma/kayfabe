/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * nb_launch.c -- the broker's START-UP decisions, and nothing else.
 *
 * No I/O, no environment, no logging: every function takes what it should
 * judge by.  That is what lets test/test_launch.c pin each rule and
 * test/mutate_launch.py break each one and watch it get caught.  The callers
 * (nvkvm_broker.c, nb_session_wl.c, nb_common.c) own the syscalls.
 *
 * The rule they share: a flag always wins over the environment variable it
 * replaces, and with no flag the old environment behaviour is unchanged.
 */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/un.h>

#include "nvkvm_broker.h"

int nb_wl_socket_path(const char *flag_name, const char *flag_runtime_dir,
                      const char *env_name, const char *env_runtime_dir,
                      char *out, size_t cap)
{
    const char *name, *dir;
    int n;

    if (cap) {
        out[0] = '\0';
    }
    if (!flag_name && !flag_runtime_dir) {
        return 0;               /* libwayland reads its own environment */
    }
    name = flag_name ? flag_name
                     : (env_name && *env_name ? env_name : "wayland-0");
    if (!*name) {
        return NB_WL_EMPTY_NAME;
    }
    if (name[0] == '/') {
        n = snprintf(out, cap, "%s", name);
    } else {
        dir = flag_runtime_dir ? flag_runtime_dir : env_runtime_dir;
        if (!dir || !*dir) {
            return NB_WL_NO_RUNTIME_DIR;
        }
        n = snprintf(out, cap, "%s/%s", dir, name);
    }
    if (n < 0 || (size_t)n >= cap ||
        (size_t)n >= sizeof(((struct sockaddr_un *)0)->sun_path)) {
        if (cap) {
            out[0] = '\0';
        }
        return NB_WL_TOO_LONG;
    }
    return 0;
}

int nb_sockdir_verdict(uint32_t st_mode, uid_t st_uid, uid_t self,
                       const uid_t *trusted, int n_trusted)
{
    bool owner_ok = st_uid == self || st_uid == 0;
    int i;

    if (!S_ISDIR(st_mode)) {
        return NB_DIR_NOT_DIR;
    }
    for (i = 0; i < n_trusted && !owner_ok; i++) {
        owner_ok = trusted[i] == st_uid;
    }
    if (!owner_ok) {
        return NB_DIR_FOREIGN_OWNER;
    }
    /* Our own directory is ours to widen; someone else's must not be open to
     * a third party.  A sticky directory (like /tmp) stops others deleting
     * entries they do not own, which is the one property that matters here. */
    if (st_uid != self && (st_mode & 0022) && !(st_mode & S_ISVTX)) {
        return NB_DIR_FOREIGN_WIDE;
    }
    return NB_DIR_OK;
}

unsigned nb_sockdir_create_mode(unsigned socket_mode)
{
    unsigned m = 0700;

    if (socket_mode & 0060) {
        m |= 0010;
    }
    if (socket_mode & 0006) {
        m |= 0001;
    }
    return m;
}

bool nb_sudo_uid_fallback(uid_t self_uid, const char *sudo_uid_env,
                          bool allow_uid_flag, uid_t *out)
{
    if (self_uid != 0 || allow_uid_flag || !sudo_uid_env) {
        return false;
    }
    *out = (uid_t)strtoul(sudo_uid_env, NULL, 10);
    return true;
}

int nb_parse_uid(const char *s, uid_t *out)
{
    unsigned long long v = 0;
    const char *p;

    if (!s || !*s) {
        return -EINVAL;
    }
    for (p = s; *p; p++) {
        if (*p < '0' || *p > '9') {
            return -EINVAL;
        }
        v = v * 10 + (unsigned)(*p - '0');
        if (v > 4294967294ull) {
            return -ERANGE;     /* (uid_t)-1 is "no uid" to chown() */
        }
    }
    *out = (uid_t)v;
    return 0;
}

const char *nb_display_flag_conflict(const struct nb_config *cfg)
{
    const char *be = cfg->backend ? cfg->backend : "auto";
    bool wl_ok = strcmp(be, "x11") != 0 && strcmp(be, "test") != 0;
    bool x_ok  = strcmp(be, "wayland") != 0 && strcmp(be, "test") != 0;

    if (cfg->wayland_display && !wl_ok) {
        return "--wayland-display";
    }
    if (cfg->runtime_dir && !wl_ok) {
        return "--runtime-dir";
    }
    if (cfg->x11_display && !x_ok) {
        return "--x11-display";
    }
    if (cfg->xauthority && !x_ok) {
        return "--xauthority";
    }
    return NULL;
}
