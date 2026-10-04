/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Elevated disposable-target smoke/negative checks. Stops capture at completion.
 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdio.h>
#include "../trace_protocol.h"
static unsigned failures;
#define CHECK(x) do { if(!(x)) { fprintf(stderr,"FAIL line %u: %s (Win32 %lu)\n",(unsigned)__LINE__,#x,GetLastError());failures++; } } while(0)
static HANDLE open_device(const wchar_t *name) { return CreateFileW(name,GENERIC_READ,0,NULL,OPEN_EXISTING,0,NULL); }
int main(void) {
    HANDLE h,other,token=NULL,restricted=NULL; DWORD bytes=0; KFGT_STATS first,last;
    BYTE world[SECURITY_MAX_SID_SIZE];DWORD world_size=sizeof(world);SID_AND_ATTRIBUTES restrict_sid;
    BYTE input=0,small[4];BOOL ok;
    h=open_device(L"\\\\.\\KayfabeGspTrace");
    if(h==INVALID_HANDLE_VALUE) { fprintf(stderr,"Driver is not running or caller is not elevated: %lu\n",GetLastError());return 1; }
    CHECK(DeviceIoControl(h,KFGT_IOCTL_STATS,NULL,0,&first,sizeof(first),&bytes,NULL) && bytes==sizeof(first) && first.version==KFGT_ABI);
    CHECK(!DeviceIoControl(h,KFGT_IOCTL_STATS,&input,1,&last,sizeof(last),&bytes,NULL));
    CHECK(!DeviceIoControl(h,KFGT_IOCTL_STATS,NULL,0,small,sizeof(small),&bytes,NULL));
    CHECK(!DeviceIoControl(h,KFGT_IOCTL_READ,NULL,0,small,sizeof(small),&bytes,NULL));
    CHECK(!DeviceIoControl(h,0x0022600cu,NULL,0,&last,sizeof(last),&bytes,NULL));
    other=open_device(L"\\\\.\\KayfabeGspTrace");CHECK(other==INVALID_HANDLE_VALUE);
    if(other!=INVALID_HANDLE_VALUE) CloseHandle(other);
    Sleep(500);
    CHECK(DeviceIoControl(h,KFGT_IOCTL_STATS,NULL,0,&last,sizeof(last),&bytes,NULL));
    CHECK(last.scanned_bytes>0);
    CloseHandle(h);
    other=open_device(L"\\\\.\\KayfabeGspTrace\\child");CHECK(other==INVALID_HANDLE_VALUE);
    if(other!=INVALID_HANDLE_VALUE) CloseHandle(other);
    /* A restricting World SID requires a grant for World in addition to the
       caller's normal token. Device grants only SYSTEM/Administrators. */
    CHECK(CreateWellKnownSid(WinWorldSid,NULL,world,&world_size));
    restrict_sid.Sid=world;restrict_sid.Attributes=0;
    ok=OpenProcessToken(GetCurrentProcess(),TOKEN_DUPLICATE|TOKEN_QUERY|TOKEN_IMPERSONATE,&token);
    CHECK(ok);
    if(ok) {
        ok=CreateRestrictedToken(token,DISABLE_MAX_PRIVILEGE,0,NULL,0,NULL,1,&restrict_sid,&restricted);CHECK(ok);
        if(ok) {
            ok=ImpersonateLoggedOnUser(restricted);CHECK(ok);
            if(ok) {
                other=open_device(L"\\\\.\\KayfabeGspTrace");
                CHECK(other==INVALID_HANDLE_VALUE && GetLastError()==ERROR_ACCESS_DENIED);
                if(other!=INVALID_HANDLE_VALUE) CloseHandle(other);
                CHECK(RevertToSelf());
            }
            CloseHandle(restricted);
        }
        CloseHandle(token);
    }
    h=open_device(L"\\\\.\\KayfabeGspTrace");CHECK(h!=INVALID_HANDLE_VALUE);
    if(h!=INVALID_HANDLE_VALUE) {
        CHECK(DeviceIoControl(h,KFGT_IOCTL_STOP,NULL,0,NULL,0,&bytes,NULL));
        CHECK(DeviceIoControl(h,KFGT_IOCTL_STATS,NULL,0,&first,sizeof(first),&bytes,NULL));
        Sleep(100);
        CHECK(DeviceIoControl(h,KFGT_IOCTL_STATS,NULL,0,&last,sizeof(last),&bytes,NULL));
        CHECK(first.scanned_bytes==last.scanned_bytes);
        CloseHandle(h);
    }
    printf("Windows API tests: %u failure(s). Recorder stopped; restart service before capture.\n",failures);
    return failures?1:0;
}
