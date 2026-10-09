/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "observer.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef struct { unsigned char ram[65536];unsigned emitted,reads,mutate;KFGT_RECORD last;uint64_t gen; } FIXTURE;
static void w32(unsigned char *p,uint32_t v) {unsigned i;for(i=0;i<4;i++)p[i]=(unsigned char)(v>>(i*8));}
static void w64(unsigned char *p,uint64_t v) {w32(p,(uint32_t)v);w32(p+4,(uint32_t)(v>>32));}
static int read_ram(void *opaque,uint64_t pa,void *out,size_t n) {
    FIXTURE *f=opaque;f->reads++;
    /* Only [0x1000,0xc000) is ordinary guest RAM; rest models holes/MMIO. */
    if(pa<0x1000 || pa>=0xc000 || n>0xc000-pa) return 0;
    if(out)memcpy(out,f->ram+pa,n);
    if(f->mutate && out && pa==0x8000) f->ram[0x8038]^=1;
    return 1;
}
static void emit(void *opaque,uint64_t gen,const KFGT_RECORD *r,const unsigned char *payload) {
    FIXTURE *f=opaque;assert(r->payload_bytes==80);assert(kf_u32(payload+52)==KFGT_RPC_SIGNATURE);
    f->emitted++;f->last=*r;f->gen=gen;
}
static void header(unsigned char *p) {
    w32(p,0);w32(p+4,16384);w32(p+8,4096);w32(p+12,3);w32(p+24,32);w32(p+28,4096);
}
static void message(unsigned char *p,uint32_t seq,uint32_t fn,uint32_t result) {
    unsigned i;uint32_t sum=0;memset(p,0,4096);w32(p+36,seq);w32(p+40,1);
    w32(p+48,0x03000000);w32(p+52,KFGT_RPC_SIGNATURE);w32(p+56,32);w32(p+60,fn);w32(p+64,result);
    for(i=0;i<80;i+=4)sum^=kf_u32(p+i);
    w32(p+32,sum);
}
static void fixture(FIXTURE *f) {
    unsigned i;memset(f,0,sizeof(*f));w64(f->ram+0x1000,UINT64_C(0x524d41524753));
    w64(f->ram+0x1008,0x2000);w64(f->ram+0x1010,80);f->ram[0x1018]=1;f->ram[0x1019]=1;
    w64(f->ram+0x2000,0x3000);w32(f->ram+0x2008,9);w64(f->ram+0x2010,4096);w64(f->ram+0x2018,20480);
    for(i=0;i<9;i++)w64(f->ram+0x3000+i*8,0x3000+(uint64_t)i*4096);
    header(f->ram+0x4000);message(f->ram+0x5000,0,72,UINT32_MAX);w32(f->ram+0x4010,1);
}
static void boot(VG_OBSERVER *v) {vg_mmio(v,0x110040,0x1000,4,1,1);vg_mmio(v,0x110044,0,4,1,2);}

/* ---- refusal ablation tests ---- */
typedef struct { unsigned n; uint32_t fn[8],key[8],newkey[8],seq[8]; int ok[8]; } RLOG;
static int write_ram(void *opaque,uint64_t pa,const void *in,size_t n) {
    FIXTURE *f=opaque;
    if(pa<0x1000 || pa>=0xc000 || n>0xc000-pa) return 0;
    memcpy(f->ram+pa,in,n);return 1;
}
static void rlog(void *o,const VG_RULE *r,uint32_t seq,uint32_t slot,uint32_t pages,int ok) {
    RLOG *l=o;(void)slot;(void)pages;
    if(l->n<8){l->fn[l->n]=r->fn;l->key[l->n]=r->key;l->newkey[l->n]=r->newkey;l->seq[l->n]=seq;l->ok[l->n]=ok;}
    l->n++;
}
static void rpcmsg(unsigned char *p,uint32_t seq,uint32_t fn,uint32_t key,uint32_t keyoff) {
    unsigned i;uint32_t sum=0;memset(p,0,4096);w32(p+36,seq);w32(p+40,1);
    w32(p+48,0x03000000);w32(p+52,KFGT_RPC_SIGNATURE);w32(p+56,64);w32(p+60,fn);w32(p+64,UINT32_MAX);
    w32(p+keyoff,key);
    for(i=0;i<112;i+=4)sum^=kf_u32(p+i);
    w32(p+32,sum);
}
static int msg_ok(const unsigned char *p) {unsigned i;uint32_t s=0;for(i=0;i<112;i+=4)s^=kf_u32(p+i);return s==0;}
static void refuse_tests(void) {
    VG_OBSERVER *v=calloc(1,sizeof(*v));FIXTURE *f=calloc(1,sizeof(*f));VG_REFUSE r;RLOG l;
    memset(&l,0,sizeof(l));
    assert(v&&f);fixture(f);vg_init(v,read_ram,emit,f);boot(v);assert(v->active);
    vg_refuse_init(&r,write_ram,rlog,&l);r.write=write_ram;
    assert(vg_refuse_default_key(76,0x20809004)==0x2080ff04);
    assert(vg_refuse_default_key(103,0x402c)==0xfff0402c);
    assert(!vg_refuse_add(&r,71,1,0)&&!vg_refuse_add(&r,76,5,5));
    assert(vg_refuse_add(&r,76,0x20809004,0)&&vg_refuse_add(&r,103,0x402c,0x1234));
    assert(!vg_refuse_add_field(&r,103,0x402c,0,1)); /* field 1 only for fn76 */
    assert(vg_refuse_add_field(&r,76,0x1,0,1));      /* a 3rd rule: fn76 cmd 0x1 -> hObject=0 (NULL) */
    /* no doorbell scan with an empty window */
    vg_refuse_scan(v,&r);assert(r.rewritten==0);
    /* queue: slot0 seq0 fn72 (fixture), add slot1 seq1 fn76 match, slot2 seq2 fn103 match; write=3, read=1 */
    rpcmsg(f->ram+0x6000,1,76,0x20809004,88);
    rpcmsg(f->ram+0x7000,2,103,0x402c,92);
    /* geometry: 16384 bytes = 3 entries; slots 0..2 live at 0x5000,0x6000,0x7000 */
    w32(f->ram+0x4010,0);w32(f->ram+0x4020,1);                /* write=0 (after slot2), read=1 */
    vg_refuse_scan(v,&r);
    assert(r.matched==2&&r.rewritten==2&&l.n==2);
    assert(kf_u32(f->ram+0x6000+88)==0x2080ff04&&kf_u32(f->ram+0x7000+92)==0x1234);
    assert(msg_ok(f->ram+0x6000)&&msg_ok(f->ram+0x7000));
    assert(l.fn[0]==76&&l.seq[0]==1&&l.fn[1]==103&&l.seq[1]==2&&l.ok[0]&&l.ok[1]);
    vg_refuse_scan(v,&r);assert(r.rewritten==2); /* seen: never twice */
    /* field 1 (hObject): cmd 0x1 at slot with a nonzero hObject gets it zeroed, cmd stays */
    rpcmsg(f->ram+0x5000,3,76,0x1,88);
    {   /* poke hObject after the checksum, then fix the checksum (the test's own job, not the code under test) */
        uint32_t old=kf_u32(f->ram+0x5000+84);
        w32(f->ram+0x5000+84,0x77);
        w32(f->ram+0x5000+32,kf_u32(f->ram+0x5000+32)^old^0x77);
    }
    assert(msg_ok(f->ram+0x5000));
    w32(f->ram+0x4020,0);w32(f->ram+0x4010,1);           /* read=0 (GSP consumed through slot2), write=1 (slot0 new) */
    vg_refuse_scan(v,&r);
    assert(r.matched==3&&r.rewritten==3&&l.n==3);
    assert(kf_u32(f->ram+0x5000+88)==0x1&&kf_u32(f->ram+0x5000+84)==0);
    assert(msg_ok(f->ram+0x5000));
    free(f);free(v);
}
int main(void) {
    VG_OBSERVER *v=calloc(1,sizeof(*v));FIXTURE *f=calloc(1,sizeof(*f));unsigned i;
    assert(v&&f);fixture(f);vg_init(v,read_ram,emit,f);boot(v);
    assert(v->active&&v->stats.attached==1&&f->emitted==1);
    assert(f->last.queue_sequence==0&&f->last.flags&KFGT_PREFIX_UNKNOWN);
    assert(v->stats.uninitialized_headers==1); /* command prefix before firmware status init */
    header(f->ram+0x8000);message(f->ram+0x9000,0,4097,0);w32(f->ram+0x8010,1);vg_poll(v,3);
    assert(f->emitted==2&&f->last.direction==1&&f->last.flags&KFGT_PREFIX_UNKNOWN);
    vg_poll(v,4);assert(f->emitted==2); /* no duplicate retained history */
    message(f->ram+0x6000,2,76,UINT32_MAX);w32(f->ram+0x4010,2);vg_mmio(v,0x110c00,0,4,1,5);
    assert(f->emitted==3&&f->last.missing_before==1&&v->stats.gaps==1);
    f->mutate=1;vg_poll(v,6);assert(v->stats.unstable==1);f->mutate=0;
    w64(f->ram+0x3010,0xc000);vg_poll(v,7);assert(!v->active&&v->stats.mapping_changed==1);
    fixture(f);vg_reset(v);boot(v);assert(v->active&&v->generation==2&&f->gen==2);
    /* Reject malformed bootstrap sizes, duplicate pages, non-RAM and wrapped pointers. */
    for(i=0;i<5;i++) {
        fixture(f);vg_reset(v);
        if(i==0)w32(f->ram+0x2008,513);
        if(i==1)w64(f->ram+0x3010,0x3000);
        if(i==2)w64(f->ram+0x3008,0xc000);
        if(i==3)w64(f->ram+0x1008,UINT64_MAX-8);
        if(i==4)w64(f->ram+0x2018,4096);
        boot(v);assert(!v->active&&f->emitted==0);
    }
    /* High mailbox alone or unrelated MMIO cannot attach. */
    fixture(f);vg_reset(v);vg_mmio(v,0x110044,0,4,1,8);assert(!v->active);
    vg_mmio(v,0x1000,0x1000,4,1,9);assert(!v->active);
    free(f);free(v);refuse_tests();puts("VFIO GSP bootstrap/queue core tests passed");return 0;
}
