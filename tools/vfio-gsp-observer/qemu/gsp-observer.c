/* SPDX-License-Identifier: GPL-2.0-or-later
 * Explicit diagnostic profile only. Producer callbacks never perform file I/O.
 */
#include "qemu/osdep.h"
#include "qemu/atomic.h"
#include "qemu/error-report.h"
#include "qemu/main-loop.h"
#include "qemu/thread.h"
#include "qemu/rcu.h"
#include "qemu/timer.h"
#include "qapi/error.h"
#include "system/address-spaces.h"
#include "system/system.h"
#include "pci.h"
#include "gsp-observer.h"
#include "observer.h"

#define FIFO_BYTES (32u * 1024u * 1024u)
#define SOURCE_CAP (64u * 1024u * 1024u)
#define RECORD_CAP 100000u
#define DURATION_NS (300ULL * 1000000000ULL)
QEMU_BUILD_BUG_ON(sizeof(KFGT_RECORD) != 64);
QEMU_BUILD_BUG_ON(sizeof(KFGT_FILE_HEADER) != 64);
struct VFIOGspObserver {
    VG_OBSERVER core;
    VFIOPCIDevice *device;
    FILE *output;
    QemuThread writer;
    QemuEvent ready;
    Notifier exit;
    Error *unplug_blocker;
    uint8_t *fifo;
    uint64_t produced, consumed; /* one BQL producer, one writer */
    uint64_t start_ns, accepted_bytes, accepted_records, dropped;
    uint64_t trigger;
    bool stopped, io_error, limited;
};

static uint64_t now_ns(void)
{
    return qemu_clock_get_ns(QEMU_CLOCK_REALTIME);
}

static bool plain_ram(MemoryRegion *mr)
{
    /* VFIO BAR mmap regions also have mr->ram=true. They are device memory,
     * and memcpy through those mappings would perform an unintended GPU read. */
    return memory_region_is_ram(mr) && !memory_region_is_rom(mr) &&
           !memory_region_is_ram_device(mr) &&
           !memory_region_is_protected(mr) && !memory_region_has_guest_memfd(mr);
}

static int guest_ram(void *opaque, uint64_t pa, void *out, size_t bytes)
{
    VFIOGspObserver *s = opaque;
    AddressSpace *as = pci_device_iommu_address_space(PCI_DEVICE(s->device));
    size_t copied = 0;
    /* First profile is identity guest DMA. Never guess an IOVA translation. */
    if (as != &address_space_memory || pa > UINT64_MAX - bytes) {
        return 0;
    }
    RCU_READ_LOCK_GUARD();
    while (copied < bytes) {
        hwaddr offset, length = bytes - copied;
        MemoryRegion *mr = address_space_translate(as, pa + copied, &offset,
                                                   &length, false,
                                                   MEMTXATTRS_UNSPECIFIED);
        if (!plain_ram(mr) ||
            !length || offset > memory_region_size(mr) ||
            length > memory_region_size(mr) - offset) {
            return 0;
        }
        if (out) {
            memcpy((uint8_t *)out + copied,
                   (uint8_t *)memory_region_get_ram_ptr(mr) + offset, length);
        }
        copied += length;
    }
    return 1;
}

static void ring_copy(VFIOGspObserver *s, uint64_t at, void *data,
                      size_t bytes, bool into_ring)
{
    size_t index = at % FIFO_BYTES;
    size_t first = MIN(bytes, FIFO_BYTES - index);
    if (into_ring) {
        memcpy(s->fifo + index, data, first);
        memcpy(s->fifo, (uint8_t *)data + first, bytes - first);
    } else {
        memcpy(data, s->fifo + index, first);
        memcpy((uint8_t *)data + first, s->fifo, bytes - first);
    }
}

static void record(void *opaque, uint64_t generation, const KFGT_RECORD *r,
                   const unsigned char *payload)
{
    VFIOGspObserver *s = opaque;
    uint64_t at = s->produced, read = qatomic_load_acquire(&s->consumed);
    uint64_t meta[2] = { generation, s->trigger };
    size_t size = sizeof(meta) + sizeof(*r) + r->payload_bytes;
    if (s->accepted_records >= RECORD_CAP ||
        sizeof(KFGT_FILE_HEADER) + s->accepted_bytes + sizeof(*r) +
            r->payload_bytes > SOURCE_CAP) {
        s->limited = true;
        s->dropped++;
        return;
    }
    if (size > FIFO_BYTES - (at - read)) {
        s->dropped++;
        return;
    }
    ring_copy(s, at, meta, sizeof(meta), true);
    ring_copy(s, at + sizeof(meta), (void *)r, sizeof(*r), true);
    ring_copy(s, at + sizeof(meta) + sizeof(*r), (void *)payload,
              r->payload_bytes, true);
    s->accepted_bytes += sizeof(*r) + r->payload_bytes;
    s->accepted_records++;
    qatomic_store_release(&s->produced, at + size);
    qemu_event_set(&s->ready);
}

static void write_record(FILE *out, uint64_t generation, uint64_t trigger, const KFGT_RECORD *r,
                         const uint8_t *payload, char *hex)
{
    static const char digits[] = "0123456789abcdef";
    unsigned i;
    for (i = 0; i < r->payload_bytes; i++) {
        hex[i * 2] = digits[payload[i] >> 4];
        hex[i * 2 + 1] = digits[payload[i] & 15];
    }
    hex[r->payload_bytes * 2] = 0;
    fprintf(out, "{\"kind\":\"record\",\"generation\":%" PRIu64
            ",\"trigger\":%" PRIu64 ",\"magic\":%u,\"header_bytes\":%u,\"payload_bytes\":%u,"
            "\"direction\":%u,\"qpc\":%" PRIu64 ",\"table_pa\":%" PRIu64
            ",\"queue_sequence\":%u,\"rpc_sequence\":%u,\"rpc_function\":%u,"
            "\"rpc_result\":%u,\"flags\":%u,\"missing_before\":%u,"
            "\"rpc_version\":%u,\"reserved\":0,\"payload_hex\":\"%s\"}\n",
            generation, trigger, r->magic, r->header_bytes, r->payload_bytes, r->direction,
            r->qpc, r->table_pa, r->queue_sequence, r->rpc_sequence, r->rpc_function,
            r->rpc_result, r->flags, r->missing_before, r->rpc_version, hex);
}

static bool writer_drained(VFIOGspObserver *s, uint64_t read)
{
    /* Acquire stop before reloading produced: an earlier empty observation may
     * predate the final record that the producer published before stopping. */
    return qatomic_load_acquire(&s->stopped) &&
           qatomic_load_acquire(&s->produced) == read;
}

static void *writer(void *opaque)
{
    VFIOGspObserver *s = opaque;
    g_autofree uint8_t *payload = g_malloc(KFGT_MAX_MESSAGE);
    g_autofree char *hex = g_malloc(KFGT_MAX_MESSAGE * 2 + 1);
    GChecksum *hash = g_checksum_new(G_CHECKSUM_SHA256);
    GChecksum *observations = g_checksum_new(G_CHECKSUM_SHA256);
    KFGT_FILE_HEADER header = { .magic = KFGT_FILE_MAGIC, .version = 1,
        .header_bytes = 64, .record_header_bytes = 64,
        .qpc_frequency = 1000000000, .started_qpc = s->start_ns, .flags = 1 };
    uint64_t count = 0, bytes = sizeof(header);
    FILE *out = s->output;
    g_checksum_update(hash, (const uint8_t *)&header, sizeof(header));
    g_checksum_update(observations, (const uint8_t *)&header, sizeof(header));
    fprintf(out, "{\"schema\":\"kayfabe-gsp-text/1\",\"kind\":\"header\","
            "\"capture_complete\":false,\"selection\":\"all\",\"observer\":\"vfio-bootstrap-580\","
            "\"magic\":%u,\"version\":1,\"header_bytes\":64,\"record_header_bytes\":64,"
            "\"qpc_frequency\":1000000000,\"started_qpc\":%" PRIu64
            ",\"flags\":1,\"reserved\":0,\"reserved2\":[0,0,0]}\n",
            KFGT_FILE_MAGIC, s->start_ns);
    for (;;) {
        uint64_t read = s->consumed, available;
        qemu_event_reset(&s->ready);
        available = qatomic_load_acquire(&s->produced);
        if (read == available) {
            if (writer_drained(s, read)) {
                break;
            }
            if (qatomic_load_acquire(&s->stopped)) {
                continue; /* final producer publication followed our first load */
            }
            qemu_event_wait(&s->ready);
            continue;
        }
        KFGT_RECORD r;
        uint64_t meta[2];
        ring_copy(s, read, meta, sizeof(meta), false);
        ring_copy(s, read + sizeof(meta), &r, sizeof(r), false);
        assert(r.payload_bytes <= KFGT_MAX_MESSAGE);
        assert(available - read >= sizeof(meta) + sizeof(r) + r.payload_bytes);
        ring_copy(s, read + sizeof(meta) + sizeof(r), payload,
                  r.payload_bytes, false);
        qatomic_store_release(&s->consumed,
                              read + sizeof(meta) + sizeof(r) + r.payload_bytes);
        g_checksum_update(hash, (const uint8_t *)&r, sizeof(r));
        g_checksum_update(hash, payload, r.payload_bytes);
        g_checksum_update(observations, (const uint8_t *)meta, sizeof(meta));
        g_checksum_update(observations, (const uint8_t *)&r, sizeof(r));
        g_checksum_update(observations, payload, r.payload_bytes);
        write_record(out, meta[0], meta[1], &r, payload, hex);
        bytes += sizeof(r) + r.payload_bytes;
        count++;
        if ((count % 32 == 0 && fflush(out)) || ferror(out)) {
            qatomic_store_release(&s->io_error, true);
        }
    }
    /* stopped is acquired: producer counters and the core now remain immutable. */
    VG_STATS *st = &s->core.stats;
    fprintf(out, "{\"kind\":\"footer\",\"file_export_complete\":%s,\"capture_complete\":false,"
            "\"records\":%" PRIu64 ",\"source_records\":%" PRIu64 ",\"omitted_records\":0,"
            "\"source_bytes\":%" PRIu64 ",\"source_sha256\":\"%s\","
            "\"observations_sha256\":\"%s\","
            "\"driver_stats\":{\"triggers\":%" PRIu64 ",\"bootstrap_attempts\":%" PRIu64
            ",\"attached_tables\":%" PRIu64 ",\"invalid_bootstrap\":%" PRIu64
            ",\"uninitialized_headers\":%" PRIu64 ",\"invalid_headers\":%" PRIu64
            ",\"read_failures\":%" PRIu64 ",\"unstable_snapshots\":%" PRIu64
            ",\"mapping_changed\":%" PRIu64 ",\"recorded\":%" PRIu64
            ",\"dropped\":%" PRIu64 ",\"sequence_gaps\":%" PRIu64
            ",\"invalid_elements\":%" PRIu64 ",\"limited\":%s,\"buffered_bytes\":0}}\n",
            s->io_error ? "false" : "true", count, count, bytes,
            g_checksum_get_string(hash), g_checksum_get_string(observations),
            st->triggers, st->bootstrap_attempts,
            st->attached, st->invalid_bootstrap, st->uninitialized_headers,
            st->invalid_headers, st->read_failures, st->unstable, st->mapping_changed,
            count, s->dropped, st->gaps, st->invalid_elements,
            s->limited ? "true" : "false");
    if (fflush(out) || ferror(out)) {
        qatomic_store_release(&s->io_error, true);
    }
    if (fclose(out)) {
        qatomic_store_release(&s->io_error, true);
    }
    g_checksum_free(hash);
    g_checksum_free(observations);
    return NULL;
}

void vfio_gsp_close(VFIOPCIDevice *vdev)
{
    VFIOGspObserver *s = vdev->gsp_observer;
    if (!s) {
        return;
    }
    vdev->gsp_observer = NULL;
    qdev_del_unplug_blocker(DEVICE(vdev), s->unplug_blocker);
    error_free(s->unplug_blocker);
    qemu_remove_exit_notifier(&s->exit);
    qatomic_store_release(&s->stopped, true);
    qemu_event_set(&s->ready);
    /* Teardown only, after VM stop or device removal; never an MMIO callback. */
    qemu_thread_join(&s->writer);
    if (s->io_error) {
        error_report("VFIO GSP observer output failed; capture is incomplete");
    }
    qemu_event_destroy(&s->ready);
    g_free(s->fifo);
    g_free(s);
}

static void observer_exit(Notifier *n, void *data)
{
    VFIOGspObserver *s = container_of(n, VFIOGspObserver, exit);
    vfio_gsp_close(s->device);
}

bool vfio_gsp_open(VFIOPCIDevice *vdev, Error **errp)
{
    VFIOGspObserver *s;
    int fd;
    if (!vdev->gsp_observer_path) {
        return true;
    }
    if (HOST_BIG_ENDIAN || !vdev->no_kvm_intx ||
        !vdev->no_kvm_msi || !vdev->no_kvm_msix || !vdev->no_kvm_ioeventfd ||
        !vdev->no_vfio_ioeventfd || vdev->vbasedev.enable_migration != ON_OFF_AUTO_OFF ||
        vdev->vbasedev.mdev ||
        pci_get_word(PCI_DEVICE(vdev)->config + PCI_VENDOR_ID) != 0x10de ||
        pci_device_iommu_address_space(PCI_DEVICE(vdev)) != &address_space_memory) {
        error_setg(errp, "GSP observer requires little-endian host, NVIDIA PCI, identity guest DMA, "
                   "enable-migration=off, x-no-vfio-ioeventfd=on and "
                   "x-no-kvm-{intx,msi,msix,ioeventfd}=on");
        return false;
    }
    fd = open(vdev->gsp_observer_path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0600);
    if (fd < 0) {
        error_setg_errno(errp, errno, "Cannot create exclusive GSP trace");
        return false;
    }
    s = g_new0(VFIOGspObserver, 1);
    s->output = fdopen(fd, "w");
    if (!s->output) {
        error_setg_errno(errp, errno, "Cannot open GSP trace stream");
        close(fd);
        g_free(s);
        return false;
    }
    s->device = vdev;
    s->fifo = g_malloc(FIFO_BYTES);
    s->start_ns = now_ns();
    vg_init(&s->core, guest_ram, record, s);
    qemu_event_init(&s->ready, false);
    s->exit.notify = observer_exit;
    qemu_add_exit_notifier(&s->exit);
    error_setg(&s->unplug_blocker, "GSP diagnostic capture must drain at QEMU exit; hot unplug disabled");
    qdev_add_unplug_blocker(DEVICE(vdev), s->unplug_blocker);
    vdev->gsp_observer = s;
    qemu_thread_create(&s->writer, "vfio-gsp-write", writer, s, QEMU_THREAD_JOINABLE);
    return true;
}

static bool active(VFIOGspObserver *s, uint64_t now)
{
    if (!s || s->limited || qatomic_load_acquire(&s->io_error)) {
        return false;
    }
    if (now - s->start_ns > DURATION_NS) {
        s->limited = true;
        return false;
    }
    return true;
}

void vfio_gsp_irq(VFIOPCIDevice *vdev)
{
    uint64_t now;
    if (!vdev->gsp_observer) {
        return;
    }
    assert(bql_locked());
    now = now_ns();
    if (active(vdev->gsp_observer, now)) {
        vdev->gsp_observer->trigger = 3;
        vg_poll(&vdev->gsp_observer->core, now);
    }
}

void vfio_gsp_region(VFIODevice *dev, int region, hwaddr addr, uint64_t data,
                     unsigned size, bool write)
{
    VFIOPCIDevice *vdev = container_of(dev, VFIOPCIDevice, vbasedev);
    uint64_t now;
    if (region != 0 || !vdev->gsp_observer) {
        return;
    }
    assert(bql_locked());
    if (addr != 0x110040 && addr != 0x110044 && addr != 0x110004 &&
        addr != 0x110008 && !(write && size == 4 && addr >= 0x110c00 &&
                            addr < 0x110c40 && !(addr & 7))) {
        return;
    }
    now = now_ns();
    if (active(vdev->gsp_observer, now)) {
        vdev->gsp_observer->trigger = addr == 0x110040 || addr == 0x110044 ? 1 :
                                     addr >= 0x110c00 ? 2 : 4;
        vg_mmio(&vdev->gsp_observer->core, addr, data, size, write, now);
    }
}

void vfio_gsp_reset(VFIOPCIDevice *vdev)
{
    if (vdev->gsp_observer) {
        assert(bql_locked());
        vg_reset(&vdev->gsp_observer->core);
    }
}
