/* SPDX-License-Identifier: GPL-2.0-or-later
 * Bootstrap offsets: compiler output from OGKM 580.65.06, see layout.c.
 * No NVIDIA implementation code copied.
 * 2026-10-09: vg_refuse_scan() (observer.h) is the one place that writes guest RAM, and only when
 * a caller has turned the DEBUG-ONLY refusal ablation on (see gsp-observer.c, x-gsp-refuse). With
 * no rules loaded (vg_refuse_init() never called, or VG_REFUSE.write/nrules unset) nothing here
 * writes anything; vg_init()/vg_mmio()/vg_poll() are unchanged and still never write RAM.
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

/* ---- refusal ablation (see observer.h) ---- */
void vg_refuse_init(VG_REFUSE *r,VG_WRITE w,VG_REFUSE_LOG log,void *opaque) {
    memset(r,0,sizeof(*r));r->write=w;r->log=log;r->opaque=opaque;
}
uint32_t vg_refuse_default_key(uint32_t fn,uint32_t key) {
    /* Same class bits, command index moved into 0xff00-0xffff: no such method exists on any object. */
    if(fn==76) return (key&0xffff0000u)|0xff00u|(key&0xffu);
    if(fn==103) return 0xfff00000u|(key&0xffffu);
    return 0;
}
int vg_refuse_add(VG_REFUSE *r,uint32_t fn,uint32_t key,uint32_t newkey) {
    return vg_refuse_add_field(r,fn,key,newkey,0);
}
int vg_refuse_add_field(VG_REFUSE *r,uint32_t fn,uint32_t key,uint32_t newkey,unsigned field) {
    if(r->nrules>=VG_REFUSE_MAX_RULES || (fn!=76 && fn!=103) || (field && (field!=1 || fn!=76))) return 0;
    if(!field) {
        if(!newkey) newkey=vg_refuse_default_key(fn,key);
        if(!newkey || newkey==key) return 0;              /* key: the matched field (cmd/hClass) */
    }
    /* field==1 (hObject): newkey is the replacement handle, 0 (NULL) is a valid choice; it is
     * compared against the ORIGINAL HANDLE at scan time, not against `key` (the matched cmd). */
    r->rule[r->nrules].fn=fn;r->rule[r->nrules].key=key;r->rule[r->nrules].newkey=newkey;
    r->rule[r->nrules].field=field;r->rule[r->nrules].hits=0;r->nrules++;
    return 1;
}
static uint32_t xor_words(const unsigned char *p,uint32_t bytes) {
    uint32_t i,s=0;for(i=0;i<bytes;i+=4) s^=kf_u32(p+i);return s;
}
/* The committed message that ENDS at element `end` with `i` pages, read into msg[] (i*4096 bytes).
 * 1 if the element header and checksum are valid. */
static int load_message(VG_OBSERVER *v,unsigned first,const KFGT_QUEUE *q,uint32_t slot,uint32_t i,
                        unsigned char *msg,uint32_t *seq) {
    uint32_t k,length,bytes;
    for(k=0;k<i;k++) {
        if(!read_ram(v,v->ptes[first+1+(slot+k)%q->count],msg+(size_t)k*KFGT_PAGE,KFGT_PAGE)) return 0;
    }
    length=kf_u32(msg+56);
    if(kf_u32(msg+48)!=0x03000000u || kf_u32(msg+52)!=KFGT_RPC_SIGNATURE || length<32 ||
       length>KFGT_MAX_MESSAGE-48 || kf_u32(msg+40)!=i || i!=(48+length+KFGT_PAGE-1)/KFGT_PAGE) return 0;
    bytes=(48+length+7u)&~7u;
    if(xor_words(msg,bytes)) return 0;
    *seq=kf_u32(msg+36);return 1;
}
void vg_refuse_scan(VG_OBSERVER *v,VG_REFUSE *r) {
    unsigned char hdr[KFGT_PAGE];
    static unsigned char msg[KFGT_MAX_MESSAGE],chk[KFGT_MAX_MESSAGE];
    struct { uint32_t slot,pages,seq; } m[32];
    KFGT_QUEUE q;unsigned first,nm=0,j,x;uint32_t end,readp,unread,total=0;
    r->doorbells++;
    if(!r->nrules || !r->write) return;
    if(!v->active) {r->inactive++;return;}
    if(r->generation!=v->generation) {r->generation=v->generation;r->have_next=0;}
    first=v->offset[0]/KFGT_PAGE;
    if(!read_ram(v,v->ptes[first],hdr,KFGT_PAGE) || !kf_queue_header(hdr,sizeof(hdr),&q) ||
       q.size!=v->size[0]) {r->header_bad++;return;}
    end=q.write;readp=kf_u32(hdr+32);
    unread=(end+q.count-readp)%q.count;
    while(nm<32 && total<unread) {
        uint32_t i,found=0,fseq=0,best=0;
        for(i=1;i<=16 && i<q.count;i++) {
            uint32_t s=(end+q.count-i)%q.count,seq;
            if(load_message(v,first,&q,s,i,msg,&seq)) {
                if(!found || (uint32_t)(seq-fseq)<0x80000000u) {fseq=seq;best=i;}
                found++;
            }
        }
        if(!found) break;
        if(found>1) r->ambiguous++;
        if(total+best>unread) break;                         /* straddles the GSP's read pointer: consumed */
        if(r->have_next && (uint32_t)(fseq-r->next_seq)>=0x80000000u) break; /* already seen */
        m[nm].slot=(end+q.count-best)%q.count;m[nm].pages=best;m[nm].seq=fseq;nm++;
        total+=best;end=m[nm-1].slot;
    }
    if(!nm) return;
    if(nm==32) r->truncated++;
    if(r->have_next && m[nm-1].seq!=r->next_seq) r->seq_gaps+=(uint32_t)(m[nm-1].seq-r->next_seq);
    for(j=nm;j--;) {                                           /* oldest first */
        uint32_t seq,fn,keyoff,key,newkey=0,old_sum,ok=0;unsigned field=0,ruleidx=0,hit=0;
        r->scanned++;
        if(!load_message(v,first,&q,m[j].slot,m[j].pages,msg,&seq)) continue;
        fn=kf_u32(msg+60);
        if(fn!=76 && fn!=103) continue;
        keyoff=fn==76?88:92;key=kf_u32(msg+keyoff);
        for(x=0;x<r->nrules;x++) if(r->rule[x].fn==fn && r->rule[x].key==key) {
            newkey=r->rule[x].newkey;field=r->rule[x].field;ruleidx=x;hit=1;break;
        }
        if(!hit) continue;
        if(field==1) {
            keyoff=84; /* fn76 hObject, in place of cmd */
            if(kf_u32(msg+keyoff)==newkey) continue; /* already that handle: nothing to rewrite */
        }
        r->matched++;
        old_sum=kf_u32(msg+32);
        /* one write of bytes 32..keyoff+3 of the first page: checksum, seq, count, header, the
         * rewritten field. The checksum delta is old-value-AT-keyoff XOR newkey: for field 0,
         * keyoff is where `key` was read from, so they are the same value; for field 1 (hObject)
         * `key` is the matched cmd at a DIFFERENT offset, so the old hObject must be re-read. */
        {
            unsigned char patch[64];
            uint32_t oldval=field?kf_u32(msg+keyoff):key;
            uint32_t len=keyoff+4-32,ns=old_sum^oldval^newkey,k;
            memcpy(patch,msg+32,len);
            for(k=0;k<4;k++) {patch[k]=(unsigned char)(ns>>(8*k));patch[keyoff-32+k]=(unsigned char)(newkey>>(8*k));}
            if(r->write(v->opaque,v->ptes[first+1+m[j].slot%q.count]+32,patch,len)) {
                ok=load_message(v,first,&q,m[j].slot,m[j].pages,chk,&seq) && kf_u32(chk+keyoff)==newkey;
                if(!ok) {  /* restore the original bytes */
                    r->write(v->opaque,v->ptes[first+1+m[j].slot%q.count]+32,msg+32,len);
                }
            }
        }
        if(ok) {r->rewritten++;} else r->verify_failed++;
        r->rule[ruleidx].hits+=ok;
        if(r->log) {VG_RULE shown;shown.fn=fn;shown.key=key;shown.newkey=newkey;shown.hits=0;
                    r->log(r->opaque,&shown,seq,m[j].slot,m[j].pages,ok);}
    }
    r->next_seq=m[0].seq+1;r->have_next=1;
}
