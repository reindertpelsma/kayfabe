/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "trace_protocol.h"
static volatile LONG interrupted;
static BOOL WINAPI ctrl(DWORD event) { (void)event;InterlockedExchange(&interrupted,1);return TRUE; }
static int stats(HANDLE h,KFGT_STATS *s) {
    DWORD n=0;
    return DeviceIoControl(h,KFGT_IOCTL_STATS,NULL,0,s,sizeof(*s),&n,NULL) && n==sizeof(*s) && s->version==KFGT_ABI && s->bytes==sizeof(*s);
}
static int print_stats(FILE *f,const KFGT_STATS *s) {
    int written=fprintf(f,"{\"schema\":\"kayfabe-gsp-observer/1\",\"complete\":false,\"scanned_bytes\":%llu,\"scan_passes\":%llu,\"candidates\":%llu,\"attached_tables\":%llu,\"read_failures\":%llu,\"unstable_snapshots\":%llu,\"invalid_elements\":%llu,\"recorded\":%llu,\"dropped\":%llu,\"sequence_gaps\":%llu,\"buffered_bytes\":%llu}\n",
      (unsigned long long)s->scanned_bytes,(unsigned long long)s->scan_passes,(unsigned long long)s->candidates,(unsigned long long)s->attached_tables,
      (unsigned long long)s->read_failures,(unsigned long long)s->unstable_snapshots,(unsigned long long)s->invalid_elements,
      (unsigned long long)s->recorded,(unsigned long long)s->dropped,(unsigned long long)s->observed_sequence_gaps,(unsigned long long)s->buffered_bytes);
    if(written<0 || fflush(f)) { perror("write capture statistics");return 0; }
    return 1;
}
static int append_trace(FILE *f,const void *data,size_t bytes) {
    if(fwrite(data,1,bytes,f)!=bytes || fflush(f)) { perror("write/flush trace");return 0; }
    return 1;
}
/* Marker contents are ignored. Caller chooses a path in a protected directory. */
static int stop_marker(const char *path) {
    DWORD attributes=GetFileAttributesA(path),error;
    if(attributes!=INVALID_FILE_ATTRIBUTES) {
        if(attributes&FILE_ATTRIBUTE_DIRECTORY) { fprintf(stderr,"Stop marker is a directory.\n");return -1; }
        return 1;
    }
    error=GetLastError();
    if(error==ERROR_FILE_NOT_FOUND || error==ERROR_PATH_NOT_FOUND) return 0;
    fprintf(stderr,"Cannot inspect stop marker: Win32 %lu\n",error);return -1;
}
int main(int argc,char **argv) {
    HANDLE h; FILE *f=NULL,*meta=NULL; unsigned char *buffer; KFGT_STATS s; KFGT_FILE_HEADER header;
    DWORD n=0; ULONGLONG end; unsigned long seconds; char *tail; char meta_name[MAX_PATH],stop_path[MAX_PATH],output_path[MAX_PATH]; int rc=1,stop_error=0;
    const char *stop_file=NULL; ULONGLONG next_stop_poll=0;
    if(argc==2 && !strcmp(argv[1],"--status")) seconds=0;
    else if(argc==3 || (argc==5 && !strcmp(argv[3],"--stop-file"))) {
        seconds=strtoul(argv[2],&tail,10);
        if(!*argv[2] || *tail || !seconds || seconds>86400) { fprintf(stderr,"seconds must be 1..86400\n");return 2; }
    } else { fprintf(stderr,"Usage: gsptrace.exe OUTPUT.kgwt SECONDS [--stop-file PATH] | --status\n");return 2; }
    if(argc==5) {
        DWORD a=GetFullPathNameA(argv[4],MAX_PATH,stop_path,NULL),b=GetFullPathNameA(argv[1],MAX_PATH,output_path,NULL);
        if(!*argv[4] || !a || a>=MAX_PATH || !b || b>=MAX_PATH || !_stricmp(stop_path,output_path)) {
            fprintf(stderr,"Stop marker must be a valid separate path shorter than MAX_PATH.\n");return 2;
        }
        if(stop_marker(stop_path)!=0) {
            fprintf(stderr,"Stop marker must be absent before capture; remove an old marker explicitly.\n");return 2;
        }
        stop_file=stop_path;next_stop_poll=GetTickCount64()+250;
    }
    h=CreateFileW(L"\\\\.\\KayfabeGspTrace",GENERIC_READ,0,NULL,OPEN_EXISTING,0,NULL);
    if(h==INVALID_HANDLE_VALUE) { fprintf(stderr,"Open driver: Win32 %lu (elevate, then start the test-signed driver)\n",GetLastError());return 1; }
    if(!stats(h,&s)) { fprintf(stderr,"Unsupported driver ABI or stats failed: %lu\n",GetLastError());CloseHandle(h);return 1; }
    if(!seconds) { rc=print_stats(stdout,&s)?0:1;CloseHandle(h);return rc; }
    if(strlen(argv[1])+12>=sizeof(meta_name)) { CloseHandle(h);return 2; }
    strcpy(meta_name,argv[1]);strcat(meta_name,".stats.json");
    f=fopen(argv[1],"wb");buffer=malloc(1024*1024);
    if(!f || !buffer) { fprintf(stderr,"Cannot create output or allocate read buffer\n");if(f)fclose(f);free(buffer);CloseHandle(h);return 1; }
    memset(&header,0,sizeof(header));header.magic=KFGT_FILE_MAGIC;header.version=KFGT_ABI;
    header.header_bytes=sizeof(header);header.record_header_bytes=sizeof(KFGT_RECORD);
    header.qpc_frequency=s.qpc_frequency;header.started_qpc=s.started_qpc;header.flags=KFGT_SAMPLED;
    if(!append_trace(f,&header,sizeof(header))) goto done;
    SetConsoleCtrlHandler(ctrl,TRUE);end=GetTickCount64()+seconds*1000ull;
    while(GetTickCount64()<end && !interrupted) {
        ULONGLONG now=GetTickCount64();
        if(stop_file && now>=next_stop_poll) {
            int marker=stop_marker(stop_file);next_stop_poll=now+250;
            if(marker) { stop_error=marker<0;fprintf(stderr,marker>0?"Stop marker observed; draining capture.\n":"Stop marker error; draining capture before failure.\n");break; }
        }
        if(!DeviceIoControl(h,KFGT_IOCTL_READ,NULL,0,buffer,1024*1024,&n,NULL)) goto done;
        if(n && !append_trace(f,buffer,n)) goto done;
        if(!n) Sleep(10);
    }
    /* Stop worker before final drain, ensuring no record is cut at the end. */
    if(!DeviceIoControl(h,KFGT_IOCTL_STOP,NULL,0,NULL,0,&n,NULL)) goto done;
    do {
        if(!DeviceIoControl(h,KFGT_IOCTL_READ,NULL,0,buffer,1024*1024,&n,NULL)) goto done;
        if(n && !append_trace(f,buffer,n)) goto done;
    } while(n);
    if(!stats(h,&s)) goto done;
    if(!print_stats(stdout,&s)) goto done;
    meta=fopen(meta_name,"w");
    if(!meta) goto done;
    if(!print_stats(meta,&s)) goto done;
    if(fclose(meta)) { perror("close capture statistics");meta=NULL;goto done; }meta=NULL;
    rc=stop_error?1:(s.recorded?0:4);
    if(!s.recorded) fprintf(stderr,"No validated GSP messages recorded; this is not a successful attachment.\n");
    if(s.dropped || s.observed_sequence_gaps) fprintf(stderr,"Observed losses: inspect stats before interpreting samples.\n");
done:
    if(rc==1) fprintf(stderr,"Capture failed or output truncated; Win32 %lu\n",GetLastError());
    if(meta) fclose(meta);
    if(fclose(f)) { perror("close trace");rc=1; }
    free(buffer);CloseHandle(h);return rc;
}
