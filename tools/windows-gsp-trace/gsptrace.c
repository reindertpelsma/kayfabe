/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Experimental passive observer. No MMIO mapping, memory write or NVIDIA hooks.
 */
#include <ntifs.h>
#include <wdmsec.h>
#include "queue.h"
#define TAG 'tGsK'
#define FIFO_BYTES (32u*1024u*1024u)
#define SCAN_BYTES (1024u*1024u)
#define MAX_TABLES 8u
#define MAX_RANGES 4096u

typedef struct {
    uint64_t pa, ptes[KFGT_MAX_PAGES];
    ULONG pages, offset[2], size[2], active;
    KFGT_CURSOR cursor[2];
} TABLE;
typedef struct {
    FAST_MUTEX lock;
    KEVENT stop;
    HANDLE thread;
    PPHYSICAL_MEMORY_RANGE ranges;
    ULONG range_count, range_index;
    uint64_t range_offset;
    unsigned char *fifo, *scan, *copy_a, *copy_b, *scratch, *table_copy;
    ULONG head, used;
    TABLE tables[MAX_TABLES];
    KFGT_STATS stats;
} STATE;
static STATE *state;
static UNICODE_STRING link_name=RTL_CONSTANT_STRING(L"\\DosDevices\\KayfabeGspTrace");
static const GUID device_class={0x4f784520,0xf7f5,0x4195,{0x96,0x3b,0x53,0x2a,0x81,0x48,0xb1,0x2f}};
typedef char record_size_must_be_64[(sizeof(KFGT_RECORD)==64)?1:-1];
typedef char header_size_must_be_64[(sizeof(KFGT_FILE_HEADER)==64)?1:-1];
DRIVER_INITIALIZE DriverEntry;
DRIVER_UNLOAD Unload;
DRIVER_DISPATCH Dispatch;
static int stopping(STATE *s) { return KeReadStateEvent(&s->stop)!=0; }
static int ram(STATE *s,uint64_t pa,SIZE_T n) {
    ULONG i;
    if(!n || pa>UINT64_MAX-n) return 0;
    for(i=0;i<s->range_count;i++) {
        uint64_t lo=(uint64_t)s->ranges[i].BaseAddress.QuadPart;
        uint64_t bytes=(uint64_t)s->ranges[i].NumberOfBytes.QuadPart;
        if(pa>=lo && pa-lo<bytes && n<=bytes-(pa-lo)) return 1;
    }
    return 0;
}
static int read_ram(STATE *s,uint64_t pa,void *out,SIZE_T n) {
    MM_COPY_ADDRESS from; SIZE_T got=0; NTSTATUS status;
    if(!ram(s,pa,n)) return 0;
    from.PhysicalAddress.QuadPart=(LONGLONG)pa;
    status=MmCopyMemory(out,from,n,MM_COPY_MEMORY_PHYSICAL,&got);
    if(!NT_SUCCESS(status) || got!=n) { InterlockedIncrement64((LONG64*)&s->stats.read_failures); return 0; }
    return 1;
}
static void fifo_copy(STATE *s,const void *data,ULONG n) {
    ULONG at=(s->head+s->used)%FIFO_BYTES, first=min(n,FIFO_BYTES-at);
    RtlCopyMemory(s->fifo+at,data,first);
    if(n>first) RtlCopyMemory(s->fifo,(const unsigned char *)data+first,n-first);
    s->used+=n;
}
static void emit(void *context,const KFGT_RECORD *r,const unsigned char *payload) {
    STATE *s=context; ULONG n=(ULONG)sizeof(*r)+r->payload_bytes;
    ExAcquireFastMutex(&s->lock);
    if(n>FIFO_BYTES-s->used) s->stats.dropped++;
    else { fifo_copy(s,r,sizeof(*r));fifo_copy(s,payload,r->payload_bytes);s->stats.recorded++; }
    s->stats.buffered_bytes=s->used;
    ExReleaseFastMutex(&s->lock);
}
static int mapping_current(STATE *s,TABLE *t) {
    return read_ram(s,t->pa,s->table_copy,t->pages*8) &&
           RtlCompareMemory(s->table_copy,t->ptes,t->pages*8)==t->pages*8;
}
static int copy_queue(STATE *s,TABLE *t,ULONG direction,unsigned char *out) {
    ULONG i,first=t->offset[direction]/KFGT_PAGE,pages=t->size[direction]/KFGT_PAGE;
    for(i=0;i<pages;i++) {
        if(stopping(s) || !read_ram(s,t->ptes[first+i],out+i*KFGT_PAGE,KFGT_PAGE)) return 0;
    }
    return 1;
}
static void poll_table(STATE *s,TABLE *t) {
    ULONG d; uint64_t old_gaps,old_invalid;
    if(!mapping_current(s,t)) { t->active=0;return; }
    for(d=0;d<2;d++) {
        KFGT_QUEUE header;
        if(!copy_queue(s,t,d,s->copy_a) || !copy_queue(s,t,d,s->copy_b)) return;
        if(!mapping_current(s,t)) { t->active=0;return; }
        if(RtlCompareMemory(s->copy_a,s->copy_b,t->size[d])!=t->size[d]) {
            InterlockedIncrement64((LONG64*)&s->stats.unstable_snapshots);continue;
        }
        if(!kf_queue_header(s->copy_a,t->size[d],&header) || header.size!=t->size[d]) {
            t->active=0;return;
        }
        old_gaps=t->cursor[d].gaps;old_invalid=t->cursor[d].invalid;
        (void)kf_queue_records(s->copy_a,t->size[d],s->scratch,&t->cursor[d],d,t->pa,
                             (uint64_t)KeQueryPerformanceCounter(NULL).QuadPart,emit,s);
        InterlockedAdd64((LONG64*)&s->stats.observed_sequence_gaps,(LONG64)(t->cursor[d].gaps-old_gaps));
        InterlockedAdd64((LONG64*)&s->stats.invalid_elements,(LONG64)(t->cursor[d].invalid-old_invalid));
    }
}
static void candidate(STATE *s,uint64_t pa) {
    TABLE *t=NULL; KFGT_QUEUE cmd,reply; ULONG i,reply_index,pages;
    for(i=0;i<MAX_TABLES;i++) {
        if(s->tables[i].active && s->tables[i].pa==pa) return;
        if(!s->tables[i].active && !t) t=&s->tables[i];
    }
    if(!t) return;
    InterlockedIncrement64((LONG64*)&s->stats.candidates);
    if(!read_ram(s,pa,s->table_copy,KFGT_PAGE) || kf_u64(s->table_copy)!=pa) return;
    RtlZeroMemory(t,sizeof(*t));
    for(i=0;i<KFGT_MAX_PAGES;i++) t->ptes[i]=kf_u64(s->table_copy+i*8);
    if(!read_ram(s,t->ptes[1],s->copy_a,KFGT_PAGE) || !kf_queue_header(s->copy_a,KFGT_PAGE,&cmd)) return;
    reply_index=1+cmd.size/KFGT_PAGE;
    if(reply_index>=KFGT_MAX_PAGES || !read_ram(s,t->ptes[reply_index],s->copy_a,KFGT_PAGE) ||
       !kf_queue_header(s->copy_a,KFGT_PAGE,&reply)) return;
    pages=reply_index+reply.size/KFGT_PAGE;
    if(pages>KFGT_MAX_PAGES) return; /* Only the published single-page page table profile. */
    for(i=0;i<pages;i++) {
        ULONG j;
        if((t->ptes[i]&(KFGT_PAGE-1)) || !t->ptes[i] || !ram(s,t->ptes[i],KFGT_PAGE)) return;
        for(j=0;j<i;j++) if(t->ptes[j]==t->ptes[i]) return;
    }
    t->pa=pa;t->pages=pages;t->offset[0]=KFGT_PAGE;t->offset[1]=reply_index*KFGT_PAGE;
    t->size[0]=cmd.size;t->size[1]=reply.size;
    if(!mapping_current(s,t)) return;
    t->active=1;InterlockedIncrement64((LONG64*)&s->stats.attached_tables);
    poll_table(s,t);
}
static void scan_step(STATE *s) {
    uint64_t base,size,pa,left; ULONG n,i;
    if(s->range_index>=s->range_count) {
        s->range_index=0;s->range_offset=0;
        InterlockedIncrement64((LONG64*)&s->stats.scan_passes);
    }
    base=(uint64_t)s->ranges[s->range_index].BaseAddress.QuadPart;
    size=(uint64_t)s->ranges[s->range_index].NumberOfBytes.QuadPart;
    if(s->range_offset>=size) { s->range_index++;s->range_offset=0;return; }
    pa=base+s->range_offset;left=size-s->range_offset;n=(ULONG)min(left,SCAN_BYTES);
    if(read_ram(s,pa,s->scan,n)) {
        for(i=0;i+KFGT_PAGE<=n;i+=KFGT_PAGE) {
            if(stopping(s)) return;
            if(kf_u64(s->scan+i)==pa+i && kf_u64(s->scan+i+8)!=0) candidate(s,pa+i);
        }
    }
    s->range_offset+=n;InterlockedAdd64((LONG64*)&s->stats.scanned_bytes,n);
}
static void worker(void *context) {
    STATE *s=context; LARGE_INTEGER delay; ULONG i,chunks=0;
    delay.QuadPart=-10000; /* 1 ms requested wait, actual scheduling is measured by QPC. */
    while(!stopping(s)) {
        /* One bounded discovery chunk per pass, then every known queue. */
        scan_step(s);
        for(i=0;i<MAX_TABLES && !stopping(s);i++) if(s->tables[i].active) poll_table(s,&s->tables[i]);
        if(++chunks>=64) {
            chunks=0;(void)KeWaitForSingleObject(&s->stop,Executive,KernelMode,FALSE,&delay);
        }
    }
    PsTerminateSystemThread(STATUS_SUCCESS);
}
static void release_state(STATE *s) {
    if(!s) return;
    if(s->thread) { KeSetEvent(&s->stop,0,FALSE);ZwWaitForSingleObject(s->thread,FALSE,NULL);ZwClose(s->thread); }
    if(s->ranges) ExFreePool(s->ranges);
    if(s->fifo) ExFreePoolWithTag(s->fifo,TAG);
    if(s->scan) ExFreePoolWithTag(s->scan,TAG);
    if(s->copy_a) ExFreePoolWithTag(s->copy_a,TAG);
    if(s->copy_b) ExFreePoolWithTag(s->copy_b,TAG);
    if(s->scratch) ExFreePoolWithTag(s->scratch,TAG);
    if(s->table_copy) ExFreePoolWithTag(s->table_copy,TAG);
    ExFreePoolWithTag(s,TAG);
}
NTSTATUS Dispatch(PDEVICE_OBJECT device,PIRP irp) {
    PIO_STACK_LOCATION stack=IoGetCurrentIrpStackLocation(irp);
    NTSTATUS status=STATUS_INVALID_DEVICE_REQUEST; ULONG_PTR bytes=0; STATE *s=state;
    UNREFERENCED_PARAMETER(device);
    if(stack->MajorFunction==IRP_MJ_CREATE) status=stack->FileObject->FileName.Length?STATUS_OBJECT_NAME_INVALID:STATUS_SUCCESS;
    else if(stack->MajorFunction==IRP_MJ_CLOSE || stack->MajorFunction==IRP_MJ_CLEANUP) status=STATUS_SUCCESS;
    else if(stack->MajorFunction==IRP_MJ_DEVICE_CONTROL && stack->Parameters.DeviceIoControl.InputBufferLength==0) {
        ULONG code=stack->Parameters.DeviceIoControl.IoControlCode;
        ULONG capacity=stack->Parameters.DeviceIoControl.OutputBufferLength;
        unsigned char *out=irp->AssociatedIrp.SystemBuffer;
        if(code==KFGT_IOCTL_STOP && capacity==0) {
            KeSetEvent(&s->stop,0,FALSE);ZwWaitForSingleObject(s->thread,FALSE,NULL);status=STATUS_SUCCESS;
        } else if(code==KFGT_IOCTL_STATS && capacity>=sizeof(s->stats)) {
            ExAcquireFastMutex(&s->lock);RtlCopyMemory(out,&s->stats,sizeof(s->stats));ExReleaseFastMutex(&s->lock);
            bytes=sizeof(s->stats);status=STATUS_SUCCESS;
        } else if(code==KFGT_IOCTL_READ && capacity>=sizeof(KFGT_RECORD)+KFGT_MAX_MESSAGE && capacity<=4*1024*1024) {
            ULONG take,first;
            ExAcquireFastMutex(&s->lock);
            take=min(capacity,s->used);first=min(take,FIFO_BYTES-s->head);
            if(first) RtlCopyMemory(out,s->fifo+s->head,first);
            if(take>first) RtlCopyMemory(out+first,s->fifo,take-first);
            s->head=(s->head+take)%FIFO_BYTES;s->used-=take;s->stats.buffered_bytes=s->used;
            ExReleaseFastMutex(&s->lock);bytes=take;status=STATUS_SUCCESS;
        } else if(code==KFGT_IOCTL_READ || code==KFGT_IOCTL_STATS) status=STATUS_BUFFER_TOO_SMALL;
    }
    irp->IoStatus.Status=status;irp->IoStatus.Information=bytes;IoCompleteRequest(irp,IO_NO_INCREMENT);return status;
}
void Unload(PDRIVER_OBJECT driver) {
    IoDeleteSymbolicLink(&link_name);release_state(state);state=NULL;IoDeleteDevice(driver->DeviceObject);
}
NTSTATUS DriverEntry(PDRIVER_OBJECT driver,PUNICODE_STRING registry_path) {
    UNICODE_STRING name=RTL_CONSTANT_STRING(L"\\Device\\KayfabeGspTrace");
    UNICODE_STRING sddl=RTL_CONSTANT_STRING(L"D:P(A;;GA;;;SY)(A;;GA;;;BA)");
    PDEVICE_OBJECT device=NULL; NTSTATUS status; STATE *s; LARGE_INTEGER frequency; ULONG i;
    OBJECT_ATTRIBUTES attrs;
    UNREFERENCED_PARAMETER(registry_path);
    status=IoCreateDeviceSecure(driver,0,&name,FILE_DEVICE_UNKNOWN,FILE_DEVICE_SECURE_OPEN,TRUE,&sddl,&device_class,&device);
    if(!NT_SUCCESS(status)) return status;
    s=ExAllocatePool2(POOL_FLAG_NON_PAGED,sizeof(*s),TAG);
    if(!s) { IoDeleteDevice(device);return STATUS_INSUFFICIENT_RESOURCES; }
    ExInitializeFastMutex(&s->lock);KeInitializeEvent(&s->stop,NotificationEvent,FALSE);
    s->ranges=MmGetPhysicalMemoryRanges();
    if(s->ranges) for(i=0;i<MAX_RANGES && s->ranges[i].NumberOfBytes.QuadPart;i++) s->range_count++;
    s->fifo=ExAllocatePool2(POOL_FLAG_NON_PAGED,FIFO_BYTES,TAG);
    s->scan=ExAllocatePool2(POOL_FLAG_NON_PAGED,SCAN_BYTES,TAG);
    s->copy_a=ExAllocatePool2(POOL_FLAG_NON_PAGED,KFGT_MAX_QUEUE,TAG);
    s->copy_b=ExAllocatePool2(POOL_FLAG_NON_PAGED,KFGT_MAX_QUEUE,TAG);
    s->scratch=ExAllocatePool2(POOL_FLAG_NON_PAGED,KFGT_MAX_MESSAGE,TAG);
    s->table_copy=ExAllocatePool2(POOL_FLAG_NON_PAGED,KFGT_PAGE,TAG);
    if(!s->ranges || !s->range_count || s->range_count==MAX_RANGES || !s->fifo || !s->scan || !s->copy_a || !s->copy_b || !s->scratch || !s->table_copy) {
        release_state(s);IoDeleteDevice(device);return STATUS_INSUFFICIENT_RESOURCES;
    }
    s->stats.version=KFGT_ABI;s->stats.bytes=sizeof(s->stats);
    s->stats.started_qpc=(uint64_t)KeQueryPerformanceCounter(&frequency).QuadPart;s->stats.qpc_frequency=(uint64_t)frequency.QuadPart;
    state=s;
    for(i=0;i<=IRP_MJ_MAXIMUM_FUNCTION;i++) driver->MajorFunction[i]=Dispatch;
    driver->DriverUnload=Unload;
    InitializeObjectAttributes(&attrs,NULL,OBJ_KERNEL_HANDLE,NULL,NULL);
    status=PsCreateSystemThread(&s->thread,THREAD_ALL_ACCESS,&attrs,NULL,NULL,worker,s);
    if(NT_SUCCESS(status)) status=IoCreateSymbolicLink(&link_name,&name);
    if(!NT_SUCCESS(status)) { release_state(s);state=NULL;IoDeleteDevice(device);return status; }
    device->Flags&=~DO_DEVICE_INITIALIZING;
    return STATUS_SUCCESS;
}
