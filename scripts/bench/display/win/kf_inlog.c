// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// low-level keyboard+mouse hook logger: logs events that reach the interactive desktop. args: out.txt seconds
#include <windows.h>
#include <stdio.h>
static FILE *f; static DWORD t0;
static LRESULT CALLBACK kb(int c, WPARAM w, LPARAM l){ if(c>=0){KBDLLHOOKSTRUCT*k=(KBDLLHOOKSTRUCT*)l; fprintf(f,"KFIN %lu K vk=0x%02lx sc=0x%02lx %s inj=%d\n",GetTickCount()-t0,k->vkCode,k->scanCode,(w==WM_KEYDOWN||w==WM_SYSKEYDOWN)?"down":"up",(int)(k->flags&LLKHF_INJECTED)); fflush(f);} return CallNextHookEx(0,c,w,l);}
static LRESULT CALLBACK ms(int c, WPARAM w, LPARAM l){ if(c>=0){MSLLHOOKSTRUCT*m=(MSLLHOOKSTRUCT*)l; if(w!=WM_MOUSEMOVE||1){fprintf(f,"KFIN %lu M msg=0x%03x x=%ld y=%ld data=%ld\n",GetTickCount()-t0,(unsigned)w,m->pt.x,m->pt.y,(long)(short)HIWORD(m->mouseData)); fflush(f);}} return CallNextHookEx(0,c,w,l);}
int main(int argc,char**argv){ f=fopen(argv[1],"w"); int secs=atoi(argv[2]); t0=GetTickCount(); fprintf(f,"KFIN START screen=%dx%d\n",GetSystemMetrics(SM_CXSCREEN),GetSystemMetrics(SM_CYSCREEN)); fflush(f);
 SetWindowsHookExA(WH_KEYBOARD_LL,kb,GetModuleHandleA(0),0); SetWindowsHookExA(WH_MOUSE_LL,ms,GetModuleHandleA(0),0);
 DWORD end=GetTickCount()+secs*1000; MSG m; while(GetTickCount()<end){ while(PeekMessageA(&m,0,0,0,PM_REMOVE)){TranslateMessage(&m);DispatchMessageA(&m);} Sleep(5);} fprintf(f,"KFIN END\n"); fclose(f); return 0;}
