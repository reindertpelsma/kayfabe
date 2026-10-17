/* SPDX-License-Identifier: GPL-2.0 OR Apache-2.0 */
/*
 * nvkvm_broker_proto.h — the display-broker wire protocol (version 2).
 *
 * TWO DIRECTIONS, AND THEY ARE NOT SYMMETRIC.
 *
 * The broker runs as the user's desktop session and owns the window, the
 * keyboard grab and the compositor connection.  The VMM is sandboxed and is
 * treated as HOSTILE: this project has produced multiple guest→host bugs in
 * the QEMU-side VMM, including an arbitrary-address pin primitive reachable
 * from a guest kernel, and the whole point of the broker is that such a VMM
 * can be contained without giving up native zero-copy rendering.
 *
 *   broker → VMM   fixed-size EVENT packets (input, pacing, state).  The VMM
 *                  is the untrusted side here, so this direction carries no
 *                  fds, no lengths and nothing the broker has to be careful
 *                  about.
 *
 *   VMM → broker   fixed-size COMMAND records, two of which carry a single
 *                  fd as SCM_RIGHTS ancillary data: ATTACH (a dma-buf, or a
 *                  memfd on the shm tier) and CURSOR's SET (a memfd).  THIS is
 *                  the attack surface.  Everything in it is validated by the
 *                  privileged side against the real buffer and against what
 *                  the GPU actually advertises — see src/broker/README.md §3.
 *
 * Both records are fixed size, little-endian, and their sizes are compile-time
 * constants on both sides.  There is no length field anywhere and no
 * variable-length payload, so neither side ever parses a length an attacker
 * chose.  A short or oversized read is a framing error and MUST be treated as
 * a disconnect, never resynced: a resync is a parser, and a parser is the
 * thing this shape exists to avoid.
 *
 * The wire format is IDENTICAL on every backend.  A Wayland broker and an X11
 * broker are indistinguishable to the VMM — that is deliberate, so the VMM
 * cannot make a decision (or a bug) conditional on the host's display stack.
 */
#ifndef NVKVM_BROKER_PROTO_H
#define NVKVM_BROKER_PROTO_H

#include <stddef.h>
#include <stdint.h>

#define NVKVM_BROKER_PROTO_VERSION 2u

/* ── broker → VMM: events ────────────────────────────────────────────────── */

#define NVKVM_BROKER_PKT_SIZE      24u

/* Event types.  Values are frozen; append only. */
enum {
    NVKVM_BROKER_EV_HELLO     = 1,  /* w0=proto version, w1=capability bits.
                                     * Always the first packet on a connection */
    NVKVM_BROKER_EV_SURFACE   = 2,  /* x=width y=height of the broker's window.
                                     * Sent at attach and on every resize.     */
    NVKVM_BROKER_EV_FRAME     = 3,  /* the display is ready for another frame
                                     * (wl frame callback / PresentComplete).
                                     * This is the pacing signal; it is NOT a
                                     * request, and ignoring it only costs fps */
    NVKVM_BROKER_EV_RELEASE   = 4,  /* w0,w1 = low,high 32 bits of the buffer
                                     * id (its dma-buf inode).  The display is
                                     * no longer reading that buffer.          */
    NVKVM_BROKER_EV_KEY       = 5,  /* x=Linux evdev keycode (KEY_*), y=down   */
    NVKVM_BROKER_EV_BTN       = 6,  /* x=Linux evdev button (BTN_*),  y=down   */
    NVKVM_BROKER_EV_ABS       = 7,  /* x,y = position; w0,w1 = the range.
                                     * Sent ONLY when not grabbed and only
                                     * while the pointer is over the window.   */
    NVKVM_BROKER_EV_REL       = 8,  /* x=dx y=dy.  Sent ONLY when grabbed.     */
    NVKVM_BROKER_EV_WHEEL     = 9,  /* x=vertical detents, y=horizontal        */
    NVKVM_BROKER_EV_GRAB      = 10, /* x=1 grab on, 0 off.  The client should
                                     * switch pointing device on this edge.    */
    NVKVM_BROKER_EV_FOCUS     = 11, /* x=1 active, 0 inactive.  While 0 the
                                     * broker sends no KEY/BTN/ABS/REL at all  */
    NVKVM_BROKER_EV_POINTER   = 12, /* x=1 pointer over the window, 0 not      */
    NVKVM_BROKER_EV_BYE       = 13, /* broker is going away; x=reason code     */
    /*
     * EV_CLOSE — THE USER CLOSED THE DISPLAY.  The X button, or the
     * compositor's own close request (Alt+F4, a window-list close).
     *
     * It states a fact and requests nothing: the broker knows nothing about
     * VMs and has no business deciding what closing a window means for one.
     * The VMM applies its own policy -- an ACPI powerdown, a prompt, a
     * snapshot, or nothing at all.
     *
     * DELIBERATELY NOT A DIALOG IN THE BROKER.  The broker is the privileged
     * process holding the keyboard grab; putting dialog UI, hit-testing and
     * the state machine behind it in there buys attack surface for a decision
     * it cannot make anyway.
     *
     * The broker does NOT exit on sending this.  It exits when the client
     * disconnects (or keeps the window with --persist), so the VMM decides how
     * and when the display goes away.
     */
    NVKVM_BROKER_EV_CLOSE     = 14,
    /*
     * EV_CLIPBOARD — one fixed-size chunk of host clipboard text on its way to
     * the guest.  See "Clipboard framing" below for why it is chunked rather
     * than length-prefixed.
     */
    NVKVM_BROKER_EV_CLIPBOARD = 15,

    /*
     * Answer to NVKVM_BROKER_CMD_QUERY_FORMAT.  The question is echoed back so
     * the client needs no outstanding-query state and cannot mismatch a reply
     * to a request:
     *     x  = 1 the display can show it, 0 it cannot
     *     y  = the fourcc that was asked about
     *     w0 = modifier, low 32 bits
     *     w1 = modifier, high 32 bits
     *
     * x=1 is the same judgement ATTACH would make, taken through the same code
     * path -- including the opaque-twin substitution -- so an accepted answer
     * cannot be followed by a rejected frame.
     *
     * ALSO SENT UNSOLICITED, with x=0, when the display refuses an import of a
     * pair it had advertised (Wayland: the asynchronous probe's `failed`; X11:
     * DRI3 answering the import with an error).  A relay must therefore accept
     * a LATER x=0 for a pair it already holds x=1 for -- a downgrade is news --
     * and must never treat an unsolicited x=1 as an upgrade.  After the
     * refusal, QUERY_FORMAT for that pair answers x=0 too.
     */
    NVKVM_BROKER_EV_FORMAT    = 16,
    /*
     * EV_DEVICE — WHICH GPU THE DISPLAY RENDERS ON.  Advertised by
     * NVKVM_BROKER_CAP_DEVICE.  Sent once at attach, as the packet AFTER the
     * priming FRAME (so the five-packet handshake prefix an older client reads
     * is byte-for-byte what it always was), and again, unsolicited, whenever
     * the display server reports a different device.
     *
     *     x  = NVKVM_BROKER_DEVICE_F_* (0 = the broker does not know)
     *     y  = 0, reserved.  Deliberately NOT "where the answer came from":
     *          that would name the backend, and the rule at the top of this
     *          file is that the VMM cannot tell Wayland from X11.
     *     w0 = major, w1 = minor of the DRM character device
     *
     * WHY.  A relay that can tell "the compositor imports on the same GPU the
     * guest renders on" from "it scans out on another one" can choose its
     * present path up front instead of discovering a cross-GPU host by having
     * every ATTACH refused.  It is a HINT for choosing, never a gate the
     * broker relies on: every ATTACH is still validated as if it were not
     * sent.
     *
     * It discloses the host compositor's DRM device number to the VMM.  That
     * is deliberate and small -- a major:minor names which of the host's GPUs
     * draws the desktop, nothing about what is on it -- and it is written down
     * in docs/internal/broker-design.md §3 so it is a decision, not a leak.
     */
    NVKVM_BROKER_EV_DEVICE    = 17,
    /*
     * EV_TICK -- THE DISPLAY'S REFRESH, AND THE LOCK-STEP FRAME REQUEST.
     * Advertised by NVKVM_BROKER_CAP_TICK and sent ONLY to a VMM that opted in
     * with NVKVM_BROKER_CLIENT_TICK in CMD_CAPS.  Laid over the standard event
     * packet as struct nvkvm_broker_tick_pkt, below.  One per DECIMATED host
     * refresh while the window is visible, whether or not anything is wanted:
     * it is a heartbeat, so a static desktop does not look like a dead broker.
     *
     * A tick whose refresh word carries NVKVM_BROKER_TICK_F_WANT_FRAME OPENS
     * the request numbered tick_seq: the broker has received (and the
     * compositor has presented or discarded) the previous frame and can take
     * the next one.  The VMM answers it with exactly one COMMIT whose `seq` is
     * that number, or with one CMD_NO_CHANGE naming it.  The broker never has
     * more than one request open, and opens the next only after the answer
     * (and, for a frame, its presentation) or after
     * NVKVM_BROKER_TICK_REQ_TIMEOUT_NS.
     *
     * The VMM is the side that must not trust this: a broker tick at the wrong
     * moment, out of sequence or in a flood is the VMM's to ignore and count.
     */
    NVKVM_BROKER_EV_TICK      = 18,
};

/* EV_DEVICE.x */
#define NVKVM_BROKER_DEVICE_F_KNOWN  (1u << 0) /* w0:w1 are meaningful       */
#define NVKVM_BROKER_DEVICE_F_RENDER (1u << 1) /* w0:w1 is a RENDER node
                                                * (renderD*), either as the
                                                * display reported it or as
                                                * the broker resolved the
                                                * reported node through
                                                * sysfs.  Clear with KNOWN set
                                                * means it could not be
                                                * resolved; compare with care */
/* ── Clipboard framing ───────────────────────────────────────────────────── *
 *
 * Clipboard content is the first VARIABLE-LENGTH thing this protocol carries,
 * and docs/internal/audit-broker-client-2026-08-25.md names "no length field
 * anywhere to lie about" as the reason framing cannot be overrun by
 * construction.  Adding a length prefix would hand that property back.
 *
 * So it is not length-prefixed.  Every message stays EXACTLY the same size as
 * every other message of its direction, and content is split into fixed chunks
 * with an explicit end marker.  The reader still reads a constant number of
 * bytes and never consults a client-supplied number to decide how much to read.
 *
 * `info` is not a length field in that sense: it says how much of an
 * ALREADY-READ, fixed-size array is meaningful.  It is bounds-checked against
 * the array's compile-time size, and a lie in it cannot move the read cursor,
 * cannot allocate, and cannot reach past the struct.  The bound that matters --
 * total size -- is enforced as a CHUNK COUNT the receiver keeps itself, never
 * as a number the sender supplies.
 *
 * TEXT ONLY, UTF-8.  No images, no file lists, no arbitrary MIME: every one of
 * those is a decoder in the privileged process.
 */

/* Meaningful bytes in this chunk, and the end-of-transfer marker, packed into
 * one byte so `flags` can go on mirroring grab/focus state like every other
 * packet -- an invariant worth more than the byte it costs. */
#define NVKVM_BROKER_CLIP_NBYTES(info)  ((info) & 0x1fu)
#define NVKVM_BROKER_CLIP_LAST          0x20u

/* Payload bytes per message, each direction.  Chosen so the containing struct
 * is exactly the size that direction already uses. */
#define NVKVM_BROKER_CLIP_PKT_BYTES  15u   /* broker -> VMM, inside 24 */
#define NVKVM_BROKER_CLIP_CMD_BYTES  27u   /* VMM -> broker, inside 40 */

/*
 * The hard cap on one clipboard transfer, both directions.  A paste is text a
 * person selected; 7 KiB is far more than an ordinary paste and, unlike the
 * old 16 KiB claim, it provably fits the broker's fixed 512-packet outbound
 * ring with its four-packet control reserve.  Enforced as a chunk count on the
 * receiving side of each direction.
 */
#define NVKVM_BROKER_CLIP_MAX_BYTES  7168u
#define NVKVM_BROKER_CLIP_MAX_CHUNKS_PKT \
    ((NVKVM_BROKER_CLIP_MAX_BYTES + NVKVM_BROKER_CLIP_PKT_BYTES - 1u) / \
     NVKVM_BROKER_CLIP_PKT_BYTES)
#define NVKVM_BROKER_CLIP_MAX_CHUNKS_CMD \
    ((NVKVM_BROKER_CLIP_MAX_BYTES + NVKVM_BROKER_CLIP_CMD_BYTES - 1u) / \
     NVKVM_BROKER_CLIP_CMD_BYTES)

/*
 * EV_CLOSE.x — WHICH close the user asked for.  The broker asks the human and
 * reports the answer; what each one MEANS is still entirely the VMM's, which
 * is what keeps VM policy out of the privileged process.
 */
#define NVKVM_BROKER_CLOSE_POWERDOWN 0  /* graceful: the guest OS decides     */
#define NVKVM_BROKER_CLOSE_FORCE     1  /* stop the machine now               */

/*
 * Capability bits reported in HELLO (w1).  These describe what the backend on
 * THIS session can actually do, so a client — and a human reading the broker's
 * startup log — learns the truth before a grab is attempted, not during.
 *
 * A partial grab is acceptable.  A partial grab announced as a total one is
 * not: the user would believe their keystrokes are going to the guest when
 * some of them are still reaching the host.
 */
#define NVKVM_BROKER_CAP_KEYBOARD     (1u << 0) /* keyboard events at all      */
#define NVKVM_BROKER_CAP_ABS_POINTER  (1u << 1) /* absolute motion available   */
#define NVKVM_BROKER_CAP_REL_POINTER  (1u << 2) /* true relative motion        */
#define NVKVM_BROKER_CAP_POINTER_LOCK (1u << 3) /* pointer confined under grab */
#define NVKVM_BROKER_CAP_TOTAL_GRAB   (1u << 4) /* compositor/WM shortcuts are
                                                 * inhibited too.  Clear means
                                                 * SOME shortcuts still reach
                                                 * the host — see the log line */
#define NVKVM_BROKER_CAP_FOCUS_EVENTS (1u << 5) /* focus loss is observable —
                                                 * REQUIRED for grab to be
                                                 * offered at all              */
#define NVKVM_BROKER_CAP_FULLSCREEN   (1u << 6) /* CTRL+ALT+F does something   */
#define NVKVM_BROKER_CAP_DMABUF       (1u << 7) /* the session can accept
                                                 * dma-buf buffers.  Clear ⇒
                                                 * ATTACH will always fail     */
#define NVKVM_BROKER_CAP_MODIFIERS    (1u << 8) /* explicit format modifiers
                                                 * are negotiated.  Clear ⇒
                                                 * only DRM_FORMAT_MOD_INVALID
                                                 * (implicit) is accepted      */
#define NVKVM_BROKER_CAP_RELEASE      (1u << 9) /* EV_RELEASE is real, not
                                                 * synthesised                 */
#define NVKVM_BROKER_CAP_CURSOR       (1u << 10)/* CMD_CURSOR is understood and
                                                 * this backend can show the
                                                 * guest's cursor as the host
                                                 * pointer.  Clear ⇒ sending
                                                 * CMD_CURSOR is a protocol
                                                 * violation (an older broker
                                                 * does not know the type), and
                                                 * the VMM must keep composing
                                                 * the cursor into the frame   */
#define NVKVM_BROKER_CAP_CURSOR_STREAM (1u << 12)/* this viewer draws the
                                                 * guest's cursor ITSELF in
                                                 * every input mode, position
                                                 * included: the host pointer
                                                 * in hover, an overlay at the
                                                 * last MOVE under grab.  Only
                                                 * with CAP_CURSOR.  Then CMD_
                                                 * CURSOR also takes MOVE and a
                                                 * SET may carry F_XOR, and the
                                                 * VMM composes NO cursor into
                                                 * the frame in any mode.
                                                 * Clear (an older broker, or a
                                                 * backend that cannot overlay)
                                                 * ⇒ MOVE and F_XOR are framing
                                                 * violations and the VMM keeps
                                                 * composing as before.  An
                                                 * older VMM ignores the bit   */
#define NVKVM_BROKER_CAP_DEVICE       (1u << 11)/* an EV_DEVICE follows the
                                                 * handshake's FRAME.  Set by
                                                 * every backend of a broker
                                                 * that knows the type, even
                                                 * when its answer is "unknown"
                                                 * -- so "too old to say" and
                                                 * "could not tell" differ     */
#define NVKVM_BROKER_CAP_TICK         (1u << 13)/* this backend reports the display's
                                                 * refresh (EV_TICK) and runs the
                                                 * lock-step frame requests.  The
                                                 * broker sends EV_TICK only after
                                                 * the VMM replied CMD_CAPS with
                                                 * NVKVM_BROKER_CLIENT_TICK; an
                                                 * older VMM ignores this bit and
                                                 * gets exactly today's exchange.
                                                 * Clear (an older broker, or the
                                                 * X11 backend) => the VMM keeps
                                                 * its own clock and EV_FRAME     */

/* BYE reason codes. */
enum {
    NVKVM_BROKER_BYE_SHUTDOWN     = 0,
    NVKVM_BROKER_BYE_DISPLAY_LOST = 1,  /* the compositor/X server went away  */
    NVKVM_BROKER_BYE_PROTOCOL     = 2,  /* the client violated the protocol   */
};

struct nvkvm_broker_pkt {
    uint16_t type;      /* NVKVM_BROKER_EV_*                                  */
    uint16_t flags;     /* NVKVM_BROKER_F_*                                   */
    uint32_t seq;       /* monotonic per connection; a gap means packets were
                         * dropped, which the broker never does silently for
                         * keys or buttons — it disconnects instead            */
    int32_t  x;
    int32_t  y;
    uint32_t w0;
    uint32_t w1;
};

/* Mirrored on every packet so a client can never disagree with the broker
 * about grab state, whatever it did with the GRAB event. */
#define NVKVM_BROKER_F_GRABBED  (1u << 0)
#define NVKVM_BROKER_F_FOCUSED  (1u << 1)
/*
 * The window is fullscreen.  Mirrored like the others, and load-bearing on
 * EV_SURFACE: it is what separates the two kinds of size change.
 *
 *   windowed resize  -> the host scales the buffer it already has.  The guest
 *                       is NOT told and MUST NOT re-mode: applications inside
 *                       would reflow, games would reinitialise swapchains, and
 *                       X clients would get a ConfigureNotify storm, all
 *                       because someone dragged a window edge.
 *   fullscreen       -> the size IS propagated, so the guest re-modes to the
 *                       output's resolution.  That is when you want it: 1:1
 *                       pixels, no scaling, and a surface that covers the
 *                       output, which is the compositor's condition for
 *                       promoting it to direct scanout.  It is also a
 *                       deliberate user action where a brief re-mode is
 *                       expected -- games do this routinely.
 */
#define NVKVM_BROKER_F_FULLSCREEN (1u << 2)

/*
 * EV_CLIPBOARD laid over the standard event packet.  Same 24 bytes, same
 * `type`/`flags`/`seq` in the same places -- so a receiver that does not know
 * this type still parses the header correctly and skips a whole, well-formed
 * message.  Ordering is the stream's; `seq` is the usual monotonic counter, so
 * a gap is already detectable.
 */
struct nvkvm_broker_clip_pkt {
    uint16_t type;      /* NVKVM_BROKER_EV_CLIPBOARD                        */
    uint16_t flags;     /* mirrored state, exactly as on every other packet */
    uint32_t seq;       /* monotonic per connection                         */
    uint8_t  info;      /* NVKVM_BROKER_CLIP_NBYTES / _LAST                 */
    uint8_t  data[NVKVM_BROKER_CLIP_PKT_BYTES];
};

/*
 * EV_TICK laid over the standard event packet: same 24 bytes, same
 * type/flags/seq in the same places, so a receiver that does not know the type
 * still skips a whole, well-formed message.
 *
 *   flags        the mirrored NVKVM_BROKER_F_* state, as on every packet
 *   seq          the connection's PACKET counter (NOT the tick number)
 *   tick_seq     the tick number: the first tick is 1, +1 per EMITTED tick,
 *                wrapping from 0xffffffff to 1 -- 0 is never a tick, so 0 can
 *                mean "none" in a reply.  A request is named by the tick_seq
 *                of the tick that opened it.
 *   refresh_word bits 0..27   the DECIMATED period in nanoseconds (the host
 *                             period x the divisor), 0 = unknown, else
 *                             NVKVM_BROKER_TICK_REFRESH_MIN_NS..MASK
 *                bit 28       NVKVM_BROKER_TICK_F_WANT_FRAME
 *                bit 29       NVKVM_BROKER_TICK_F_CLOCK_ONLY
 *                bits 30..31  reserved, must be zero (a violation otherwise)
 *   present_ns   CLOCK_MONOTONIC nanoseconds of the presentation, never
 *                decreasing on a connection
 *
 * A packed word rather than a separate flags field, as the clipboard packs its
 * byte count and its end marker into `info`: 24 bytes cannot hold a 64-bit
 * sequence, a 64-bit timestamp, the refresh and the flags.
 */
struct nvkvm_broker_tick_pkt {
    uint16_t type;          /* NVKVM_BROKER_EV_TICK                           */
    uint16_t flags;         /* mirrored state                                 */
    uint32_t seq;           /* packet counter                                 */
    uint32_t tick_seq;      /* the tick number (never 0)                      */
    uint32_t refresh_word;  /* period ns | NVKVM_BROKER_TICK_F_*              */
    uint64_t present_ns;    /* CLOCK_MONOTONIC ns                             */
};

#define NVKVM_BROKER_TICK_REFRESH_MASK    0x0fffffffu
#define NVKVM_BROKER_TICK_REFRESH_MIN_NS  1000000u
#define NVKVM_BROKER_TICK_F_WANT_FRAME    (1u << 28)
/* present_ns is when the broker LEARNED of the refresh (a frame callback, or
 * feedback on a clock other than CLOCK_MONOTONIC), not the compositor's own
 * presentation timestamp. */
#define NVKVM_BROKER_TICK_F_CLOCK_ONLY    (1u << 29)
#define NVKVM_BROKER_TICK_F_RESERVED      0xc0000000u
/* A request left unanswered this long is dead and the next one may open; an
 * answered frame whose presentation feedback has not come in this long counts
 * as presented.  (Windows resets its display driver after ~2 s without a flip
 * completion; this is far inside it.) */
#define NVKVM_BROKER_TICK_REQ_TIMEOUT_NS  100000000u
/* The most Hz the VMM's virtual monitor carries (kayfabe's EDID range, D3 --
 * kf_disp::pace::MAX_HZ) and the least (MIN_HZ).  The broker decimates the
 * host refresh so the tick rate is at most the VMM's cap. */
#define NVKVM_BROKER_GUEST_MAX_HZ         75u
#define NVKVM_BROKER_GUEST_MIN_HZ         24u

/* ── VMM → broker: commands ──────────────────────────────────────────────── */

#define NVKVM_BROKER_CMD_SIZE      40u

enum {
    /*
     * ATTACH — SCM_RIGHTS carries EXACTLY ONE fd, which must be a dma-buf.
     * The descriptor fields describe it.  The broker validates all of them
     * against the fd's real size before importing anything (README §3).
     * The fd is consumed: the broker imports it and closes its copy.
     */
    NVKVM_BROKER_CMD_ATTACH = 1,
    /*
     * COMMIT — present the most recently ATTACHed buffer.  Carries no fd and
     * no descriptor; every descriptor field must be zero.  Split from ATTACH
     * because a compositor distinguishes "the content changed" from "the
     * frame is finished", and committing a half-drawn buffer is visible.
     * `seq` is the request answered when the VMM opted in with
     * NVKVM_BROKER_CLIENT_TICK, and advisory otherwise.
     */
    NVKVM_BROKER_CMD_COMMIT = 2,
    /*
     * WINDOW — the guest changed resolution; ask for a window this size.
     * width/height only.  It is a REQUEST: the broker clamps it to its own
     * limits and the window manager may ignore it entirely.  The size that
     * actually took effect comes back as EV_SURFACE.
     */
    NVKVM_BROKER_CMD_WINDOW = 3,
    /*
     * CLIPBOARD — one fixed-size chunk of GUEST clipboard text.  Carries no
     * fd.  Accepted only in a mode that permits guest->host, only while the
     * window is focused, rate-limited, and capped by chunk count.
     *
     * The guest WRITING the host clipboard is the direction people
     * underestimate: you copy in the guest, paste into a host terminal, and
     * the guest planted something else.  That is why it is visible when it
     * happens rather than silent.
     */
    NVKVM_BROKER_CMD_CLIPBOARD = 4,
    /*
     * CAPS — what the VMM can do, sent once after connecting.  `width` carries
     * NVKVM_BROKER_CLIENT_* bits; every other field must be zero -- except
     * `height` with NVKVM_BROKER_CLIENT_TICK (see there).
     *
     * It exists so "clipboard is enabled but nothing happens" can be told
     * apart from "the guest has no clipboard agent".  Those have completely
     * different fixes, and conflating them is how someone ends up turning the
     * mode up to `full` believing the mode was the problem.
     */
    NVKVM_BROKER_CMD_CAPS = 5,

    /*
     * "Can you display a buffer of this (fourcc, modifier)?"  Answered with
     * NVKVM_BROKER_EV_FORMAT.  Uses the ordinary command record: `fourcc` and
     * `modifier` are the question; every other field is ignored.
     *
     * WHY THIS EXISTS.  The guest renders in its GPU's native tiling, and on a
     * CROSS-VENDOR host -- guest on NVIDIA, compositor scanning out on an
     * Intel/AMD iGPU -- the compositor cannot import that at any price.
     * MEASURED on a hybrid laptop: Mutter advertised 28 (format, modifier)
     * pairs, every one of them Intel, LINEAR or INVALID, while the guest
     * presented NVIDIA block-linear 0x0300000000606014.  Every ATTACH was
     * refused and the window stayed black.
     *
     * The VMM cannot work that out for itself: the advertised set lives in the
     * broker, and an ATTACH rejection is not reported back (it drops the frame
     * and logs).  So it must be able to ASK, once, at mode-set -- and on "no"
     * fall back to reading the frame back through the guest's own GPU into a
     * LINEAR buffer that any compositor can take.
     *
     * Asked once per mode change, never per frame.
     */
    NVKVM_BROKER_CMD_QUERY_FORMAT = 6,

    /*
     * CURSOR — the guest's pointer image, shown as the HOST pointer while the
     * pointer hovers over the guest's picture and the input is NOT grabbed.
     * Only valid when HELLO advertised NVKVM_BROKER_CAP_CURSOR.  Laid out as
     * struct nvkvm_broker_cursor_cmd, below; `op` says which of three it is:
     *
     *   SET   defines the image AND shows it.  SCM_RIGHTS carries exactly one
     *         fd, which must be shmem-backed (a memfd): the broker MEASURES
     *         it, COPIES the declared rows out with pread(2) -- never mmap,
     *         so a client that truncates it afterwards gets a refused cursor
     *         rather than a SIGBUS in the privileged process -- and closes it.
     *         The compositor is never handed the client's fd.
     *   HIDE  the guest has no visible cursor (or the VMM will compose it into
     *         the frame itself -- an XOR cursor, say).  No fd, every other
     *         field zero.  The host pointer over the picture is then NONE,
     *         which is exactly what it is before any CURSOR was ever sent.
     *   SHOW  show the last SET image again.  No fd, every other field zero.
     *         Before any SET on this connection it changes nothing visible.
     *
     * UNDER GRAB THE BROKER HIDES IT REGARDLESS.  The pointer is locked and
     * the guest's own pointer moves by relative motion, so a host cursor at
     * the lock position would point at the wrong place.  The VMM composes
     * the cursor into the frame while grabbed (it learns grab state from
     * EV_GRAB and from F_GRABBED on every packet); the broker keeps the image
     * and puts it back the moment the grab ends.
     *
     * SCALED WITH THE FRAME.  The image and hot spot are scaled by the same
     * factor the broker applies to the guest's frame in the window, so a
     * cursor over a 2x-scaled guest is twice the size, exactly as the guest
     * drew it relative to its own content.
     *
     * A SET whose fields do not describe its fd is REFUSED and the connection
     * lives, exactly like a rejected ATTACH: the geometry originates in the
     * guest, and a guest flipping nonsense must not kill the display of a VMM
     * that is behaving.  Framing errors -- unknown op, flags or reserved1 not
     * zero, an fd where none belongs or none where one does, a non-zero field
     * on HIDE/SHOW -- are violations, like everywhere else.
     */
    NVKVM_BROKER_CMD_CURSOR = 7,
    /*
     * NO_CHANGE -- the VMM's answer to a frame request when the picture did not
     * change.  `width` is the request number being answered (a tick_seq of a
     * WANT_FRAME tick, never 0); every other field must be zero; no fd.  Only
     * valid on a connection whose CMD_CAPS carried NVKVM_BROKER_CLIENT_TICK
     * (otherwise a framing violation, like an unknown type).  A request the
     * broker never issued is a violation; one it issued that is no longer open
     * (timed out and replaced, or answered already) is ignored and counted.
     */
    NVKVM_BROKER_CMD_NO_CHANGE = 8,
};

/* NVKVM_BROKER_CMD_CURSOR `op`. */
#define NVKVM_BROKER_CURSOR_SET  1u
#define NVKVM_BROKER_CURSOR_HIDE 2u
#define NVKVM_BROKER_CURSOR_SHOW 3u
/* Only with NVKVM_BROKER_CAP_CURSOR_STREAM: the image's top-left on the guest
 * frame, and the last absolute pointer position the VMM injected; see the MOVE
 * note by struct nvkvm_broker_cursor_cmd. */
#define NVKVM_BROKER_CURSOR_MOVE 4u

/*
 * Cursor bounds.  256x256 is the largest cursor plane any KMS driver this
 * project meets exposes (NVIDIA's, and amdgpu/i915 stop at 256 too), so it is
 * the largest image a guest can have.  The stride bound lets a small image sit
 * in a cursor buffer of the full 256-pixel width -- a 64x64 cursor in a
 * 256x256 plane buffer has a 1024-byte pitch -- and no wider: with both, the
 * most the broker ever copies for one SET is 256 rows of 1 KiB.
 */
#define NVKVM_BROKER_CURSOR_MAX_DIM    256u
#define NVKVM_BROKER_CURSOR_MAX_STRIDE (4u * NVKVM_BROKER_CURSOR_MAX_DIM)

/*
 * ATTACH flags.
 *
 * F_SHM says the fd is a memfd to be presented from shared memory rather than
 * imported as a dma-buf.  How that happens is the backend's business: Wayland
 * wraps it in a wl_shm_pool, X11 pushes it with core-protocol PutImage.  The
 * point of the rung is that NEITHER can refuse it.
 * DECLARED rather than sniffed from the fd type: a memfd is a legitimate
 * carrier for other things -- the broker's own test client sends one for every
 * frame -- so inferring intent from st.f_type misclassifies honest clients.
 * The broker still CHECKS that the fd really is a memfd when this is set; the
 * flag states intent, the check enforces it.
 *
 * The field was `reserved0`, always zero, so an older client reads as "not
 * shm", which is what it meant.
 */
#define NVKVM_BROKER_CMD_F_SHM (1u << 0)
#define NVKVM_BROKER_CMD_F_ALL NVKVM_BROKER_CMD_F_SHM

/* NVKVM_BROKER_CMD_CAPS `width` bits. */
#define NVKVM_BROKER_CLIENT_CLIPBOARD (1u << 0)
/* The VMM understands EV_TICK and will answer the frame requests it opens.
 * Only sent when HELLO carried NVKVM_BROKER_CAP_TICK.  With it, CMD_CAPS.height
 * is the most Hz the VMM's vblank may run at: 0 (= NVKVM_BROKER_GUEST_MAX_HZ),
 * or NVKVM_BROKER_GUEST_MIN_HZ..NVKVM_BROKER_GUEST_MAX_HZ; anything else, or a
 * non-zero height without this bit, is a framing violation.  With it,
 * COMMIT.seq names the request answered (0 = unsolicited: a replay frame, or a
 * VMM that had no credit); ATTACH.seq stays advisory. */
#define NVKVM_BROKER_CLIENT_TICK      (1u << 1)

/*
 * Explicitly laid out so every field is naturally aligned and the struct is
 * exactly 40 bytes with no compiler padding on any ABI either side may be
 * built for.  Checked by the assertion below, on both sides.
 */
struct nvkvm_broker_cmd {
    uint16_t type;      /* NVKVM_BROKER_CMD_*                       offset  0 */
    uint16_t flags;     /* NVKVM_BROKER_CMD_F_*; 0 for every older client   2 */
    uint32_t width;     /*                                                  4 */
    uint32_t height;    /*                                                  8 */
    uint32_t stride;    /* bytes per row of plane 0                        12 */
    uint32_t offset;    /* byte offset of plane 0 within the dma-buf       16 */
    uint32_t fourcc;    /* DRM_FORMAT_*                                    20 */
    uint64_t modifier;  /* DRM_FORMAT_MOD_*                                24 */
    uint32_t seq;       /* client-side counter; advisory, logged only      32 */
    uint32_t reserved1; /* must be 0                                       36 */
};

/* CMD_CLIPBOARD laid over the standard command record: same 40 bytes, same
 * `type` and `reserved0` in the same places. */
struct nvkvm_broker_clip_cmd {
    uint16_t type;      /* NVKVM_BROKER_CMD_CLIPBOARD */
    uint16_t flags;     /* must be 0 on this layout   */
    uint32_t chunk;     /* 0-based; bounded by the receiver's own count */
    uint32_t reserved1; /* must be 0                  */
    uint8_t  info;      /* NVKVM_BROKER_CLIP_NBYTES / _LAST */
    uint8_t  data[NVKVM_BROKER_CLIP_CMD_BYTES];
};

/*
 * CMD_CURSOR laid over the standard command record: same 40 bytes, and
 * type/flags/width/height/stride/offset/fourcc at the same offsets as ATTACH,
 * so the two describe a buffer the same way.  The 8 bytes ATTACH spends on a
 * modifier carry the hot spot instead (a cursor is always linear); `op` sits
 * where ATTACH keeps its advisory seq.
 *
 * PIXELS: DRM_FORMAT_ARGB8888 only -- 32-bit little-endian words, A in bits
 * 31..24, R 23..16, G 15..8, B 7..0, so B,G,R,A in memory -- with PREMULTIPLIED
 * alpha, which is what DRM's default "Pre-multiplied" blend mode means and what
 * a KMS cursor plane buffer already holds.  The broker copies the image anyway
 * (to scale it) and clamps every colour channel to its alpha on the way, so a
 * non-premultiplied image costs the sender wrong colours, never a malformed
 * buffer in the compositor.
 *
 * SET (fd required):
 *   width, height  1..NVKVM_BROKER_CURSOR_MAX_DIM
 *   stride         width*4 .. NVKVM_BROKER_CURSOR_MAX_STRIDE, in bytes
 *   offset         byte offset of row 0 within the memfd
 *   fourcc         DRM_FORMAT_ARGB8888 ('AR24'), nothing else
 *   hot_x, hot_y   < width, < height, in image pixels
 *   and offset + stride*(height-1) + width*4 <= the memfd's real size,
 *   computed in 64 bits.
 * HIDE / SHOW: no fd; width..hot_y all zero.
 * Every op: reserved1 == 0, and flags == 0 -- except a SET on a connection whose
 * HELLO advertised NVKVM_BROKER_CAP_CURSOR_STREAM, which may carry
 * NVKVM_BROKER_CURSOR_F_XOR (any other bit, or the flag on any other op or
 * without the capability, is a framing violation).
 *
 * XOR SET (flags == NVKVM_BROKER_CURSOR_F_XOR): the same ARGB8888 memfd image,
 * the same geometry rules and bounds, but the alpha byte is a MASK: 0xff where
 * the guest's XOR cursor INVERTS the screen, 0 elsewhere (the colour channels
 * carry the XOR colour).  A viewer cannot invert, so the BROKER builds the
 * visible cursor from it itself, always (nb_cursor_xor_approx(): opaque black
 * fill, a one-pixel opaque white outline around it in the 8-neighbourhood,
 * transparent elsewhere); the substitute is then scaled and shown like any
 * other image.  An all-clear mask draws nothing.
 *
 * MOVE (only with CAP_CURSOR_STREAM; otherwise an unknown op, a violation): no
 * fd, stride/offset/fourcc zero, and ONE atomic pair of facts:
 *   (a) the image's TOP-LEFT in the guest's frame pixels, signed 32-bit in the
 *       two hot-spot words: hot_x = x, hot_y = y (the cursor hangs off the frame
 *       when negative or past its edge);
 *   (b) the LAST ABSOLUTE pointer position the VMM injected into the guest, in
 *       the same frame pixels, signed 32-bit in width (x) and height (y), valid
 *       only when flags == NVKVM_BROKER_CURSOR_F_ABS.  The flag is CLEAR (and
 *       width/height zero) when the last input was relative, or none was
 *       injected yet: grab, where the hot spot is irrelevant.
 * |value| above NVKVM_BROKER_CURSOR_MOVE_LIMIT makes the MOVE refused (the
 * previous state stays, the connection lives); otherwise each value is clamped
 * to -MAX_DIM .. NVKVM_BROKER_MAX_DIM, outside of which an image is invisible
 * anyway.  Any other flag, or width/height without F_ABS, is a framing
 * violation.  The VMM sends a MOVE at its display pass rate, when the pair
 * changed, never per input event.
 *
 * What the broker does with it -- the VMM is mode-free (grab is the viewer's
 * own state, not the protocol's): in HOVER (absolute pointer) it keeps showing
 * the image at the HOST pointer through the native pointer API and ignores (a)
 * for position, but derives the hot spot from the pair: the Windows driver
 * programs hot 0,0 while the real spot is inside the image, and hot = (b) - (a).
 * The pair is only meaningful while the pointer is stationary (the guest moves
 * its cursor a little after it receives an injected event), so the broker
 * LATCHES a derived hot spot only when the SAME difference appears in two
 * consecutive MOVEs (event-counted, no timer), ignores a difference outside
 * the image, corrects a latched spot only by more than a pixel, and until it
 * latches -- and after every new image -- keeps the SET hot spot.  Under GRAB
 * (relative, locked) it has no pointer of its own and draws the image at (a),
 * scaled with the frame; (b) is then invalid and the hot spot irrelevant.
 * HIDE means: nothing is drawn -- the guest hid its cursor, or its cursor plane
 * is not enabled and it draws a software cursor into the frame itself.
 */
#define NVKVM_BROKER_CURSOR_MOVE_LIMIT (1 << 20)
/* MOVE flag: width/height carry the absolute pointer position. */
#define NVKVM_BROKER_CURSOR_F_ABS (1u << 0)
#define NVKVM_BROKER_CURSOR_F_XOR (1u << 0)

struct nvkvm_broker_cursor_cmd {
    uint16_t type;      /* NVKVM_BROKER_CMD_CURSOR                  offset  0 */
    uint16_t flags;     /* 0, or NVKVM_BROKER_CURSOR_F_XOR on a SET         2 */
    uint32_t width;     /*                                                  4 */
    uint32_t height;    /*                                                  8 */
    uint32_t stride;    /* bytes per row                                   12 */
    uint32_t offset;    /* byte offset of row 0 within the memfd           16 */
    uint32_t fourcc;    /* DRM_FORMAT_ARGB8888                             20 */
    uint32_t hot_x;     /*                                                 24 */
    uint32_t hot_y;     /*                                                 28 */
    uint32_t op;        /* NVKVM_BROKER_CURSOR_*                           32 */
    uint32_t reserved1; /* must be 0                                       36 */
};

/*
 * Single-plane only, on purpose.  The nvkvm guest head advertises XRGB8888 and
 * ARGB8888 (src/guest/nvkvm_kms.c nvkvm_pipe_formats[]) — both single-plane —
 * so multi-plane support would be untested code on the privileged side of the
 * boundary.  A multi-plane format is rejected as an unadvertised fourcc.
 */

/*
 * Largest window/buffer edge the broker will accept, in pixels.
 *
 * This is the same 8192 as NVKVM_PRESENT_MAX_DIM in
 * src/qemu/nvkvm_isolate_handlers.c, one process further out.  That bound
 * exists because finding S-3 caught guest-controlled geometry landing in the
 * VMM unexamined; the broker must not assume the VMM still enforces it,
 * because in this threat model the VMM is the attacker.
 */
#define NVKVM_BROKER_MAX_DIM   8192u

/* Both sides must agree byte for byte; a mismatch is a silent desync. */
#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(struct nvkvm_broker_pkt) == NVKVM_BROKER_PKT_SIZE,
               "nvkvm_broker_pkt must be exactly 24 bytes");
_Static_assert(sizeof(struct nvkvm_broker_tick_pkt) == NVKVM_BROKER_PKT_SIZE,
               "tick_pkt must be exactly the event packet size");
_Static_assert(offsetof(struct nvkvm_broker_tick_pkt, tick_seq) == 8 &&
               offsetof(struct nvkvm_broker_tick_pkt, refresh_word) == 12 &&
               offsetof(struct nvkvm_broker_tick_pkt, present_ns) == 16,
               "tick_pkt field offsets are part of the wire");
_Static_assert((NVKVM_BROKER_TICK_REFRESH_MASK &
                (NVKVM_BROKER_TICK_F_WANT_FRAME |
                 NVKVM_BROKER_TICK_F_CLOCK_ONLY |
                 NVKVM_BROKER_TICK_F_RESERVED)) == 0,
               "the refresh bits and the flag bits must not overlap");
_Static_assert(sizeof(struct nvkvm_broker_cmd) == NVKVM_BROKER_CMD_SIZE,
               "nvkvm_broker_cmd must be exactly 40 bytes");
/* The clipboard overlays MUST be the same size as what they overlay, or the
 * "every message is fixed size" property is gone and the reader desynchronises
 * on the first clipboard chunk. */
_Static_assert(sizeof(struct nvkvm_broker_clip_pkt) == NVKVM_BROKER_PKT_SIZE,
               "clip_pkt must be exactly the event packet size");
_Static_assert(sizeof(struct nvkvm_broker_clip_cmd) == NVKVM_BROKER_CMD_SIZE,
               "clip_cmd must be exactly the command size");
_Static_assert(sizeof(struct nvkvm_broker_cursor_cmd) == NVKVM_BROKER_CMD_SIZE,
               "cursor_cmd must be exactly the command size");
/* The cursor overlay deliberately shares ATTACH's buffer description.  If a
 * field ever moves, a reader that knows one layout misreads the other. */
_Static_assert(offsetof(struct nvkvm_broker_cursor_cmd, fourcc) ==
               offsetof(struct nvkvm_broker_cmd, fourcc) &&
               offsetof(struct nvkvm_broker_cursor_cmd, stride) ==
               offsetof(struct nvkvm_broker_cmd, stride) &&
               offsetof(struct nvkvm_broker_cursor_cmd, reserved1) ==
               offsetof(struct nvkvm_broker_cmd, reserved1),
               "cursor_cmd must overlay the command record field for field");
/* The largest SET copies MAX_STRIDE * MAX_DIM bytes; that must fit an int. */
_Static_assert((unsigned long long)NVKVM_BROKER_CURSOR_MAX_STRIDE *
               NVKVM_BROKER_CURSOR_MAX_DIM <= 0x7fffffffull,
               "cursor copy span must fit an int");
_Static_assert(NVKVM_BROKER_CLIP_PKT_BYTES <= 0x1fu &&
               NVKVM_BROKER_CLIP_CMD_BYTES <= 0x1fu,
               "chunk payload must fit the 5-bit nbytes field");
#endif

#endif /* NVKVM_BROKER_PROTO_H */
