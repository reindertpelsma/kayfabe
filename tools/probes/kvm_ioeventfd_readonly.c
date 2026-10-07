/* SPDX-License-Identifier: MIT
 * GPU-free x86 KVM mechanism check, NOT a GPU performance benchmark.
 * cc -O2 -Wall -Wextra -Werror kvm_ioeventfd_readonly.c -o <temporary-binary>
 * timeout 10 <temporary-binary>
 * Verifies read-only memslot reads, token-specific kicks, unknown-value fallback,
 * and deassign/reassign. No device access other than /dev/kvm; no host changes.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/kvm.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/eventfd.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

#define PAGE 4096
#define DB_GPA 0x2000
#define TOKEN_A UINT32_C(0x10203040)
#define TOKEN_B UINT32_C(0xa0b0c0d0)
#define UNKNOWN UINT32_C(0x55667788)
#define SENTINEL UINT32_C(0xdecafbad)

static void fail(const char *why)
{
    fprintf(stderr, "KVM_IOEVENTFD_READONLY=FAIL reason=%s errno=%d (%s)\n",
            why, errno, strerror(errno));
    exit(1);
}

static int checked_ioctl(int fd, unsigned long request, unsigned long arg)
{
    int rc = ioctl(fd, request, arg);
    if (rc < 0)
        fail("ioctl");
    return rc;
}

static void route(int vm, int fd, uint32_t token, int remove)
{
    struct kvm_ioeventfd io = {
        .datamatch = token, .addr = DB_GPA, .len = 4, .fd = fd,
        .flags = KVM_IOEVENTFD_FLAG_DATAMATCH |
                 (remove ? KVM_IOEVENTFD_FLAG_DEASSIGN : 0),
    };
    checked_ioctl(vm, KVM_IOEVENTFD, (unsigned long)&io);
}

static uint64_t drain(int fd)
{
    uint64_t count = 0;
    ssize_t n;
    do {
        n = read(fd, &count, sizeof(count));
    } while (n < 0 && errno == EINTR);
    if (n < 0 && errno == EAGAIN)
        return 0;
    if (n != sizeof(count))
        fail("eventfd read");
    return count;
}

static void append_store(uint8_t **code, uint32_t token)
{
    /* Real mode: operand-size override; mov dword ptr [disp16], imm32. */
    const uint8_t op[] = {0x66, 0xc7, 0x06, DB_GPA & 255, DB_GPA >> 8};
    memcpy(*code, op, sizeof(op));
    *code += sizeof(op);
    memcpy(*code, &token, sizeof(token));
    *code += sizeof(token);
}

static void scenario(const char *name, int vcpu, struct kvm_run *run,
                     const uint32_t *backing, int a, int b, int routed_a, int routed_b)
{
    struct kvm_regs regs = {.rip = 0, .rflags = 2};
    unsigned exits = 0;
    uint32_t expected[3];
    unsigned n_expected = 0;
    if (!routed_a) expected[n_expected++] = TOKEN_A;
    if (!routed_b) expected[n_expected++] = TOKEN_B;
    expected[n_expected++] = UNKNOWN;
    checked_ioctl(vcpu, KVM_SET_REGS, (unsigned long)&regs);
    for (;;) {
        if (ioctl(vcpu, KVM_RUN, 0) < 0) {
            if (errno == EINTR) continue;
            fail("KVM_RUN");
        }
        if (run->exit_reason == KVM_EXIT_HLT)
            break;
        if (run->exit_reason != KVM_EXIT_MMIO || !run->mmio.is_write ||
            run->mmio.phys_addr != DB_GPA || run->mmio.len != 4 || exits >= n_expected)
            fail("unexpected exit or MMIO shape");
        uint32_t token;
        memcpy(&token, run->mmio.data, sizeof(token));
        if (token != expected[exits++])
            fail("wrong fallback token/order");
        /* Re-enter to complete the emulated write; never alter the read-only backing. */
    }
    checked_ioctl(vcpu, KVM_GET_REGS, (unsigned long)&regs);
    uint64_t count_a = drain(a), count_b = drain(b);
    if (exits != n_expected || count_a != (unsigned)routed_a ||
        count_b != (unsigned)routed_b || (uint32_t)regs.rax != SENTINEL || *backing != SENTINEL)
        fail("count, fallback, or read-only readback mismatch");
    printf("CASE %s PASS userspace_mmio=%u event_a=%llu event_b=%llu readback=%08x\n",
           name, exits, (unsigned long long)count_a, (unsigned long long)count_b,
           (uint32_t)regs.rax);
}

int main(void)
{
    int kvm = open("/dev/kvm", O_RDWR | O_CLOEXEC);
    if (kvm < 0) {
        puts("KVM_IOEVENTFD_READONLY=SKIP reason=no-access-to-/dev/kvm");
        return 77;
    }
    if (checked_ioctl(kvm, KVM_GET_API_VERSION, 0) != KVM_API_VERSION)
        fail("KVM API version");
    if (!checked_ioctl(kvm, KVM_CHECK_EXTENSION, KVM_CAP_READONLY_MEM) ||
        !checked_ioctl(kvm, KVM_CHECK_EXTENSION, KVM_CAP_IOEVENTFD)) {
        puts("KVM_IOEVENTFD_READONLY=SKIP reason=missing-capability");
        close(kvm);
        return 77;
    }
    int vm = checked_ioctl(kvm, KVM_CREATE_VM, 0);
    checked_ioctl(vm, KVM_SET_TSS_ADDR, 0xfffbd000);
    uint8_t *code = mmap(NULL, PAGE, PROT_READ | PROT_WRITE,
                         MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    uint32_t *backing = mmap(NULL, PAGE, PROT_READ | PROT_WRITE,
                             MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (code == MAP_FAILED || backing == MAP_FAILED)
        fail("mmap guest pages");
    struct kvm_userspace_memory_region mem = {
        .slot = 0, .guest_phys_addr = 0, .memory_size = PAGE,
        .userspace_addr = (uintptr_t)code,
    };
    checked_ioctl(vm, KVM_SET_USER_MEMORY_REGION, (unsigned long)&mem);
    mem.slot = 1;
    mem.flags = KVM_MEM_READONLY;
    mem.guest_phys_addr = DB_GPA;
    mem.userspace_addr = (uintptr_t)backing;
    checked_ioctl(vm, KVM_SET_USER_MEMORY_REGION, (unsigned long)&mem);
    *backing = SENTINEL;
    uint8_t *p = code;
    append_store(&p, TOKEN_A);
    append_store(&p, TOKEN_B);
    append_store(&p, UNKNOWN);
    const uint8_t finish[] = {0x66, 0xa1, DB_GPA & 255, DB_GPA >> 8, 0xf4};
    memcpy(p, finish, sizeof(finish)); /* mov eax,[disp16]; hlt */
    int vcpu = checked_ioctl(vm, KVM_CREATE_VCPU, 0);
    int run_size = checked_ioctl(kvm, KVM_GET_VCPU_MMAP_SIZE, 0);
    if ((size_t)run_size < sizeof(struct kvm_run))
        fail("KVM run size");
    struct kvm_run *run = mmap(NULL, run_size, PROT_READ | PROT_WRITE, MAP_SHARED, vcpu, 0);
    if (run == MAP_FAILED)
        fail("mmap KVM run");
    struct kvm_sregs sregs;
    checked_ioctl(vcpu, KVM_GET_SREGS, (unsigned long)&sregs);
    sregs.cs.base = 0;
    sregs.cs.selector = 0;
    sregs.ds.base = 0;
    sregs.ds.selector = 0;
    checked_ioctl(vcpu, KVM_SET_SREGS, (unsigned long)&sregs);
    int a = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
    int b = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
    if (a < 0 || b < 0)
        fail("eventfd");
    scenario("inline", vcpu, run, backing, a, b, 0, 0);
    route(vm, a, TOKEN_A, 0);
    route(vm, b, TOKEN_B, 0);
    scenario("datamatch", vcpu, run, backing, a, b, 1, 1);
    route(vm, a, TOKEN_A, 1);
    scenario("deassigned-a", vcpu, run, backing, a, b, 0, 1);
    route(vm, a, TOKEN_A, 0);
    scenario("reassigned-a", vcpu, run, backing, a, b, 1, 1);
    route(vm, a, TOKEN_A, 1);
    route(vm, b, TOKEN_B, 1);
    close(a);
    close(b);
    munmap(run, run_size);
    close(vcpu);
    close(vm);
    close(kvm);
    munmap(code, PAGE);
    munmap(backing, PAGE);
    puts("KVM_IOEVENTFD_READONLY=PASS cases=4 gpu_performance=UNMEASURED");
    return 0;
}
