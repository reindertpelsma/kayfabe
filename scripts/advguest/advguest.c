// SPDX-License-Identifier: GPL-2.0
/*
 * advguest — kayfabe's first deliberately MALICIOUS client.
 * =========================================================
 *
 * A mini "hostile ogkm": a Linux kernel module that pretends to be a hostile
 * NVIDIA kernel driver. It maps the emulated GPU's guest-visible MMIO surface
 * DIRECTLY (BAR0 registers, the doorbell page, the GSP command queue, the
 * MMU-invalidate registers, the BAR1/BAR2 windows) — never /dev/nvidia* — and
 * fires a suite of adversarial submissions at it: out-of-range doorbell tokens,
 * rings/USERD pointing outside guest RAM, CE pushbuffers whose PHYSICAL operands
 * name addresses outside the framebuffer, MMU invalidates with no valid PDB, and
 * several kthreads hammering all of the above at once.
 *
 * ── THE PROPERTY UNDER TEST ────────────────────────────────────────────────────
 * kayfabe must REFUSE BY NAME and STAY ALIVE. The suite PASSES when every
 * adversarial input is refused or contained AND the VMM (the kayfabe archive
 * linked into qemu-system-x86_64) is still serving MMIO at the end. A VMM crash,
 * a hang, or a SILENT ACCEPT of a dangerous operation is a FAIL.
 *
 * ⊘ What this module can and cannot SEE. It runs inside the guest, so its only
 * in-guest observable is "did the device still respond sanely after the op?"
 * (ADV_ALIVE: TIME_0 advances and is not all-ones). kayfabe's refusal-BY-NAME
 * (e.g. `FaultTag("FwdFault::PushbufferAperture")`, the `DOORBELL ... REFUSED`
 * line) is printed on the HOST side, into the qemu log, NOT into this guest's
 * serial. So each adversarial case prints the name of the refusal it is DESIGNED
 * to provoke as a trailing `want=<name>` field, and run_adv_guest.sh cross-checks
 * the qemu log for it. The guest-side verdict is liveness+containment; the
 * host-side grep is refusal-by-name. Both are needed; neither alone is the whole
 * property. (`the_last_line_is_not_the_verdict`, `a_check_that_reports_is_not_a
 * _check_that_gates`.)
 *
 * ── VERDICT VOCABULARY ─────────────────────────────────────────────────────────
 *   PASS      a positive control that must succeed (device present, BAR mappable,
 *             liveness baseline). If any of these FAIL, later phases are UNMEASURED.
 *   REFUSED   the adversarial input was refused/contained and the VMM is still
 *             alive. This is the GOOD outcome for an attack (expected).
 *   FAIL      the device stopped responding after the op (VMM crash), or a case
 *             that must be contained was silently accepted in a detectable way.
 *   NOTRUN    phase 0 bring-up did not reach a known-good state, so this case was
 *             never armed. NOTRUN is not PASS and not FAIL — it is unmeasured.
 *             (`a_census_zero_needs_a_known_positive`.)
 *
 * ── PHASES (owner addendum #2) ────────────────────────────────────────────────
 *   phase 0  bring-up: discover the device, map its BARs, assert a known-good
 *            state. Reported as its OWN verdict. If it fails, phases 1-3 are
 *            NOTRUN, never PASS/FAIL.
 *   phase 1  well-formed but hostile submissions (the valuable ones): structurally
 *            valid ogkm structures naming addresses they must not reach.
 *   phase 2  malformed / fuzzed inputs and raw register abuse.
 *   phase 3  concurrency: kthreads hammering doorbell + invalidate + queue heads.
 *
 * post_init: the harness sets post_init=1 when it loaded the real nvidia stack
 * FIRST (ogkm's own boot/devinit/GSP bring-up sequence executed — owner addendum
 * #2). From that post-WPR2, channels-scheduled, VAS-live state, a hostile
 * submission can reach the deep pushbuffer decoder and provoke a named FwdFault.
 * Cold (post_init=0), the same submission is a non-event (dbtable::Route =
 * Unallocated) because no channel is installed on the token; the module reports
 * the reach honestly in `want=` and does not claim a decoder refusal it could not
 * have caused.
 *
 * ⚠ This module deliberately does NOT request_mem_region() the BARs before
 * ioremap: a real hostile kernel driver bypasses ownership, and in post_init mode
 * the real nvidia driver already owns them. That bypass is the point.
 */
#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/pci.h>
#include <linux/io.h>
#include <linux/delay.h>
#include <linux/slab.h>
#include <linux/kthread.h>
#include <linux/atomic.h>
#include <linux/completion.h>
#include <linux/mm.h>
#include <linux/stdarg.h>

#include "advguest.h"

MODULE_LICENSE("GPL");
MODULE_AUTHOR("kayfabe adversarial-guest");
MODULE_DESCRIPTION("Hostile kernel client for the kayfabe emulated GPU (refuse-by-name + stay-alive suite)");

static int post_init;
module_param(post_init, int, 0444);
MODULE_PARM_DESC(post_init, "1 = the real nvidia stack was loaded first (attack a post-init GPU)");

static int storm = 4096;
module_param(storm, int, 0444);
MODULE_PARM_DESC(storm, "rings per storm/flood case");

static int threads = 4;
module_param(threads, int, 0444);
MODULE_PARM_DESC(threads, "concurrent kthreads for phase 3");

/* ── device handle ─────────────────────────────────────────────────────────── */
static struct pci_dev *adv_pdev;
static void __iomem *adv_bar0;
static void __iomem *adv_bar1;
static void __iomem *adv_bar2;
static u64 adv_bar1_len;
static u64 adv_bar2_len;

/* ── tally, owned by the reporting code ──────────────────────────────────────
 * `the_last_line_is_not_the_verdict`: the tally is written by the same function
 * that increments the counters, and a BEGIN marker is printed up front so the
 * harness can tell "suite crashed mid-run" (BEGIN present, TOTAL absent) from
 * "suite never started" (neither present). Absence is made detectable. */
static int n_pass, n_refused, n_fail, n_notrun;

enum adv_verdict { V_PASS, V_REFUSED, V_FAIL, V_NOTRUN };

static const char *verdict_str(enum adv_verdict v)
{
	switch (v) {
	case V_PASS:    return "PASS";
	case V_REFUSED: return "REFUSED (expected)";
	case V_FAIL:    return "FAIL";
	default:        return "NOTRUN";
	}
}

static void adv_count(enum adv_verdict v)
{
	switch (v) {
	case V_PASS:    n_pass++;    break;
	case V_REFUSED: n_refused++; break;
	case V_FAIL:    n_fail++;    break;
	default:        n_notrun++;  break;
	}
}

/* One self-describing line per case, and the counter moves HERE, so a line that
 * printed is a line that counted. `want` names the host-side refusal to look for
 * (may be NULL for positive controls / true non-events). */
__printf(4, 5)
static void adv_report(const char *id, const char *name, enum adv_verdict v,
		       const char *want_fmt, ...)
{
	char want[96];

	if (want_fmt) {
		va_list ap;

		va_start(ap, want_fmt);
		vsnprintf(want, sizeof(want), want_fmt, ap);
		va_end(ap);
		pr_info("ADVGUEST %s %s = %s want=%s\n", id, name, verdict_str(v), want);
	} else {
		pr_info("ADVGUEST %s %s = %s\n", id, name, verdict_str(v));
	}
	adv_count(v);
}

/* ── liveness ────────────────────────────────────────────────────────────────
 * A dead MMIO region reads 0xFFFFFFFF. A live emulated GPU reads a nanosecond
 * clock at TIME_0/TIME_1 that advances. "alive" = TIME_0 is not all-ones and the
 * pair is not frozen across a short wait. ⊘ We cannot detect a VMM that HUNG
 * inside a trap handler from here — that wedges this vCPU on the MMIO exit and no
 * further guest code runs; run_adv_guest.sh's whole-run budget catches that as a
 * crash ("a timeout IS a crash"). This is the crash detector for a VMM that
 * FAULTED and stopped decoding, not for one that spins. */
static bool adv_alive(void)
{
	u32 a0, a1, b0;

	if (!adv_bar0)
		return false;
	a0 = ioread32(adv_bar0 + ADV_VF_TIME_0);
	a1 = ioread32(adv_bar0 + ADV_VF_TIME_1);
	if (a0 == 0xFFFFFFFFu && a1 == 0xFFFFFFFFu)
		return false;           /* device stopped decoding */
	udelay(50);
	b0 = ioread32(adv_bar0 + ADV_VF_TIME_0);
	/* advancing OR at least reading a plausible clock — a frozen-but-readable
	 * value is still "the trap returned", which is the containment we test. */
	return b0 != 0xFFFFFFFFu;
}

/* helper: verdict from liveness — an attack is REFUSED (contained) iff alive. */
static enum adv_verdict alive_verdict(void)
{
	return adv_alive() ? V_REFUSED : V_FAIL;
}

/* ── fabricated guest-RAM structures ───────────────────────────────────────────
 * A hostile driver lays out its own USERD + GPFIFO ring + pushbuffer in guest
 * RAM. In a KVM guest, virt_to_phys() of a kernel page is the guest-physical
 * address the emulated device would read — exactly what a GPFIFO GET or a
 * PHYSICAL CE operand references. */
struct adv_chan {
	void *userd;   u64 userd_gpa;
	void *ring;    u64 ring_gpa;    /* GPFIFO entries */
	void *pb;      u64 pb_gpa;      /* pushbuffer (method stream) */
};

static bool adv_chan_alloc(struct adv_chan *c)
{
	memset(c, 0, sizeof(*c));
	c->userd = (void *)get_zeroed_page(GFP_KERNEL);
	c->ring  = (void *)get_zeroed_page(GFP_KERNEL);
	c->pb    = (void *)get_zeroed_page(GFP_KERNEL);
	if (!c->userd || !c->ring || !c->pb)
		goto fail;
	c->userd_gpa = virt_to_phys(c->userd);
	c->ring_gpa  = virt_to_phys(c->ring);
	c->pb_gpa    = virt_to_phys(c->pb);
	return true;
fail:
	free_page((unsigned long)c->userd);
	free_page((unsigned long)c->ring);
	free_page((unsigned long)c->pb);
	return false;
}

static void adv_chan_free(struct adv_chan *c)
{
	free_page((unsigned long)c->userd);
	free_page((unsigned long)c->ring);
	free_page((unsigned long)c->pb);
	memset(c, 0, sizeof(*c));
}

/* Write a dword into a fabricated pushbuffer at method position `*pos` (dwords). */
static void adv_pb_put(u32 *pb, unsigned int *pos, u32 v)
{
	if (*pos < (PAGE_SIZE / 4))
		pb[(*pos)++] = cpu_to_le32(v);
}

/* Build a single-entry GPFIFO pointing at the pushbuffer, and set GP_PUT=1 in
 * USERD so a doorbell would tell kayfabe "one entry to fetch". */
static void adv_arm_ring(struct adv_chan *c, unsigned int pb_len_dwords)
{
	u32 *ring = c->ring;
	u32 *userd = c->userd;

	ring[0] = cpu_to_le32(adv_gp_entry0(c->pb_gpa));
	ring[1] = cpu_to_le32(adv_gp_entry1(c->pb_gpa, pb_len_dwords));
	/* GP_GET=0, GP_PUT=1 → one unconsumed entry (ogkm-580: ga100/dev_ram.h:37-38) */
	userd[ADV_USERD_GP_GET_OFF / 4] = cpu_to_le32(0);
	userd[ADV_USERD_GP_PUT_OFF / 4] = cpu_to_le32(1);
}

/* Ring the doorbell for `token`. This is the whole guest→device submit gesture:
 * "my GP_PUT moved, go look" (THE_CONSTRAINTS §43). */
static void adv_ring_doorbell(u32 token)
{
	iowrite32(token, adv_bar0 + ADV_DOORBELL);
}

/* ═══════════════════════════════════════════════════════════════════════════════
 *  PHASE 0 — bring-up / discovery
 * ═══════════════════════════════════════════════════════════════════════════════ */
static bool adv_phase0(void)
{
	u32 t0, t1;

	adv_pdev = pci_get_device(ADV_PCI_VENDOR, ADV_PCI_DEVICE, NULL);
	if (!adv_pdev) {
		adv_report("P00", "device_present", V_FAIL,
			   "pci %04x:%04x", ADV_PCI_VENDOR, ADV_PCI_DEVICE);
		return false;
	}
	adv_report("P00", "device_present", V_PASS, NULL);

	/* ⊘ ioremap WITHOUT request_mem_region: hostile-driver behaviour, and in
	 * post_init mode nvidia.ko already holds the region. */
	adv_bar0 = ioremap(pci_resource_start(adv_pdev, ADV_BAR0_INDEX), ADV_BAR0_SIZE);
	if (!adv_bar0) {
		adv_report("P01", "bar0_mappable", V_FAIL, "ioremap bar0");
		return false;
	}
	adv_report("P01", "bar0_mappable", V_PASS, NULL);

	adv_bar1_len = pci_resource_len(adv_pdev, ADV_BAR1_INDEX);
	adv_bar2_len = pci_resource_len(adv_pdev, ADV_BAR2_INDEX);
	/* Map only a bounded probe window of each translated aperture. */
	adv_bar1 = ioremap(pci_resource_start(adv_pdev, ADV_BAR1_INDEX),
			   min_t(u64, adv_bar1_len, PAGE_SIZE));
	adv_bar2 = ioremap(pci_resource_start(adv_pdev, ADV_BAR2_INDEX),
			   min_t(u64, adv_bar2_len, PAGE_SIZE));
	adv_report("P02", "bars_mappable",
		   (adv_bar1 && adv_bar2) ? V_PASS : V_FAIL,
		   "bar1=%lluMiB bar2=%lluMiB",
		   adv_bar1_len >> 20, adv_bar2_len >> 20);

	/* Liveness baseline: TIME must read a plausible, advancing clock. */
	t0 = ioread32(adv_bar0 + ADV_VF_TIME_0);
	udelay(100);
	t1 = ioread32(adv_bar0 + ADV_VF_TIME_0);
	if (t0 == 0xFFFFFFFFu || !adv_alive()) {
		adv_report("P03", "identity_readback", V_FAIL,
			   "TIME_0=0x%08x", t0);
		return false;
	}
	adv_report("P03", "identity_readback", V_PASS, NULL);

	/* post-init state is its own verdict (owner addendum #2). We report the
	 * REACH we will attack from; a mislabelled reach is the exact defect
	 * `a_census_zero_needs_a_known_positive` warns about. */
	adv_report("P04", "post_init_state", V_PASS,
		   "reach=%s", post_init ? "POST-INIT (nvidia stack up)" : "COLD");
	return true;
}

/* ═══════════════════════════════════════════════════════════════════════════════
 *  PHASE 1 — well-formed but hostile submissions
 *  Each fabricates a structurally valid ogkm structure naming an address it must
 *  not reach, then rings. `want=` is the deep-decoder refusal if post_init made
 *  the decoder reachable, else the cold-path non-event (Unallocated).
 * ═══════════════════════════════════════════════════════════════════════════════ */

/* A guest-physical address well past any 8 GiB framebuffer — a host address if it
 * ever reached a PHYSICAL copy engine unmediated (THE_CONSTRAINTS §3 of
 * the_three_channel_kinds.md: "a guest-authored physical address arriving at the
 * engine unmediated is a HOST physical address"). */
#define ADV_WILD_PHYS   0x0000DEAD00000000ull
/* The hostile-token used when no real channel is known. Low 12 bits select the
 * dbtable slot; cold, this is Unallocated. */
#define ADV_HOSTILE_TOK 0x0000075Au

static const char *want_reach(const char *deep)
{
	/* post_init → the decoder is reachable and should refuse by `deep` name;
	 * cold → dbtable::Route::Unallocated non-event (dbtable.rs:87-95). */
	return post_init ? deep : "Route::Unallocated";
}

/* A10 — CE LAUNCH_DMA, SRC_TYPE=PHYSICAL, source outside the framebuffer. */
static void adv_a10_ce_phys_outside_fb(void)
{
	struct adv_chan c;
	u32 *pb;
	unsigned int p = 0;

	if (!adv_chan_alloc(&c)) {
		adv_report("A10", "ce_launch_phys_outside_fb", V_NOTRUN, "no mem");
		return;
	}
	pb = c.pb;
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_SET_SRC_PHYS_MODE, 1));
	adv_pb_put(pb, &p, ADV_PHYS_TARGET_LOCAL_FB);
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_OFFSET_IN_UPPER, 2));
	adv_pb_put(pb, &p, (u32)(ADV_WILD_PHYS >> 32));   /* OFFSET_IN_UPPER */
	adv_pb_put(pb, &p, (u32)ADV_WILD_PHYS);           /* OFFSET_IN_LOWER */
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_LINE_LENGTH_IN, 1));
	adv_pb_put(pb, &p, 0x1000);
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_LAUNCH_DMA, 1));
	adv_pb_put(pb, &p, ADV_LDMA_XFER_PIPELINED | ADV_LDMA_SRC_TYPE_PHYSICAL |
			   ADV_LDMA_DST_TYPE_PHYSICAL);
	adv_arm_ring(&c, p);
	adv_ring_doorbell(ADV_HOSTILE_TOK);
	adv_report("A10", "ce_launch_phys_outside_fb", alive_verdict(),
		   "%s", want_reach("FwdFault::PushbufferAperture|CpuCeFb"));
	adv_chan_free(&c);
}

/* A11 — CE operand straddling the end of the store (base near end, huge length). */
static void adv_a11_ce_straddle_store_end(void)
{
	struct adv_chan c;
	u32 *pb;
	unsigned int p = 0;
	u64 near_end = (adv_bar1_len ? adv_bar1_len : (128ull << 20)) - 8;

	if (!adv_chan_alloc(&c)) {
		adv_report("A11", "ce_operand_straddles_store_end", V_NOTRUN, "no mem");
		return;
	}
	pb = c.pb;
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_OFFSET_OUT_UPPER, 2));
	adv_pb_put(pb, &p, (u32)(near_end >> 32));
	adv_pb_put(pb, &p, (u32)near_end);
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_LINE_LENGTH_IN, 1));
	adv_pb_put(pb, &p, 0x00FF0000);                    /* absurd length past the end */
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_LAUNCH_DMA, 1));
	adv_pb_put(pb, &p, ADV_LDMA_XFER_PIPELINED | ADV_LDMA_DST_TYPE_PHYSICAL);
	adv_arm_ring(&c, p);
	adv_ring_doorbell(ADV_HOSTILE_TOK);
	adv_report("A11", "ce_operand_straddles_store_end", alive_verdict(),
		   "%s", want_reach("FwdFault::CpuCeStraddle|FbLeafExtent"));
	adv_chan_free(&c);
}

/* A12 — CE operand in an aperture kayfabe does not model (reserved PHYS target). */
static void adv_a12_ce_aperture_unmodeled(void)
{
	struct adv_chan c;
	u32 *pb;
	unsigned int p = 0;

	if (!adv_chan_alloc(&c)) {
		adv_report("A12", "ce_aperture_unmodeled", V_NOTRUN, "no mem");
		return;
	}
	pb = c.pb;
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_SET_SRC_PHYS_MODE, 1));
	adv_pb_put(pb, &p, ADV_PHYS_TARGET_UNMODELED);     /* target 3: not enumerated */
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_LAUNCH_DMA, 1));
	adv_pb_put(pb, &p, ADV_LDMA_XFER_PIPELINED | ADV_LDMA_SRC_TYPE_PHYSICAL);
	adv_arm_ring(&c, p);
	adv_ring_doorbell(ADV_HOSTILE_TOK);
	adv_report("A12", "ce_aperture_unmodeled", alive_verdict(),
		   "%s", want_reach("FwdFault::CePeerOperand|PushbufferAperture"));
	adv_chan_free(&c);
}

/* A13 — CE semaphore release to an unmapped VA (a forged completion target). */
static void adv_a13_ce_sema_unmapped(void)
{
	struct adv_chan c;
	u32 *pb;
	unsigned int p = 0;

	if (!adv_chan_alloc(&c)) {
		adv_report("A13", "ce_sema_release_unmapped", V_NOTRUN, "no mem");
		return;
	}
	pb = c.pb;
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_SET_SEMAPHORE_A, 3));
	adv_pb_put(pb, &p, (u32)(ADV_WILD_PHYS >> 32));    /* SEMAPHORE_A_UPPER */
	adv_pb_put(pb, &p, (u32)ADV_WILD_PHYS);            /* SEMAPHORE_B_LOWER */
	adv_pb_put(pb, &p, 0xC0FFEE00);                     /* SEMAPHORE_PAYLOAD */
	adv_pb_put(pb, &p, adv_method_hdr_inc(ADV_SUBCH_COPY, ADV_B0B5_LAUNCH_DMA, 1));
	adv_pb_put(pb, &p, ADV_LDMA_SEM_ONE_WORD);
	adv_arm_ring(&c, p);
	adv_ring_doorbell(ADV_HOSTILE_TOK);
	adv_report("A13", "ce_sema_release_unmapped", alive_verdict(),
		   "%s", want_reach("FwdFault::PushbufferAperture|NonRamGpa"));
	adv_chan_free(&c);
}

/* A14 — GPFIFO entry with LENGTH=0: a ring that brought no work. */
static void adv_a14_gpfifo_len_zero(void)
{
	struct adv_chan c;
	u32 *ring, *userd;

	if (!adv_chan_alloc(&c)) {
		adv_report("A14", "gpfifo_entry_len_zero", V_NOTRUN, "no mem");
		return;
	}
	ring = c.ring; userd = c.userd;
	ring[0] = cpu_to_le32(adv_gp_entry0(c.pb_gpa));
	ring[1] = cpu_to_le32(adv_gp_entry1(c.pb_gpa, 0));   /* LENGTH=0 */
	userd[ADV_USERD_GP_PUT_OFF / 4] = cpu_to_le32(1);
	adv_ring_doorbell(ADV_HOSTILE_TOK);
	adv_report("A14", "gpfifo_entry_len_zero", alive_verdict(),
		   "%s", want_reach("FwdFault::RingBroughtNoEntry|SubmissionDecodedNoWork"));
	adv_chan_free(&c);
}

/* A15 — GPFIFO entry with absurd LENGTH (max 21-bit) and a bogus pushbuffer VA. */
static void adv_a15_gpfifo_len_absurd(void)
{
	struct adv_chan c;
	u32 *ring, *userd;

	if (!adv_chan_alloc(&c)) {
		adv_report("A15", "gpfifo_entry_len_absurd", V_NOTRUN, "no mem");
		return;
	}
	ring = c.ring; userd = c.userd;
	ring[0] = cpu_to_le32(adv_gp_entry0(ADV_WILD_PHYS));
	ring[1] = cpu_to_le32(adv_gp_entry1(ADV_WILD_PHYS, 0x1FFFFF)); /* LENGTH max */
	userd[ADV_USERD_GP_PUT_OFF / 4] = cpu_to_le32(1);
	adv_ring_doorbell(ADV_HOSTILE_TOK);
	adv_report("A15", "gpfifo_entry_len_absurd", alive_verdict(),
		   "%s", want_reach("FwdFault::PushTooFragmented|PushbufferAperture"));
	adv_chan_free(&c);
}

/* A16 — USERD GP_PUT beyond the ring; GET pointer into non-RAM GPA. */
static void adv_a16_userd_out_of_ram(void)
{
	struct adv_chan c;
	u32 *userd;

	if (!adv_chan_alloc(&c)) {
		adv_report("A16", "userd_gp_put_beyond_ring", V_NOTRUN, "no mem");
		return;
	}
	userd = c.userd;
	/* GP_PUT beyond any plausible ring size, GP_GET backwards past it. */
	userd[ADV_USERD_GP_PUT_OFF / 4] = cpu_to_le32(0x7FFFFFFF);
	userd[ADV_USERD_GP_GET_OFF / 4] = cpu_to_le32(0x0000FFFF);
	adv_ring_doorbell(ADV_HOSTILE_TOK);
	adv_report("A16", "userd_gp_put_beyond_ring", alive_verdict(),
		   "%s", want_reach("FwdFault::RingProducerCursorOutOfRange"));
	adv_chan_free(&c);
}

/* ═══════════════════════════════════════════════════════════════════════════════
 *  PHASE 2 — malformed / fuzzed inputs and raw register abuse
 * ═══════════════════════════════════════════════════════════════════════════════ */

/* A20 — doorbell token out of range. dbtable routes low 12 bits; a wild token is
 * Unallocated: "not found, not denied" — a NON-EVENT, no lock, no log
 * (dbtable.rs:87-95). Property: contained, VMM alive. */
static void adv_a20_doorbell_out_of_range(void)
{
	adv_ring_doorbell(0xFFFFFFFFu);
	adv_report("A20", "doorbell_token_out_of_range", alive_verdict(),
		   "Route::Unallocated (non-event)");
}

/* A21 — doorbell token for a channel never created (mid-range, plausible). */
static void adv_a21_doorbell_never_created(void)
{
	adv_ring_doorbell(0x00000ABCu);
	adv_report("A21", "doorbell_token_never_created", alive_verdict(),
		   "Route::Unallocated (non-event)");
}

/* A22 — doorbell token with bits above the 11:0 vector field set. The channel
 * selector is only 11:0 (dbtable.rs:349); the high bits must not be honoured as a
 * selector or reached as an address. */
static void adv_a22_doorbell_high_bits(void)
{
	adv_ring_doorbell(0xDEAD0001u);
	adv_report("A22", "doorbell_token_high_bits_set", alive_verdict(),
		   "%s", post_init ? "FwdFault::MalformedToken|Route::Unallocated"
				   : "Route::Unallocated");
}

/* A23 — GSP command-queue heads written with garbage, no msgq ring behind them.
 * Pokes the GSP RPC submit path (THE_CONSTRAINTS: "the worst trap is the GSP RPC
 * submit"). Property: contained, VMM alive. */
static void adv_a23_gsp_queue_garbage(void)
{
	u32 i;

	for (i = 0; i < ADV_GSP_QUEUE_HEAD_CNT; i++)
		iowrite32(0xBADC0DE0u + i, adv_bar0 + ADV_GSP_QUEUE_HEAD0 + i * 8);
	adv_report("A23", "gsp_queue_head_garbage", alive_verdict(),
		   "contained (no valid msgq)");
}

/* A24 — MMU invalidate with PDB 0 and ALL_VA. */
static void adv_a24_mmu_pdb_zero(void)
{
	iowrite32(0, adv_bar0 + ADV_MMU_INVALIDATE_PDB);
	iowrite32(0, adv_bar0 + ADV_MMU_INVALIDATE_UPPER);
	iowrite32(ADV_INV_TRIGGER | ADV_INV_ALL_VA, adv_bar0 + ADV_MMU_INVALIDATE);
	adv_report("A24", "mmu_invalidate_pdb_zero", alive_verdict(),
		   "%s", post_init ? "FwdFault::UnknownPdb|UndeclaredPdb"
				   : "contained (queued/ignored)");
}

/* A25 — MMU invalidate naming a PDB address that does not exist. */
static void adv_a25_mmu_bogus_pdb(void)
{
	iowrite32(ADV_INV_PDB_ADDR(ADV_WILD_PHYS) | ADV_INV_PDB_APERTURE_SYS,
		  adv_bar0 + ADV_MMU_INVALIDATE_PDB);
	iowrite32((u32)(ADV_WILD_PHYS >> 32), adv_bar0 + ADV_MMU_INVALIDATE_UPPER);
	iowrite32(ADV_INV_TRIGGER | ADV_INV_ALL_VA, adv_bar0 + ADV_MMU_INVALIDATE);
	adv_report("A25", "mmu_invalidate_bogus_pdb", alive_verdict(),
		   "%s", post_init ? "FwdFault::UnknownPdb"
				   : "contained (queued/ignored)");
}

/* A26 — ALL_PDB invalidate flood. */
static void adv_a26_mmu_all_pdb_flood(void)
{
	int i;

	for (i = 0; i < storm; i++)
		iowrite32(ADV_INV_TRIGGER | ADV_INV_ALL_PDB | ADV_INV_ALL_VA,
			  adv_bar0 + ADV_MMU_INVALIDATE);
	adv_report("A26", "mmu_invalidate_all_pdb_flood", alive_verdict(),
		   "contained x%d", storm);
}

/* A27 — MMU invalidate trigger with NO preceding PDB write in this sequence. */
static void adv_a27_mmu_no_pdb(void)
{
	iowrite32(ADV_INV_TRIGGER | ADV_INV_ALL_VA, adv_bar0 + ADV_MMU_INVALIDATE);
	adv_report("A27", "mmu_invalidate_no_pdb_write", alive_verdict(),
		   "contained (no PDB context)");
}

/* A28 — write to an unclaimed (dead) BAR0 offset. Most of the 16 MiB CUT aperture
 * holds no register (nvkvm.c NVKVM_KIND_CUT); the archive counts these as
 * bar0_dead_writes rather than crashing. */
static void adv_a28_bar0_dead_write(void)
{
	iowrite32(0xA5A5A5A5u, adv_bar0 + 0x00500000u);  /* dead: past PROM(0x400000), below PRAMIN(0x700000) */
	adv_report("A28", "bar0_dead_region_write", alive_verdict(),
		   "bar0_dead_writes (contained)");
}

/* A29 — read near the very end of the BAR0 aperture. */
static void adv_a29_bar0_oob_read(void)
{
	(void)ioread32(adv_bar0 + (ADV_BAR0_SIZE - 4));
	adv_report("A29", "bar0_high_offset_read", alive_verdict(),
		   "contained (returns without crash)");
}

/* A30 — write to a BAR1 offset that names a VA the guest never mapped. BAR1 is
 * GMMU-translated (nvkvm.c: TRAP, not RESERVATION); an unmapped VA must fault by
 * construction (the_three_channel_kinds.md §4(a): "the VAS is the bound"). */
static void adv_a30_bar1_unmapped(void)
{
	if (!adv_bar1) {
		adv_report("A30", "bar1_unmapped_va_write", V_NOTRUN, "bar1 unmapped");
		return;
	}
	iowrite32(0x0BADCAFEu, adv_bar1 + 0x40);
	adv_report("A30", "bar1_unmapped_va_write", alive_verdict(),
		   "contained (GMMU miss = fault)");
}

/* A31 — write to a BAR2 (instance) offset; also GMMU-translated. */
static void adv_a31_bar2_unmapped(void)
{
	if (!adv_bar2) {
		adv_report("A31", "bar2_unmapped_write", V_NOTRUN, "bar2 unmapped");
		return;
	}
	iowrite32(0x0BADF00Du, adv_bar2 + 0x40);
	adv_report("A31", "bar2_unmapped_write", alive_verdict(),
		   "contained (GMMU miss = fault)");
}

/* A32 — fuzz: pseudo-random dwords across the known BAR0 register offsets. */
static void adv_a32_fuzz_registers(void)
{
	static const u32 offs[] = {
		ADV_DOORBELL, ADV_MMU_INVALIDATE, ADV_MMU_INVALIDATE_PDB,
		ADV_MMU_INVALIDATE_UPPER, ADV_GSP_QUEUE_HEAD0,
		ADV_GSP_QUEUE_HEAD0 + 8, ADV_VF_TIME_0,
	};
	u32 seed = 0x2545F491u;   /* Weyl */
	int i;

	for (i = 0; i < storm; i++) {
		seed += 0x9E3779B9u;
		iowrite32(seed ^ (seed >> 15),
			  adv_bar0 + offs[i % ARRAY_SIZE(offs)]);
	}
	adv_report("A32", "fuzz_bar0_registers", alive_verdict(),
		   "contained x%d", storm);
}

/* ═══════════════════════════════════════════════════════════════════════════════
 *  PHASE 3 — concurrency
 * ═══════════════════════════════════════════════════════════════════════════════ */
struct adv_worker {
	int id;
	int mode;         /* 0 doorbell, 1 invalidate, 2 queue head */
	int iters;
	struct task_struct *task;
};
static struct completion adv_workers_done;
static atomic_t adv_workers_left;

static int adv_hammer(void *arg)
{
	struct adv_worker *w = arg;
	int i;

	for (i = 0; i < w->iters && !kthread_should_stop(); i++) {
		switch (w->mode) {
		case 0:
			adv_ring_doorbell(0x00000A00u + w->id);
			break;
		case 1:
			iowrite32(ADV_INV_TRIGGER | ADV_INV_ALL_VA, adv_bar0 + ADV_MMU_INVALIDATE);
			break;
		default:
			iowrite32(0xF00D0000u + i,
				  adv_bar0 + ADV_GSP_QUEUE_HEAD0 +
				  (w->id % ADV_GSP_QUEUE_HEAD_CNT) * 8);
			break;
		}
		if ((i & 0x3FF) == 0)
			cond_resched();
	}
	if (atomic_dec_and_test(&adv_workers_left))
		complete(&adv_workers_done);
	return 0;
}

/* Run `n` kthreads with the given per-thread mode-fn selector, join, verdict. */
static enum adv_verdict adv_run_workers(struct adv_worker *ws, int n)
{
	int i, started = 0;

	init_completion(&adv_workers_done);
	atomic_set(&adv_workers_left, n);
	for (i = 0; i < n; i++) {
		ws[i].task = kthread_run(adv_hammer, &ws[i], "advhammer%d", i);
		if (IS_ERR(ws[i].task)) {
			ws[i].task = NULL;
			atomic_dec(&adv_workers_left);
			continue;
		}
		started++;
	}
	if (!started)
		return V_NOTRUN;
	/* Bounded wait; a VMM hang here is caught by the outer budget. */
	if (!wait_for_completion_timeout(&adv_workers_done, msecs_to_jiffies(8000))) {
		for (i = 0; i < n; i++)
			if (ws[i].task)
				kthread_stop(ws[i].task);
		return V_FAIL;   /* workers did not finish → wedged */
	}
	return alive_verdict();
}

static void adv_a40_doorbell_storm(void)
{
	struct adv_worker *ws;
	enum adv_verdict v;
	int i, n = clamp(threads, 1, 32);

	ws = kcalloc(n, sizeof(*ws), GFP_KERNEL);
	if (!ws) {
		adv_report("A40", "doorbell_storm_concurrent", V_NOTRUN, "no mem");
		return;
	}
	for (i = 0; i < n; i++) {
		ws[i].id = i; ws[i].mode = 0; ws[i].iters = storm;
	}
	v = adv_run_workers(ws, n);
	adv_report("A40", "doorbell_storm_concurrent", v,
		   "%d threads x%d rings", n, storm);
	kfree(ws);
}

static void adv_a41_invalidate_doorbell_race(void)
{
	struct adv_worker *ws;
	enum adv_verdict v;
	int i, n = clamp(threads, 2, 32);

	ws = kcalloc(n, sizeof(*ws), GFP_KERNEL);
	if (!ws) {
		adv_report("A41", "invalidate_doorbell_race", V_NOTRUN, "no mem");
		return;
	}
	for (i = 0; i < n; i++) {
		ws[i].id = i; ws[i].mode = (i & 1); ws[i].iters = storm;
	}
	v = adv_run_workers(ws, n);
	adv_report("A41", "invalidate_doorbell_race", v,
		   "%d threads (doorbell+invalidate)", n);
	kfree(ws);
}

static void adv_a42_multicpu_queue_head(void)
{
	struct adv_worker *ws;
	enum adv_verdict v;
	int i, n = clamp(threads, 2, 32);

	ws = kcalloc(n, sizeof(*ws), GFP_KERNEL);
	if (!ws) {
		adv_report("A42", "multicpu_queue_head", V_NOTRUN, "no mem");
		return;
	}
	for (i = 0; i < n; i++) {
		ws[i].id = i; ws[i].mode = 2; ws[i].iters = storm;
	}
	v = adv_run_workers(ws, n);
	adv_report("A42", "multicpu_queue_head", v, "%d threads", n);
	kfree(ws);
}

/* ── driver ─────────────────────────────────────────────────────────────────── */
static void adv_run_suite(void)
{
	bool up;

	/* BEGIN marker: the harness uses (BEGIN present, TOTAL absent) to tell a
	 * suite that crashed mid-run from one that never started. */
	pr_info("ADVGUEST_BEGIN post_init=%d storm=%d threads=%d\n",
		post_init, storm, threads);

	up = adv_phase0();
	if (!up) {
		/* phase 0 failed → every later case is UNMEASURED, not passing. */
		static const char * const ids[] = {
			"A10","A11","A12","A13","A14","A15","A16",
			"A20","A21","A22","A23","A24","A25","A26","A27",
			"A28","A29","A30","A31","A32","A40","A41","A42",
		};
		int i;

		for (i = 0; i < ARRAY_SIZE(ids); i++)
			adv_report(ids[i], "(phase 0 did not reach known-good state)",
				   V_NOTRUN, "bring-up failed");
		goto tally;
	}

	/* phase 1 */
	adv_a10_ce_phys_outside_fb();
	adv_a11_ce_straddle_store_end();
	adv_a12_ce_aperture_unmodeled();
	adv_a13_ce_sema_unmapped();
	adv_a14_gpfifo_len_zero();
	adv_a15_gpfifo_len_absurd();
	adv_a16_userd_out_of_ram();
	/* phase 2 */
	adv_a20_doorbell_out_of_range();
	adv_a21_doorbell_never_created();
	adv_a22_doorbell_high_bits();
	adv_a23_gsp_queue_garbage();
	adv_a24_mmu_pdb_zero();
	adv_a25_mmu_bogus_pdb();
	adv_a26_mmu_all_pdb_flood();
	adv_a27_mmu_no_pdb();
	adv_a28_bar0_dead_write();
	adv_a29_bar0_oob_read();
	adv_a30_bar1_unmapped();
	adv_a31_bar2_unmapped();
	adv_a32_fuzz_registers();
	/* phase 3 */
	adv_a40_doorbell_storm();
	adv_a41_invalidate_doorbell_race();
	adv_a42_multicpu_queue_head();

tally:
	/* Written by the code that owns the counters — never re-derived from the
	 * printed lines. `fail=0` AND alive is the pass condition. */
	pr_info("ADVGUEST_TOTAL pass=%d refused=%d fail=%d notrun=%d alive=%d\n",
		n_pass, n_refused, n_fail, n_notrun, adv_alive() ? 1 : 0);
	if (n_fail == 0 && adv_alive())
		pr_info("ADVGUEST_VERDICT=PASS (every adversarial input contained; VMM still serving)\n");
	else
		pr_info("ADVGUEST_VERDICT=FAIL (fail=%d alive=%d)\n",
			n_fail, adv_alive() ? 1 : 0);
}

/* Drop every resource this module took. Called on the init error path (the
 * one-shot case) and from exit (the resident case), so it must be idempotent. */
static void adv_teardown(void)
{
	if (adv_bar0) { iounmap(adv_bar0); adv_bar0 = NULL; }
	if (adv_bar1) { iounmap(adv_bar1); adv_bar1 = NULL; }
	if (adv_bar2) { iounmap(adv_bar2); adv_bar2 = NULL; }
	if (adv_pdev) { pci_dev_put(adv_pdev); adv_pdev = NULL; }
}

static int __init advguest_init(void)
{
	adv_run_suite();
	adv_teardown();
	/* Return an error so `insmod` unwinds the module immediately — this is a
	 * one-shot suite, not a resident driver. All BAR mappings and the pci_dev
	 * reference are already dropped by adv_teardown() above, and the verdict is
	 * already in the log, so nothing leaks and the suite can be re-run with
	 * another insmod without an rmmod. `insmod` prints a harmless "No such
	 * device" AFTER the report; the /init wrapper expects it. */
	return -ENODEV;
}

static void __exit advguest_exit(void)
{
	/* Reached only if init ever returns 0 (it does not today). Idempotent. */
	adv_teardown();
}

module_init(advguest_init);
module_exit(advguest_exit);
