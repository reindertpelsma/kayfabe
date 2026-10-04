/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#include "../queue.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static unsigned char ring[32*KFGT_PAGE],scratch[KFGT_MAX_MESSAGE],message[KFGT_MAX_MESSAGE];
static KFGT_RECORD records[256];static unsigned emitted;
static void w32(void *v,uint32_t x) { unsigned char *p=v;p[0]=(unsigned char)x;p[1]=(unsigned char)(x>>8);p[2]=(unsigned char)(x>>16);p[3]=(unsigned char)(x>>24); }
static void setup(void) {
    memset(ring,0,sizeof(ring));emitted=0;
    w32(ring+4,sizeof(ring));w32(ring+8,KFGT_PAGE);w32(ring+12,31);w32(ring+24,32);w32(ring+28,KFGT_PAGE);
}
static void place(uint32_t slot,uint32_t seq,uint32_t length) {
    uint32_t i,pages=(48+length+4095)/4096,xor=0;
    memset(message,0,sizeof(message));w32(message+36,seq);w32(message+40,pages);
    w32(message+48,0x03000000);w32(message+52,KFGT_RPC_SIGNATURE);w32(message+56,length);
    w32(message+60,76);w32(message+72,seq+100);w32(message+80+8,0x2080121f);
    for(i=0;i<((48+length+7)&~7u);i+=4) xor^=kf_u32(message+i);
    w32(message+32,xor);
    for(i=0;i<pages;i++) memcpy(ring+KFGT_PAGE+((slot+i)%31)*KFGT_PAGE,message+i*KFGT_PAGE,KFGT_PAGE);
    w32(ring+16,(slot+pages)%31);
}
static void emit(void *unused,const KFGT_RECORD *r,const unsigned char *data) {
    (void)unused;assert(emitted<256);records[emitted++]=*r;
    assert(kf_u32(data+52)==KFGT_RPC_SIGNATURE);assert(r->payload_bytes>=80);
}
static void poll(KFGT_CURSOR *c) { assert(kf_queue_records(ring,sizeof(ring),scratch,c,0,0x1000,99,emit,NULL)); }
int main(void) {
    KFGT_CURSOR c={0};KFGT_QUEUE q;unsigned i,j;uint32_t random=0x12345678;
    assert(sizeof(KFGT_RECORD)==64 && sizeof(KFGT_FILE_HEADER)==64);
    setup();poll(&c);assert(!emitted);
    place(0,0,112);place(1,1,113);poll(&c);
    assert(emitted==2 && records[0].flags==(KFGT_SAMPLED|KFGT_PREFIX_UNKNOWN));
    assert(records[1].payload_bytes==168 && records[1].queue_sequence==1);poll(&c);assert(emitted==2);
    place(2,4,112);poll(&c);assert(emitted==3 && records[2].missing_before==2 && c.gaps==2);
    setup();memset(&c,0,sizeof(c));place(30,20,6000);poll(&c);assert(emitted==1 && records[0].payload_bytes==6048);
    ring[KFGT_PAGE+30*KFGT_PAGE+99]^=1;memset(&c,0,sizeof(c));emitted=0;poll(&c);assert(!emitted);
    setup();memset(&c,0,sizeof(c));place(0,UINT32_MAX-1,112);place(1,UINT32_MAX,112);place(2,0,112);poll(&c);
    assert(emitted==3 && c.last==0 && c.gaps==0);
    setup();memset(&c,0,sizeof(c));place(0,1,112);place(1,2,112);w32(ring+16,1);poll(&c);
    assert(emitted==1 && records[0].queue_sequence==1); /* slot 1 filled but not submitted */
    setup();w32(ring+12,UINT32_MAX);assert(!kf_queue_header(ring,sizeof(ring),&q));
    setup();w32(ring+24,UINT32_MAX);assert(!kf_queue_header(ring,sizeof(ring),&q));
    /* Deterministic malformed queues under ASan/UBSan: valid outer geometry,
       arbitrary elements and write pointers, including adversarial size words. */
    for(i=0;i<1000;i++) {
        setup();memset(&c,0,sizeof(c));
        for(j=KFGT_PAGE;j<sizeof(ring);j+=4) { random=random*1664525u+1013904223u;w32(ring+j,random); }
        w32(ring+16,i%31);poll(&c);assert(!emitted);
    }
    puts("queue observer tests passed: chronology, gaps, wrapping, checksum, publication, malformed input");return 0;
}
