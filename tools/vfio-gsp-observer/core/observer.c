/* SPDX-License-Identifier: GPL-2.0-or-later
 * Bootstrap offsets: compiler output from OGKM 580.65.06, see layout.c.
 * No NVIDIA implementation code copied. No scanning or memory writes.
 */
#include "observer.h"
#include <string.h>
static int read_ram(VG_OBSERVER *v,uint64_t pa,void *out,size_t n) {
    if(!n || pa>UINT64_MAX-n || !v->read(v->opaque,pa,out,n)) {
        v->stats.read_failures++;return 0;
    }
    return 1;
}
void vg_init(VG_OBSERVER *v,VG_READ read,VG_EMIT emit,void *opaque) {
    memset(v,0,sizeof(*v));v->read=read;v->emit=emit;v->opaque=opaque;
}
void vg_reset(VG_OBSERVER *v) {
    v->active=0;v->mailbox_seen=0;memset(v->cursor,0,sizeof(v->cursor));
}
static int bootstrap(VG_OBSERVER *v,uint64_t libos) {
    unsigned i,j,found=0;uint64_t arg=0,arg_size=0,cmd,stat,total;unsigned char mq[32];
    v->stats.bootstrap_attempts++;
    if(!libos || (libos&4095) || !read_ram(v,libos,v->libos,sizeof(v->libos))) return 0;
    for(i=0;i<4096;i+=32) {
        const unsigned char *p=v->libos+i;
        if(kf_u64(p)==UINT64_C(0x524d41524753)) { /* numeric id for RMARGS */
            if(++found!=1 || p[24]!=1 || p[25]!=1) return 0;
            arg=kf_u64(p+8);arg_size=kf_u64(p+16);
        }
    }
    if(found!=1 || !arg || arg_size<sizeof(mq) || arg_size>4096 ||
       !read_ram(v,arg,mq,sizeof(mq))) return 0;
    v->table=kf_u64(mq);v->pages=kf_u32(mq+8);
    cmd=kf_u64(mq+16);stat=kf_u64(mq+24);
    if(!v->table || (v->table&4095) || v->pages<5 || v->pages>KFGT_MAX_PAGES ||
       cmd!=4096 || stat<=cmd || (stat&4095)) return 0;
    total=(uint64_t)v->pages*4096;
    if(stat>=total || stat-cmd<8192 || total-stat<8192 ||
       stat-cmd>KFGT_MAX_QUEUE || total-stat>KFGT_MAX_QUEUE) return 0;
    if(!read_ram(v,v->table,v->table_copy,v->pages*8) || kf_u64(v->table_copy)!=v->table) return 0;
    for(i=0;i<v->pages;i++) {
        v->ptes[i]=kf_u64(v->table_copy+i*8);
        if(!v->ptes[i] || (v->ptes[i]&4095) || !read_ram(v,v->ptes[i],NULL,4096)) return 0;
        for(j=0;j<i;j++) if(v->ptes[i]==v->ptes[j]) return 0;
    }
    v->offset[0]=(uint32_t)cmd;v->offset[1]=(uint32_t)stat;
    v->size[0]=(uint32_t)(stat-cmd);v->size[1]=(uint32_t)(total-stat);
    memset(v->cursor,0,sizeof(v->cursor));v->generation++;v->active=1;v->stats.attached++;
    return 1;
}
static int mapping(VG_OBSERVER *v) {
    unsigned i;
    if(!read_ram(v,v->table,v->table_copy,v->pages*8)) return 0;
    for(i=0;i<v->pages;i++) if(kf_u64(v->table_copy+i*8)!=v->ptes[i]) return 0;
    return 1;
}
static int copy_queue(VG_OBSERVER *v,unsigned direction,unsigned char *out) {
    unsigned i,first=v->offset[direction]/4096;
    for(i=0;i<v->size[direction]/4096;i++) {
        if(!read_ram(v,v->ptes[first+i],out+i*4096,4096)) return 0;
    }
    return 1;
}
static void emit(void *opaque,const KFGT_RECORD *record,const unsigned char *payload) {
    VG_OBSERVER *v=opaque;v->stats.records++;
    v->emit(v->opaque,v->generation,record,payload);
}
void vg_poll(VG_OBSERVER *v,uint64_t ns) {
    unsigned d;v->stats.triggers++;
    if(!v->active) return;
    if(!mapping(v)) {v->stats.mapping_changed++;v->active=0;return;}
    for(d=0;d<2;d++) {
        KFGT_QUEUE h;uint64_t old_gaps=v->cursor[d].gaps,old_invalid=v->cursor[d].invalid;
        if(!copy_queue(v,d,v->a) || !copy_queue(v,d,v->b)) return;
        if(!mapping(v)) {v->stats.mapping_changed++;v->active=0;return;}
        if(memcmp(v->a,v->b,v->size[d])) {v->stats.unstable++;continue;}
        if(!kf_queue_header(v->a,v->size[d],&h) || h.size!=v->size[d]) {
            if(!memcmp(v->a,(unsigned char[36]){0},36)) v->stats.uninitialized_headers++;
            else v->stats.invalid_headers++;
            continue;
        }
        (void)kf_queue_records(v->a,v->size[d],v->scratch,&v->cursor[d],d,v->table,ns,emit,v);
        v->stats.gaps+=v->cursor[d].gaps-old_gaps;
        v->stats.invalid_elements+=v->cursor[d].invalid-old_invalid;
    }
}
void vg_mmio(VG_OBSERVER *v,uint64_t off,uint64_t data,unsigned size,int write,uint64_t ns) {
    if(write && size==4 && off==0x110040) {
        v->active=0;v->mailbox_low=(uint32_t)data;v->mailbox_seen=1;return;
    }
    if(write && size==4 && off==0x110044 && v->mailbox_seen) {
        v->mailbox_seen=0;
        if(!bootstrap(v,((uint64_t)(uint32_t)data<<32)|v->mailbox_low)) v->stats.invalid_bootstrap++;
        vg_poll(v,ns);return;
    }
    if((write && size==4 && off>=0x110c00 && off<0x110c40 && !(off&7)) ||
       off==0x110004 || off==0x110008) vg_poll(v,ns);
}
