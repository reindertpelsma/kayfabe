# Mode-2 Interrupt / Completion Delivery

**Status:** design (2026-06-04). The cuCtxCreate blocker after the SIGSEGV+999
fixes: libcuda hangs polling `MC_SERVICE_INTERRUPTS` (0x20801702) waiting on an
interrupt-driven os-event our emulated GPU never delivers. See memory
[[mode2-cuctxcreate-999-diagnosis]].

## Driver chain (HW interrupt -> libcuda wait returns)

Traced in open 580.159.04 (`nvidia/nv.c` + `nv-kernel.o`):

```
GPU MSI-X  -> nvidia_isr() (nv.c:2826, top half)
                -> rm_isr() reads GPU/GSP intr status; flags bottom half
              nvidia_isr_kthread_bh -> rm_isr_bh()
                -> services GSP event queue / engine completion
                   -> osNotifyEvent() (RM core)
                      -> nv_post_event() (nv.c ~3983):
                           enqueue nv_event_t on nvlfp->event_data_head
                           wake_up_interruptible(&nvlfp->waitqueue)   <-- signal
libcuda: poll(nvidia_fd) -> nvidia_poll() (nv.c:2252)
           poll_wait(&nvlfp->waitqueue); return POLLIN once event_data_head!=NULL
```

No interrupt -> no wake_up -> poll never returns. In GSP mode the trigger is: GSP
posts an event to the CPU event buffer AND raises MSI-X; the guest ISR drains it.
Our emulator does neither -> guest RM falls back to libcuda pumping
MC_SERVICE_INTERRUPTS, which finds nothing pending -> infinite spin.

## How Mode-1 handled it (and why Mode-2 differs)

Mode-1's explicit event relay (nvkvm_frontend.c / virtio vq_evt) is STILL a TODO.
It didn't block Mode-1 because Mode-1 mostly AVOIDS interrupt waits: completions
are observed by libcuda polling a SEMAPHORE in mapped memory, made visible by the
GPA-window double-mmap (host GPU's semaphore writes land in guest-visible pages).
The stub creates host eventfds (stub_eventfd2 / NVKVM_DEV_EVENTFD) and vq_evt
exists, but the relay was never needed for compute.

Mode-2 runs the real nvidia.ko against the EMULATED GPU and genuinely blocks on
the interrupt-driven os-event here, so it must implement the delivery Mode-1
sidestepped.

## Design: host-completion -> guest-interrupt relay

Producer = host (forwarded work completes there); consumer = guest (needs an
emulated interrupt). One relay thread per emulated GPU / isolate:

    epoll_wait() on:
      - every host os-event eventfd (one per guest NV01_EVENT_OS_EVENT)
      - one control eventfd  (add/remove fds without races / missed wakeups)
    on host-eventfd readable:
      1. eventfd_read() drain
      2. map host-fd -> guest GSP event notification
      3. post the event to the guest GSP event buffer (serialize w/ RPC writer)
      4. raise emulated MSI-X (KVM irqfd preferred; else pci_msix_notify)
    on control-eventfd readable: rebuild the interest set
    guest: nvidia_isr -> rm_isr_bh -> GSP event -> osNotifyEvent -> wake libcuda

Rationale: epoll (O(1) over the many channel/event fds); control eventfd mutates
the set from the dispatch threads without lock-heavy hot path (lost-wakeup-safe,
same discipline as the cmd-buffer ring); irqfd for low-latency injection; one
thread per guest preserves [[mode2-isolation-cr3-key]] isolation; GSP event-post
mutex-serialized with the GSP RPC ring writer.

## Open items to resolve before coding
1. Verify emulated MSI-X is enabled by the guest and whether we ever raise it
   (guest /proc/interrupts + QEMU msix state). If guest enabled MSI-X but we never
   fire, it may have timed out into poll mode.
2. Exact GSP event-buffer format the guest RM drains in rm_isr_bh (read the GSP
   event-buffer setup RPC) so step 3 posts the correct notification.
3. Whether, given the data plane isn't forwarding GPFIFO work yet, the awaited
   completion is a forwarded host op or an emulator-faked one (the compute
   channels showed gp_put=0 at the hang).
