/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * test_launch.c -- the broker's start-up decisions (nb_launch.c), driven
 * directly: no socket, no display, no environment.  Linked against the SAME
 * nb_launch.c the broker links.  Every check prints a DISTINCT message per
 * rule, because test/mutate_launch.py counts distinct failing messages.
 */
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>

#include "nvkvm_broker.h"

static unsigned n_checks, n_fail;

#define CHECK(cond, ...) do {                                           \
        n_checks++;                                                     \
        if (!(cond)) {                                                  \
            n_fail++;                                                   \
            fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__);        \
            fprintf(stderr, __VA_ARGS__);                               \
            fprintf(stderr, "\n");                                      \
        }                                                               \
    } while (0)

static int wl(const char *fn, const char *frt, const char *en,
              const char *ert, char *out, size_t cap)
{
    return nb_wl_socket_path(fn, frt, en, ert, out, cap);
}

static void test_wayland(void)
{
    char o[256];
    char longname[300];

    /* no flag at all: the library's own environment fallback, untouched */
    CHECK(wl(NULL, NULL, "wayland-7", "/run/user/1", o, sizeof o) == 0 && !o[0],
          "no flag: out must stay empty so libwayland reads the environment");
    /* flag name + flag dir */
    CHECK(wl("wayland-1", "/rt", NULL, NULL, o, sizeof o) == 0 &&
          !strcmp(o, "/rt/wayland-1"), "name and --runtime-dir: got '%s'", o);
    /* the flag beats the variable, both for the name and the directory */
    CHECK(wl("wayland-1", "/rt", "wayland-9", "/other", o, sizeof o) == 0 &&
          !strcmp(o, "/rt/wayland-1"), "flags must beat WAYLAND_DISPLAY/XDG_RUNTIME_DIR: '%s'", o);
    /* name from the flag, directory from the environment fallback */
    CHECK(wl("wayland-1", NULL, "wayland-9", "/run/user/5", o, sizeof o) == 0 &&
          !strcmp(o, "/run/user/5/wayland-1"), "name flag + XDG_RUNTIME_DIR fallback: '%s'", o);
    /* --runtime-dir alone: the name falls back to WAYLAND_DISPLAY, then wayland-0 */
    CHECK(wl(NULL, "/rt", "wayland-3", NULL, o, sizeof o) == 0 &&
          !strcmp(o, "/rt/wayland-3"), "--runtime-dir alone, name from WAYLAND_DISPLAY: '%s'", o);
    CHECK(wl(NULL, "/rt", NULL, NULL, o, sizeof o) == 0 &&
          !strcmp(o, "/rt/wayland-0"), "--runtime-dir alone, no variable: wayland-0 expected, '%s'", o);
    /* an absolute name needs no runtime dir at all */
    CHECK(wl("/tmp/w/wayland-1", NULL, NULL, NULL, o, sizeof o) == 0 &&
          !strcmp(o, "/tmp/w/wayland-1"), "absolute name used as it is: '%s'", o);
    CHECK(wl("/tmp/w/wayland-1", "/rt", NULL, NULL, o, sizeof o) == 0 &&
          !strcmp(o, "/tmp/w/wayland-1"), "absolute name must not be joined to --runtime-dir: '%s'", o);
    /* a relative name with nowhere to look is refused, not guessed */
    CHECK(wl("wayland-1", NULL, NULL, NULL, o, sizeof o) == NB_WL_NO_RUNTIME_DIR,
          "relative name without any runtime dir must be refused");
    CHECK(wl("wayland-1", NULL, NULL, "", o, sizeof o) == NB_WL_NO_RUNTIME_DIR,
          "an empty XDG_RUNTIME_DIR is no directory");
    CHECK(wl("", "/rt", NULL, NULL, o, sizeof o) == NB_WL_EMPTY_NAME,
          "an empty --wayland-display must be refused");
    /* the empty variable counts as unset for the name */
    CHECK(wl(NULL, "/rt", "", NULL, o, sizeof o) == 0 && !strcmp(o, "/rt/wayland-0"),
          "an empty WAYLAND_DISPLAY is unset: '%s'", o);
    /* sun_path is 108 bytes including the NUL */
    memset(longname, 'a', sizeof longname);
    longname[0] = '/';
    longname[107] = '\0';
    CHECK(wl(longname, NULL, NULL, NULL, o, sizeof o) == 0,
          "a 107-byte path fits sun_path");
    longname[107] = 'a';
    longname[108] = '\0';
    CHECK(wl(longname, NULL, NULL, NULL, o, sizeof o) == NB_WL_TOO_LONG && !o[0],
          "a 108-byte path does not fit sun_path");
    CHECK(wl("/aaaaaaaa", NULL, NULL, NULL, o, 5) == NB_WL_TOO_LONG,
          "a path longer than the caller's buffer must be refused");
    CHECK(!o[0], "a path cut by the caller's buffer must not be left in out");
    CHECK(wl("/aaaaaaaa", NULL, NULL, NULL, o, 9) == NB_WL_TOO_LONG,
          "a path that leaves no room for the NUL must be refused");
    CHECK(wl("/aaaaaaaa", NULL, NULL, NULL, o, 10) == 0 && !strcmp(o, "/aaaaaaaa"),
          "a path that fits the buffer exactly");
    /* the same edge through the join, not just a literal absolute path */
    memset(longname, 'a', sizeof longname);
    longname[103] = '\0';          /* "/rt/" + 103 = 107 */
    CHECK(wl(longname, "/rt", NULL, NULL, o, sizeof o) == 0,
          "a joined 107-byte path fits sun_path");
    longname[103] = 'a';
    longname[104] = '\0';          /* "/rt/" + 104 = 108 */
    CHECK(wl(longname, "/rt", NULL, NULL, o, sizeof o) == NB_WL_TOO_LONG,
          "a joined 108-byte path does not fit sun_path");
    /* more than one thing at once */
    CHECK(wl(NULL, NULL, NULL, NULL, o, sizeof o) == 0 && !o[0],
          "no flag and no variable: still the library's business, not an error");
    CHECK(wl(NULL, "/rt", "wayland-3", "/other", o, sizeof o) == 0 &&
          !strcmp(o, "/rt/wayland-3"), "--runtime-dir beats XDG_RUNTIME_DIR for a name from WAYLAND_DISPLAY: '%s'", o);
    CHECK(wl("wayland-1", "", NULL, NULL, o, sizeof o) == NB_WL_NO_RUNTIME_DIR,
          "an empty --runtime-dir is no directory");
    CHECK(wl("", NULL, NULL, "/rt", o, sizeof o) == NB_WL_EMPTY_NAME,
          "an empty --wayland-display is refused even with a directory to join to");
    CHECK(wl(NULL, "/rt", "", "/other", o, sizeof o) == 0 &&
          !strcmp(o, "/rt/wayland-0"), "empty WAYLAND_DISPLAY + --runtime-dir: wayland-0 under the flag's dir: '%s'", o);
}

static void test_dir(void)
{
    uid_t trusted[2] = { 1000, 2000 };

    CHECK(nb_sockdir_verdict(S_IFDIR | 0700, 500, 500, NULL, 0) == NB_DIR_OK,
          "own 0700 directory");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0777, 500, 500, NULL, 0) == NB_DIR_OK,
          "our own directory may be as open as we made it");
    CHECK(nb_sockdir_verdict(S_IFREG | 0700, 500, 500, NULL, 0) == NB_DIR_NOT_DIR,
          "a regular file is not a socket directory");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0700, 600, 500, NULL, 0) == NB_DIR_FOREIGN_OWNER,
          "0700 owned by an untrusted uid must be refused (it can rename the socket)");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0777, 600, 500, NULL, 0) == NB_DIR_FOREIGN_OWNER,
          "open and owned by an untrusted uid must be refused");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0755, 0, 500, NULL, 0) == NB_DIR_OK,
          "root's non-writable directory is trusted");
    CHECK(nb_sockdir_verdict(S_IFDIR | 01777, 0, 500, NULL, 0) == NB_DIR_OK,
          "sticky /tmp-style directory is accepted");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0777, 0, 500, NULL, 0) == NB_DIR_FOREIGN_WIDE,
          "world-writable root directory without sticky must be refused");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0775, 0, 500, NULL, 0) == NB_DIR_FOREIGN_WIDE,
          "group-writable root directory without sticky must be refused");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0757, 0, 500, NULL, 0) == NB_DIR_FOREIGN_WIDE,
          "other-writable bit alone must be refused");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0770, 0, 500, NULL, 0) == NB_DIR_FOREIGN_WIDE,
          "group-write bit alone must be refused");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0700, 1000, 0, trusted, 2) == NB_DIR_OK,
          "sudo case: root broker, directory of an allowed uid, 0700");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0700, 2000, 0, trusted, 2) == NB_DIR_OK,
          "every entry of the allow list is trusted, not just the first");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0777, 1000, 0, trusted, 2) == NB_DIR_FOREIGN_WIDE,
          "an allowed uid's directory open to others must be refused");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0700, 1500, 0, trusted, 2) == NB_DIR_FOREIGN_OWNER,
          "a uid between two allowed ones is not allowed");

    CHECK(nb_sockdir_verdict(S_IFCHR | 0700, 500, 500, NULL, 0) == NB_DIR_NOT_DIR,
          "a device node is not a socket directory");
    CHECK(nb_sockdir_verdict(S_IFSOCK | 0700, 500, 500, NULL, 0) == NB_DIR_NOT_DIR,
          "a socket is not a socket directory");
    {
        uid_t three[3] = { 11, 22, 33 };

        CHECK(nb_sockdir_verdict(S_IFDIR | 0700, 33, 500, three, 3) == NB_DIR_OK,
              "the last of three allowed uids is trusted");
        CHECK(nb_sockdir_verdict(S_IFDIR | 0700, 22, 500, three, 3) == NB_DIR_OK,
              "the middle of three allowed uids is trusted");
    }
    CHECK(nb_sockdir_verdict(S_IFDIR | 0707, 0, 500, NULL, 0) == NB_DIR_FOREIGN_WIDE,
          "other-write with no group bits must be refused");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0702, 0, 500, NULL, 0) == NB_DIR_FOREIGN_WIDE,
          "other-write alone (0702) must be refused");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0720, 0, 500, NULL, 0) == NB_DIR_FOREIGN_WIDE,
          "group-write alone (0720) must be refused");
    CHECK(nb_sockdir_verdict(S_IFDIR | 01775, 0, 500, NULL, 0) == NB_DIR_OK,
          "sticky group-writable root directory is accepted");
    CHECK(nb_sockdir_verdict(S_IFDIR | 01777, 1000, 0, trusted, 2) == NB_DIR_OK,
          "sticky directory of an allowed uid is accepted");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0770, 500, 500, NULL, 0) == NB_DIR_OK,
          "our own group-writable directory is ours to share");
    CHECK(nb_sockdir_verdict(S_IFDIR | 01777, 500, 500, NULL, 0) == NB_DIR_OK,
          "our own sticky directory");
    CHECK(nb_sockdir_verdict(S_IFDIR | 0775, 500, 500, NULL, 0) == NB_DIR_OK,
          "our own 0775 directory");
    CHECK(nb_sockdir_create_mode(0600) == 0700, "default socket mode: directory 0700");
    CHECK(nb_sockdir_create_mode(0660) == 0710, "group socket mode adds only search for the group");
    CHECK(nb_sockdir_create_mode(0606) == 0701, "other socket mode adds only search for others");
    CHECK(nb_sockdir_create_mode(0666) == 0711, "both");
    CHECK(nb_sockdir_create_mode(0700) == 0700, "a socket mode with no group/other bits");
    CHECK(nb_sockdir_create_mode(0660) & 0700, "the owner's bits are always rwx");
    CHECK((nb_sockdir_create_mode(0666) & 0066) == 0, "no read or write is ever granted to group/others");
    CHECK(nb_sockdir_create_mode(0440) == 0710, "a read-only group bit still needs search");
}

static void test_uid(void)
{
    uid_t u = 99;

    CHECK(nb_sudo_uid_fallback(0, "1000", false, &u) && u == 1000,
          "root with SUDO_UID adds that uid");
    u = 99;
    CHECK(!nb_sudo_uid_fallback(0, "1000", true, &u) && u == 99,
          "--allow-uid given: SUDO_UID is not read");
    CHECK(!nb_sudo_uid_fallback(0, NULL, false, &u), "no SUDO_UID, nothing added");
    CHECK(!nb_sudo_uid_fallback(1000, "1234", false, &u),
          "a non-root broker never reads SUDO_UID");

    u = 99;
    CHECK(!nb_sudo_uid_fallback(0, "5", true, &u) && u == 99,
          "--allow-uid given and SUDO_UID=5: not read, out untouched");
    CHECK(nb_sudo_uid_fallback(0, "42", false, &u) && u == 42,
          "SUDO_UID=42 is the uid added");
    CHECK(!nb_sudo_uid_fallback(1, "5", false, &u) && !nb_sudo_uid_fallback(65534, "0", false, &u),
          "no non-root uid reads SUDO_UID");
    CHECK(!nb_sudo_uid_fallback(1000, NULL, false, &u) && !nb_sudo_uid_fallback(0, NULL, true, &u),
          "no SUDO_UID: nothing to add, whoever runs it");
    CHECK(nb_parse_uid("0", &u) == 0 && u == 0, "uid 0");
    CHECK(nb_parse_uid("1000", &u) == 0 && u == 1000, "uid 1000");
    CHECK(nb_parse_uid("4294967294", &u) == 0 && u == 4294967294u, "largest valid uid");
    CHECK(nb_parse_uid("4294967295", &u) != 0, "(uid_t)-1 is not a uid");
    CHECK(nb_parse_uid("99999999999999999999", &u) != 0, "overflow");
    CHECK(nb_parse_uid("", &u) != 0, "empty");
    CHECK(nb_parse_uid("-1", &u) != 0, "a sign is not a digit");
    CHECK(nb_parse_uid("+5", &u) != 0, "a plus sign is not a digit");
    CHECK(nb_parse_uid("12x", &u) != 0, "trailing garbage");
    CHECK(nb_parse_uid(" 5", &u) != 0, "leading space");
    CHECK(nb_parse_uid("alice", &u) != 0, "a name is --allow-user's job");
    CHECK(nb_parse_uid("0004294967295", &u) != 0, "(uid_t)-1 with leading zeros is still not a uid");
    CHECK(nb_parse_uid("4294967296", &u) != 0, "one past 32 bits");
    CHECK(nb_parse_uid(NULL, &u) != 0, "a NULL value");
    u = 99;
    CHECK(nb_parse_uid("", &u) != 0 && u == 99, "an empty value must not be read as uid 0");
    u = 99;
    CHECK(nb_parse_uid("1x", &u) != 0 && u == 99, "a refused value must leave out untouched");
    CHECK(nb_parse_uid("0000", &u) == 0 && u == 0, "leading zeros are digits");
    CHECK(nb_parse_uid("65534", &u) == 0 && u == 65534, "nobody");
}

static void test_conflict(void)
{
    static const char *const backends[] = { "auto", "wayland", "x11", "test" };
    /* which backends REFUSE each flag: wayland flags are wayland-only, x11 flags x11-only */
    static const char *const flags[] = { "--wayland-display", "--runtime-dir",
                                         "--x11-display", "--xauthority" };
    int f, b;

    for (f = 0; f < 4; f++) {
        for (b = 0; b < 4; b++) {
            struct nb_config c;
            const char *got;
            bool want_refused = !strcmp(backends[b], "test") ||
                                (f < 2 ? !strcmp(backends[b], "x11")
                                       : !strcmp(backends[b], "wayland"));

            memset(&c, 0, sizeof c);
            c.backend = backends[b];
            if (f == 0) c.wayland_display = "w";
            if (f == 1) c.runtime_dir = "/rt";
            if (f == 2) c.x11_display = ":0";
            if (f == 3) c.xauthority = "/x";
            got = nb_display_flag_conflict(&c);
            CHECK(want_refused ? (got && !strcmp(got, flags[f])) : !got,
                  "%s with --backend %s: %s", flags[f], backends[b],
                  want_refused ? "must be refused by name" : "must be accepted");
        }
    }
    {
        struct nb_config c;

        memset(&c, 0, sizeof c);
        CHECK(!nb_display_flag_conflict(&c), "no flag and no backend (NULL = auto)");
        c.backend = "auto";
        CHECK(!nb_display_flag_conflict(&c), "no flag, no conflict");
    }
}

int main(void)
{
    test_wayland();
    test_dir();
    test_uid();
    test_conflict();
    fprintf(stderr, "%u checks, %u failed\n", n_checks, n_fail);
    return n_fail ? 1 : 0;
}
