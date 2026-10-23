// SPDX-License-Identifier: GPL-2.0-only
/* The owning guest userspace process: receive the real channel-error syscall result. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    char args[256];
    int fd, on, rc, saved, expected;
    if (argc!=5 || (strcmp(argv[4],"on") && strcmp(argv[4],"off"))) return 2;
    on = !strcmp(argv[4],"on");
    if (snprintf(args,sizeof(args),"uuid=%s run=%s fault=%d",argv[2],argv[3],on)>=(int)sizeof(args)) return 2;
    fd = open(argv[1],O_RDONLY|O_CLOEXEC);
    if (fd<0) return 2;
    errno=0;
    rc=syscall(SYS_finit_module,fd,args,0);
    saved=errno;
    close(fd);
    expected = on ? (rc==-1 && saved==EIO) : rc==0;
    printf("KF_RC_OWNER run=%s pid=%ld errno=%d channel_error=%d\n",argv[3],(long)getpid(),saved,rc==-1 && saved==EIO);
    if (!on && rc==0 && syscall(SYS_delete_module,"kf_rc_probe",0)!=0) expected=0;
    return expected ? 0 : 1;
}
