/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Wire offsets derived from NVIDIA OGKM b81d58ee (MIT): msgq_priv.h,
 * message_queue_priv.h and g_rpc-message-header.h. No NVIDIA code copied.
 */
#include "queue.h"
#include <string.h>
uint32_t kf_u32(const void *v) { const unsigned char *p=v; return (uint32_t)p[0]|((uint32_t)p[1]<<8)|((uint32_t)p[2]<<16)|((uint32_t)p[3]<<24); }
uint64_t kf_u64(const void *v) { const unsigned char *p=v; return kf_u32(p)|((uint64_t)kf_u32(p+4)<<32); }
int kf_queue_header(const unsigned char *p, size_t n, KFGT_QUEUE *q) {
    if(n<36 || kf_u32(p)!=0 || kf_u32(p+8)!=KFGT_PAGE) return 0;
    q->size=kf_u32(p+4); q->count=kf_u32(p+12); q->write=kf_u32(p+16);
    q->rx_offset=kf_u32(p+24); q->entry_offset=kf_u32(p+28);
    if(q->size<2*KFGT_PAGE || q->size>KFGT_MAX_QUEUE || q->size%KFGT_PAGE ||
       q->entry_offset!=KFGT_PAGE || q->rx_offset!=32 || kf_u32(p+20)>1 ||
       q->count!=(q->size/KFGT_PAGE)-1 || q->write>=q->count || kf_u32(p+32)>=q->count) return 0;
    return 1;
}
static int element(const unsigned char *p,const KFGT_QUEUE *q,uint32_t slot,
                   unsigned char *scratch,uint32_t *seq,uint32_t *pages,uint32_t *bytes) {
    const unsigned char *h=p+q->entry_offset+slot*KFGT_PAGE;
    uint32_t i, length, checksum=0;
    *pages=kf_u32(h+40); *seq=kf_u32(h+36); length=kf_u32(h+56);
    if(kf_u32(h+48)!=0x03000000u || kf_u32(h+52)!=KFGT_RPC_SIGNATURE || length<32 || length>KFGT_MAX_MESSAGE-48 ||
       !*pages || *pages>16 || *pages>=q->count || *pages!=(48+length+KFGT_PAGE-1)/KFGT_PAGE) return 0;
    *bytes=48+length;
    for(i=0;i<*pages;i++) memcpy(scratch+i*KFGT_PAGE,p+q->entry_offset+((slot+i)%q->count)*KFGT_PAGE,KFGT_PAGE);
    /* Ordinary (non-CC) messages XOR to zero through the next 8-byte boundary. */
    for(i=0;i<((*bytes+7u)&~7u);i+=4) checksum^=kf_u32(scratch+i);
    if(checksum) return 0;
    /* Nonzero authentication data is unsupported encrypted/CC traffic. */
    for(i=0;i<32;i++) if(scratch[i]) return 0;
    return 1;
}
int kf_queue_records(const unsigned char *p,size_t n,unsigned char *scratch,KFGT_CURSOR *cursor,
                    uint32_t direction,uint64_t table,uint64_t qpc,KFGT_EMIT emit,void *context) {
    KFGT_QUEUE q; uint32_t i,s,seq,pages,bytes,latest=0,tail=0,found=0;
    uint32_t slots[256], seqs[256], count=0, j;
    if(!kf_queue_header(p,n,&q) || n!=q.size || direction>1) return 0;
    /* A committed final message ends exactly at writePtr. This bounds which
       older checksummed slots were published; producer may be filling future slots. */
    for(i=1;i<=16 && i<q.count;i++) {
        s=(q.write+q.count-i)%q.count;
        if(element(p,&q,s,scratch,&seq,&pages,&bytes) && pages==i) { latest=seq;tail=s;found++; }
    }
    if(found!=1) return 1; /* empty, unsupported, or ambiguous snapshot */
    for(s=0;s<q.count;s++) {
        if(!element(p,&q,s,scratch,&seq,&pages,&bytes)) { cursor->invalid++; continue; }
        if((uint32_t)(latest-seq)>=q.count) continue;
        /* Reject uncommitted pages after the published end. */
        for(i=0;i<pages;i++) if((s+i)%q.count==q.write) break;
        if(i!=pages || (s!=tail && seq==latest)) continue;
        if(cursor->initialized && (uint32_t)(seq-cursor->last)>=0x80000000u) continue;
        if(cursor->initialized && seq==cursor->last) continue;
        for(j=count;j && (uint32_t)(latest-seqs[j-1])<(uint32_t)(latest-seq);j--) {
            slots[j]=slots[j-1];seqs[j]=seqs[j-1];
        }
        slots[j]=s;seqs[j]=seq;count++;
    }
    for(i=0;i<count;i++) {
        KFGT_RECORD r;
        if(!element(p,&q,slots[i],scratch,&seq,&pages,&bytes)) return 0;
        if(cursor->initialized && seq==cursor->last) continue;
        memset(&r,0,sizeof(r));
        r.magic=KFGT_RECORD_MAGIC;r.header_bytes=sizeof(r);r.payload_bytes=(bytes+7u)&~7u;
        r.direction=direction;r.table_pa=table;r.qpc=qpc;r.queue_sequence=seq;
        r.rpc_sequence=kf_u32(scratch+72);r.rpc_function=kf_u32(scratch+60);
        r.rpc_result=kf_u32(scratch+64);r.rpc_version=kf_u32(scratch+48);r.flags=KFGT_SAMPLED;
        if(!cursor->initialized) r.flags|=KFGT_PREFIX_UNKNOWN;
        else if((uint32_t)(seq-cursor->last)!=1) {
            r.flags|=KFGT_GAP_BEFORE;r.missing_before=seq-cursor->last-1;cursor->gaps+=r.missing_before;
        }
        emit(context,&r,scratch);cursor->last=seq;cursor->initialized=1;
    }
    return 1;
}
