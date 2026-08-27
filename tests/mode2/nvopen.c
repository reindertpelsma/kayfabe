/* Minimal RM client: open nvidiactl + nvidia0 to trigger rm_init_adapter
 * (GSP bootstrap) without needing version-matched userspace. */
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>
int main(void){
    int ctl = open("/dev/nvidiactl", O_RDWR);
    printf("open nvidiactl = %d (%s)\n", ctl, ctl<0?strerror(errno):"ok");
    int d0 = open("/dev/nvidia0", O_RDWR);
    printf("open nvidia0   = %d (%s)\n", d0, d0<0?strerror(errno):"ok");
    /* hold them open a moment so init completes/stalls */
    sleep(2);
    return 0;
}
