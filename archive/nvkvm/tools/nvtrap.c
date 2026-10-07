// SPDX-License-Identifier: GPL-2.0
/*
 * nvtrap.c — LD_PRELOAD that traps the FIRST CPU touch of every page of every
 * /dev/nvidia* / /dev/dri mmap, to find exactly which mapped GPU-memory offsets
 * a closed userspace lib reads to make a decision (#84: libGLX computes "5 OS
 * events" on the host but "1" in the guest, from state not in the ioctl stream).
 *
 * Mechanism (per the debugging idea: "let it segfault on the region so you know
 * exactly which pointers are accessed, pause, dump"):
 *   - wrap mmap(); for an nvidia/dri fd, after the real mmap succeeds, flip the
 *     region to PROT_NONE so any access faults;
 *   - a SIGSEGV handler logs the faulting page (region base + offset + the fd's
 *     device + first 16 bytes once readable), then restores that ONE page to
 *     its original prot and returns — so the access re-executes and the program
 *     continues, but every page's first touch is recorded, in order.
 *
 * Diff the guest vs host logs: the first region/offset whose touch order or
 * content differs is where the divergent decision is read from.
 *
 * Build:  cc -O2 -fPIC -shared -o nvtrap.so nvtrap.c -ldl
 * Run:    LD_PRELOAD=./nvtrap.so NVTRAP=/tmp/trap.log \
 *         VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/nvidia_icd.json vulkaninfo --summary
 *
 * Notes: only PROT_READ/WRITE data maps are trapped (not PROT_EXEC lib maps).
 * MAP_SHARED GPU doorbell/USERD pages are CPU-written; trapping+restoring the
 * page on first touch is safe (we never deny after the first fault).
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <signal.h>
#include <sys/mman.h>

static void *(*real_mmap)(void *, size_t, int, int, int, off_t);
static FILE *logf;
static long  pgsz;

#define MAX_REGIONS 256
static struct region {
	uintptr_t base, end;
	int       prot;       /* original prot to restore */
	char      dev[24];
} regions[MAX_REGIONS];
static int nregions;

static int devname(int fd, char *buf, size_t n)
{
	char l[64];
	snprintf(l, sizeof l, "/proc/self/fd/%d", fd);
	ssize_t r = readlink(l, buf, n - 1);
	if (r < 0) r = 0;
	buf[r] = 0;
	return strstr(buf, "/dev/nvidia") != NULL || strstr(buf, "/dev/dri/") != NULL;
}
static const char *tag(const char *p){ const char *s=strrchr(p,'/'); return s?s+1:p; }

static void segv_handler(int sig, siginfo_t *si, void *uctx)
{
	(void)sig; (void)uctx;
	uintptr_t a = (uintptr_t)si->si_addr;
	for (int i = 0; i < nregions; i++) {
		if (a >= regions[i].base && a < regions[i].end) {
			uintptr_t page = a & ~(uintptr_t)(pgsz - 1);
			/* restore this page first so we can read it + let the
			 * faulting instruction re-execute. */
			mprotect((void *)page, pgsz, regions[i].prot);
			unsigned char *b = (unsigned char *)page;
			fprintf(logf, "TOUCH %-10s base=%#lx off=%#lx",
				tag(regions[i].dev), regions[i].base,
				(unsigned long)(a - regions[i].base));
			if (regions[i].prot & PROT_READ) {
				fprintf(logf, " head:");
				for (int k = 0; k < 16; k++) fprintf(logf, "%02x", b[k]);
			}
			fputc('\n', logf);
			fflush(logf);
			return;
		}
	}
	/* Not one of ours — restore default behaviour (re-raise). */
	signal(SIGSEGV, SIG_DFL);
}

__attribute__((constructor)) static void init(void)
{
	real_mmap = dlsym(RTLD_NEXT, "mmap");
	const char *p = getenv("NVTRAP");
	logf = p ? fopen(p, "w") : stderr;
	if (!logf) logf = stderr;
	pgsz = sysconf(_SC_PAGESIZE);
	struct sigaction sa = {0};
	sa.sa_sigaction = segv_handler;
	sa.sa_flags = SA_SIGINFO;
	sigemptyset(&sa.sa_mask);
	sigaction(SIGSEGV, &sa, NULL);
}

void *mmap(void *addr, size_t len, int prot, int flags, int fd, off_t off)
{
	void *r = real_mmap(addr, len, prot, flags, fd, off);
	char dev[64];
	if (r != MAP_FAILED && fd >= 0 && (prot & (PROT_READ | PROT_WRITE)) &&
	    !(prot & PROT_EXEC) && devname(fd, dev, sizeof dev) &&
	    nregions < MAX_REGIONS) {
		fprintf(logf, "MMAP  %-10s base=%p len=%#zx prot=%#x off=%#lx\n",
			tag(dev), r, len, prot, (unsigned long)off);
		fflush(logf);
		regions[nregions].base = (uintptr_t)r;
		regions[nregions].end  = (uintptr_t)r + len;
		regions[nregions].prot = prot;
		snprintf(regions[nregions].dev, sizeof regions[nregions].dev,
			 "%s", tag(dev));
		nregions++;
		mprotect(r, len, PROT_NONE);   /* arm the trap */
	}
	return r;
}
