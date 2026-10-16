/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "observer.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef struct { unsigned char ram[65536];unsigned emitted,reads,mutate;KFGT_RECORD last;uint64_t gen;unsigned char payload[KFGT_MAX_MESSAGE]; } FIXTURE;
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
    FIXTURE *f=opaque;assert(r->payload_bytes<=KFGT_MAX_MESSAGE);assert(kf_u32(payload+52)==KFGT_RPC_SIGNATURE);
    memcpy(f->payload,payload,r->payload_bytes);
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
    /* GH100/GB20x mailbox route names FMC parameters, not LibOS directly.
       Synthetic public layout: gspRmParams.target at 40, bootArgsOffset at 48. */
    fixture(f);vg_reset(v);w32(f->ram+0xb028,1);w64(f->ram+0xb030,0x1000);
    vg_mmio(v,0x110040,0xb000,4,1,12);vg_mmio(v,0x110044,0,4,1,13);
    assert(v->active&&f->emitted==1&&f->last.queue_sequence==0);
    header(f->ram+0x8000);message(f->ram+0x9000,0,4097,0);w32(f->ram+0x8010,1);
    vg_poll(v,14);assert(f->emitted==2&&f->last.direction==1&&f->last.queue_sequence==0);
    /* A single indirection, only system apertures, with all original queue
       validation retained. Never walk recursive guest pointer chains. */
    for(i=0;i<10;i++) {
        fixture(f);vg_reset(v);w32(f->ram+0xb028,1);w64(f->ram+0xb030,0x1000);
        if(i==0)w32(f->ram+0xb028,0); /* local FB is not guest RAM */
        if(i==1)w32(f->ram+0xb028,3); /* unknown aperture */
        if(i==2)w64(f->ram+0xb030,0);
        if(i==3)w64(f->ram+0xb030,0x1001);
        if(i==4)w64(f->ram+0xb030,0xc000);
        if(i==5)w64(f->ram+0xb030,UINT64_MAX-7);
        if(i==6)w64(f->ram+0xb030,0xb000); /* self */
        if(i==7) { /* valid-looking second FMC must not be followed */
            w64(f->ram+0xb030,0xa000);w32(f->ram+0xa028,1);w64(f->ram+0xa030,0x1000);
        }
        if(i==8)w32(f->ram+0x2008,513);
        if(i==9)w64(f->ram+0x3010,0x3000);
        vg_mmio(v,0x110040,0xb000,4,1,15);vg_mmio(v,0x110044,0,4,1,16);
        assert(!v->active&&f->emitted==0);
        assert(f->reads<40); /* fixed probe count, no scan or recursive walk */
    }
    fixture(f);vg_reset(v);w32(f->ram+0xb028,2);w64(f->ram+0xb030,0x1000);
    vg_mmio(v,0x110040,0xb000,4,1,17);vg_mmio(v,0x110044,0,4,1,18);
    assert(v->active&&f->emitted==1);
    /* fn103 declares params outside rpc.length but the committed transport
       element contains them. Capture only the bounded declared tail, mark it
       separately from the checksummed prefix, never include page padding. */
    fixture(f);vg_reset(v);
    {
        unsigned char *p=f->ram+0x5000;uint32_t sum=0;
        message(p,0,103,0);w32(p+56,64);w32(p+80,1);w32(p+84,2);
        w32(p+88,3);w32(p+92,0xc56f);w32(p+100,368);
        for(i=0;i<368;i++)p[112+i]=(unsigned char)(i^0x5a);
        w32(p+32,0);for(i=0;i<112;i+=4) { sum^=kf_u32(p+i); }
        w32(p+32,sum);
        boot(v);assert(f->emitted==1);
        assert(f->last.payload_bytes==480&&f->last.flags&KFGT_ALLOC_PARAMS_TAIL);
        assert(!memcmp(f->payload+112,p+112,368));
        /* A too-large params claim gets the valid prefix only, no tail flag. */
        w32(p+36,1);w32(p+100,4096);w32(p+32,0);sum=0;
        for(i=0;i<112;i+=4) { sum^=kf_u32(p+i); }
        w32(p+32,sum);
        vg_poll(v,10);assert(f->emitted==2);
        assert(f->last.payload_bytes==112&&!(f->last.flags&KFGT_ALLOC_PARAMS_TAIL));
        /* Exact remaining element extent is permitted. */
        w32(p+36,2);w32(p+100,4096-112);w32(p+32,0);sum=0;
        for(i=0;i<112;i+=4) { sum^=kf_u32(p+i); }
        w32(p+32,sum);
        vg_poll(v,11);assert(f->emitted==3&&f->last.payload_bytes==4096);
    }
    free(f);free(v);puts("VFIO GSP bootstrap/queue core tests passed");return 0;
}
