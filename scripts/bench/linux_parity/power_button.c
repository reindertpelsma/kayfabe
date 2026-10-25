/* Diskless guest cleanup: translate its ACPI KEY_POWER event into guest poweroff. */
#define _DEFAULT_SOURCE
#include <fcntl.h>
#include <glob.h>
#include <linux/input.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/reboot.h>
#include <unistd.h>

int main(int argc, char **argv) {
    char vendor[128] = {0};
    FILE *command = fopen("/sys/class/dmi/id/sys_vendor", "r");
    if (command) {
        if (!fgets(vendor, sizeof vendor, command)) vendor[0] = 0;
        fclose(command);
    }
    if (argc != 2 || strcmp(argv[1], "--guest") || strncmp(vendor, "QEMU", 4)) {
        puts("LX2_POWER_BUTTON_GUEST_ONLY");
        return 1;
    }
    glob_t paths = {0};
    struct pollfd fds[32];
    unsigned count = 0;
    if (glob("/dev/input/event*", 0, NULL, &paths)) return 1;
    for (size_t i = 0; i < paths.gl_pathc && count < 32; i++) {
        int fd = open(paths.gl_pathv[i], O_RDONLY | O_NONBLOCK);
        if (fd < 0) continue;
        unsigned long bits[(KEY_MAX + 8 * sizeof(long)) / (8 * sizeof(long))] = {0};
        if (ioctl(fd, EVIOCGBIT(EV_KEY, sizeof bits), bits) < 0 ||
            !(bits[KEY_POWER / (8 * sizeof(long))] &
              (1UL << (KEY_POWER % (8 * sizeof(long)))))) {
            close(fd);
            continue;
        }
        fds[count++] = (struct pollfd){.fd = fd, .events = POLLIN};
    }
    globfree(&paths);
    if (!count) {
        puts("LX2_POWER_BUTTON_MISSING");
        return 1;
    }
    printf("LX2_POWER_BUTTON_READY devices=%u\n", count);
    fflush(stdout);
    if (!freopen("/dev/ttyS0", "w", stdout)) return 1;
    for (;;) {
        if (poll(fds, count, -1) < 0) return 1;
        for (unsigned i = 0; i < count; i++) {
            struct input_event event;
            while (read(fds[i].fd, &event, sizeof event) == sizeof event) {
                if (event.type == EV_KEY && event.code == KEY_POWER && event.value == 1) {
                    puts("LX2_POWER_BUTTON_POWEROFF");
                    fflush(stdout);
                    sync();
                    reboot(RB_POWER_OFF);
                    perror("LX2_POWER_BUTTON_REBOOT_FAILED");
                    return 1;
                }
            }
        }
    }
}
