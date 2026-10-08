// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// uinput_mouse.c — a HOST test mouse + keyboard (docs/design/V3_DISPLAY.md §8.19): press
// CTRL+ALT+G (the broker's grab), then <n> single-count REL_X events of +1 every <us> µs, then
// CTRL+ALT+G again. With a udev hwdb MOUSE_DPI for its name, libinput normalises one count to
// 1000/dpi units — what a real high-resolution mouse hands a Wayland client as unaccelerated deltas.
//
//   uinput_mouse <name> <n> <us> [nochord]
#include <fcntl.h>
#include <linux/uinput.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

static void emit(int fd, int type, int code, int value)
{
    struct input_event ie;
    memset(&ie, 0, sizeof(ie));
    ie.type = type;
    ie.code = code;
    ie.value = value;
    if (write(fd, &ie, sizeof(ie)) != sizeof(ie)) {
        perror("write");
    }
}

static void key(int fd, int code, int v)
{
    emit(fd, EV_KEY, code, v);
    emit(fd, EV_SYN, SYN_REPORT, 0);
    usleep(30000);
}

int main(int argc, char **argv)
{
    if (argc < 4) {
        fprintf(stderr, "usage: uinput_mouse <name> <n> <us>\n");
        return 2;
    }
    int n = atoi(argv[2]), us = atoi(argv[3]);
    // argv[4] "nochord": motion only (the grab is already held, or is toggled by hand)
    int chord = !(argc > 4 && strcmp(argv[4], "nochord") == 0);
    int fd = open("/dev/uinput", O_WRONLY | O_NONBLOCK);
    if (fd < 0) {
        perror("/dev/uinput");
        return 1;
    }
    ioctl(fd, UI_SET_EVBIT, EV_KEY);
    ioctl(fd, UI_SET_KEYBIT, BTN_LEFT);
    ioctl(fd, UI_SET_KEYBIT, KEY_LEFTCTRL);
    ioctl(fd, UI_SET_KEYBIT, KEY_LEFTALT);
    ioctl(fd, UI_SET_KEYBIT, KEY_G);
    ioctl(fd, UI_SET_EVBIT, EV_REL);
    ioctl(fd, UI_SET_RELBIT, REL_X);
    ioctl(fd, UI_SET_RELBIT, REL_Y);
    struct uinput_setup s;
    memset(&s, 0, sizeof(s));
    s.id.bustype = BUS_USB;
    s.id.vendor = 0x1234;
    s.id.product = 0x5678;
    snprintf(s.name, sizeof(s.name), "%s", argv[1]);
    ioctl(fd, UI_DEV_SETUP, &s);
    ioctl(fd, UI_DEV_CREATE);
    sleep(2); // the compositor opens the new device
    if (chord) {
    key(fd, KEY_LEFTCTRL, 1);
    key(fd, KEY_LEFTALT, 1);
    key(fd, KEY_G, 1);
    key(fd, KEY_G, 0);
    key(fd, KEY_LEFTALT, 0);
    key(fd, KEY_LEFTCTRL, 0);
    }
    usleep(500000);
    for (int i = 0; i < n; i++) {
        emit(fd, EV_REL, REL_X, 1);
        emit(fd, EV_SYN, SYN_REPORT, 0);
        usleep(us);
    }
    usleep(500000);
    if (chord) {
    key(fd, KEY_LEFTCTRL, 1);
    key(fd, KEY_LEFTALT, 1);
    key(fd, KEY_G, 1);
    key(fd, KEY_G, 0);
    key(fd, KEY_LEFTALT, 0);
    key(fd, KEY_LEFTCTRL, 0);
    }
    sleep(1);
    ioctl(fd, UI_DEV_DESTROY);
    close(fd);
    printf("UINPUT sent %d REL_X +1 at %d us\n", n, us);
    return 0;
}
