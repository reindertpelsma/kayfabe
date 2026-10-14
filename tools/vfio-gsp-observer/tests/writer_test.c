/* SPDX-License-Identifier: GPL-2.0-or-later
 * Include production writer; section GC removes unused VFIO integration.
 */
#include "../qemu/gsp-observer.c"
#include "system/ramblock.h"

static void put32(uint8_t *p, uint32_t n)
{
    memcpy(p, &n, sizeof(n)); /* supported little-endian host profile */
}

int main(int argc, char **argv)
{
    GspObserver *s = g_new0(GspObserver, 1);
    uint8_t payload[8192] = { 0 };
    KFGT_RECORD r = { .magic = KFGT_RECORD_MAGIC, .header_bytes = 64,
        .payload_bytes = sizeof(payload), .table_pa = 0x3000,
        .rpc_function = 72, .rpc_version = 0x03000000, .flags = 1 };
    unsigned i, j;
    bool full;
    RAMBlock block = { .guest_memfd = -1 };
    MemoryRegion region = { .ram = true, .ram_block = &block };
    /* Real QEMU MemoryRegion/RAMBlock predicates, including VFIO's RAM-device
     * combination. No duplicate stand-in implementation of QEMU predicates. */
    assert(plain_ram(&region));
    region.ram_device = true; assert(!plain_ram(&region)); region.ram_device = false;
    region.readonly = true; assert(!plain_ram(&region)); region.readonly = false;
    region.ram = false; assert(!plain_ram(&region)); region.ram = true;
    block.flags = RAM_PROTECTED; assert(!plain_ram(&region)); block.flags = 0;
    block.guest_memfd = 3; assert(!plain_ram(&region)); block.guest_memfd = -1;
    assert(argc == 3);
    full = strcmp(argv[1], "full") == 0;
    s->output = fopen(argv[2], "w");
    assert(s->output);
    s->fifo = g_malloc(FIFO_BYTES);
    s->trigger = 2;
    qemu_event_init(&s->ready, false);
    /* Exercise exact caps and saturation without racing a consumer. */
    s->accepted_records = RECORD_CAP;
    record(s, 1, &r, payload);
    assert(s->limited && s->dropped == 1 && s->produced == 0);
    s->limited = false; s->accepted_records = 0;
    s->accepted_bytes = SOURCE_CAP;
    record(s, 1, &r, payload);
    assert(s->limited && s->dropped == 2 && s->produced == 0);
    s->limited = false; s->accepted_bytes = 0;
    s->produced = FIFO_BYTES;
    record(s, 1, &r, payload);
    assert(!s->limited && s->dropped == 3 && s->produced == FIFO_BYTES);
    s->produced = 0; s->dropped = 0;
    /* Writer saw an empty queue, then producer committed a last record+stop. */
    qatomic_store_release(&s->produced, 1);
    qatomic_store_release(&s->stopped, true);
    assert(!writer_drained(s, 0));
    assert(writer_drained(s, 1));
    s->produced = 0; s->stopped = false;
    qemu_thread_create(&s->writer, "writer-test", writer, s, QEMU_THREAD_JOINABLE);
    for (i = 0; i < (full ? 6500u : 64u); i++) {
        uint32_t sum = 0;
        r.queue_sequence = i;
        r.qpc = i;
        r.flags = i == 0 ? 3 : 1;
        put32(payload + 32, 0);
        put32(payload + 36, i);
        put32(payload + 40, 2);
        put32(payload + 48, r.rpc_version);
        put32(payload + 52, KFGT_RPC_SIGNATURE);
        put32(payload + 56, sizeof(payload) - 48);
        put32(payload + 60, r.rpc_function);
        for (j = 0; j < sizeof(payload); j += 4) { sum ^= kf_u32(payload + j); }
        put32(payload + 32, sum);
        /* Test harness pacing only. Production producer never waits. */
        while (s->produced - qatomic_load_acquire(&s->consumed) > FIFO_BYTES / 2) {
            g_usleep(1000);
        }
        record(s, 1, &r, payload);
    }
    qatomic_store_release(&s->stopped, true);
    qemu_event_set(&s->ready);
    qemu_thread_join(&s->writer);
    assert(s->dropped == 0);
    assert(s->produced == s->consumed);
    assert(s->io_error == !full);
    if (full) { assert(s->produced > FIFO_BYTES); }
    qemu_event_destroy(&s->ready);
    g_free(s->fifo); g_free(s);
    puts("VFIO GSP writer wrap/caps/drain/error test passed");
    return 0;
}
