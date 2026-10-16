// uinput_in click X Y W H | key F9 : inject through /dev/uinput (virtual absolute pointer + keyboard) so the compositor sees real input
#include <linux/uinput.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
static int fd;
static void ev(int type, int code, int val) { struct input_event e; memset(&e, 0, sizeof e); e.type = type; e.code = code; e.value = val; if (write(fd, &e, sizeof e) < 0) perror("write"); }
int main(int argc, char **argv) {
  fd = open("/dev/uinput", O_WRONLY | O_NONBLOCK); if (fd < 0) { perror("uinput"); return 2; }
  ioctl(fd, UI_SET_EVBIT, EV_KEY); ioctl(fd, UI_SET_KEYBIT, KEY_F9); ioctl(fd, UI_SET_KEYBIT, BTN_LEFT); ioctl(fd, UI_SET_EVBIT, EV_ABS); ioctl(fd, UI_SET_ABSBIT, ABS_X); ioctl(fd, UI_SET_ABSBIT, ABS_Y);
  struct uinput_abs_setup ax = { .code = ABS_X, .absinfo = { .minimum = 0, .maximum = 32767 } }, ay = { .code = ABS_Y, .absinfo = { .minimum = 0, .maximum = 32767 } };
  ioctl(fd, UI_ABS_SETUP, &ax); ioctl(fd, UI_ABS_SETUP, &ay);
  struct uinput_setup us; memset(&us, 0, sizeof us); us.id.bustype = BUS_USB; us.id.vendor = 0x1234; us.id.product = 0x5678; strcpy(us.name, "claude-perf2-virtual-input");
  ioctl(fd, UI_DEV_SETUP, &us); ioctl(fd, UI_DEV_CREATE); usleep(1500000);
  if (!strcmp(argv[1], "click")) {
    int x = atoi(argv[2]), y = atoi(argv[3]), W = atoi(argv[4]), H = atoi(argv[5]);
    ev(EV_ABS, ABS_X, x * 32767 / W); ev(EV_ABS, ABS_Y, y * 32767 / H); ev(EV_SYN, SYN_REPORT, 0); usleep(300000);
    ev(EV_KEY, BTN_LEFT, 1); ev(EV_SYN, SYN_REPORT, 0); usleep(60000); ev(EV_KEY, BTN_LEFT, 0); ev(EV_SYN, SYN_REPORT, 0);
  } else {
    ev(EV_KEY, KEY_F9, 1); ev(EV_SYN, SYN_REPORT, 0); usleep(80000); ev(EV_KEY, KEY_F9, 0); ev(EV_SYN, SYN_REPORT, 0);
  }
  usleep(500000); ioctl(fd, UI_DEV_DESTROY); close(fd); return 0;
}
