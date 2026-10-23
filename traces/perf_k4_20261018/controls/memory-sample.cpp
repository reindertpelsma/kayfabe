// SPDX-License-Identifier: MIT
// Q4 read-only diagnostic, public DXGI/PDH/NVAPI interfaces.
// NVAPI struct/interface IDs: NVIDIA/nvapi@70d337db9186e968eab622f7e786de7e437faf3d.
// NVAPI SDK MIT copyright NVIDIA Corporation, public declarations only.
#define UNICODE
#define _UNICODE
#include <windows.h>
#include <dxgi1_4.h>
#include <pdh.h>
#include <pdhmsg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <vector>
struct NvMem {uint32_t version;uint64_t dedicated,available,system,shared,current,evicted_bytes,evicted_count,promoted_bytes,promoted_count;};
static_assert(sizeof(NvMem)==80,"NVAPI public V1 struct alignment");
static uint64_t epoch(){FILETIME f;GetSystemTimePreciseAsFileTime(&f);ULARGE_INTEGER t;t.LowPart=f.dwLowDateTime;t.HighPart=f.dwHighDateTime;return (t.QuadPart-116444736000000000ULL)/10000;}
static void pdh_rows(PDH_HCOUNTER c,const char *label,uint64_t t){DWORD n=0,k=0;PDH_STATUS r=PdhGetFormattedCounterArrayW(c,PDH_FMT_DOUBLE,&n,&k,nullptr);if(r!=(PDH_STATUS)PDH_MORE_DATA){printf("{\"epoch_ms\":%llu,\"kind\":\"pdh_status\",\"counter\":\"%s\",\"status\":%lu}\n",(unsigned long long)t,label,(unsigned long)r);return;}std::vector<unsigned char>b(n);auto *a=(PDH_FMT_COUNTERVALUE_ITEM_W*)b.data();r=PdhGetFormattedCounterArrayW(c,PDH_FMT_DOUBLE,&n,&k,a);if(r!=ERROR_SUCCESS)return;printf("{\"epoch_ms\":%llu,\"kind\":\"pdh_batch\",\"counter\":\"%s\",\"instances\":%lu,\"status\":0}\n",(unsigned long long)t,label,(unsigned long)k);for(DWORD i=0;i<k;i++){if(a[i].FmtValue.CStatus<=1 && a[i].FmtValue.doubleValue==0)continue;char name[512]={};WideCharToMultiByte(CP_UTF8,0,a[i].szName,-1,name,sizeof(name),nullptr,nullptr);printf("{\"epoch_ms\":%llu,\"kind\":\"pdh\",\"counter\":\"%s\",\"instance\":\"%s\",\"value\":%.4f,\"status\":%lu}\n",(unsigned long long)t,label,name,a[i].FmtValue.doubleValue,(unsigned long)a[i].FmtValue.CStatus);}}
int main(int argc,char **argv){unsigned seconds=argc>1?(unsigned)atoi(argv[1]):1800;if(seconds>7200||seconds==0)return 64;
 IDXGIFactory1 *fac=nullptr;HRESULT hr=CreateDXGIFactory1(__uuidof(IDXGIFactory1),(void**)&fac);if(FAILED(hr))return 2;
 IDXGIAdapter3 *ad=nullptr;
 for(UINT i=0;i<8;i++){IDXGIAdapter1 *a=nullptr;if(fac->EnumAdapters1(i,&a)!=S_OK)break;DXGI_ADAPTER_DESC1 d={};a->GetDesc1(&d);if(d.VendorId==0x10de){a->QueryInterface(__uuidof(IDXGIAdapter3),(void**)&ad);printf("{\"kind\":\"adapter\",\"dedicated_bytes\":%llu,\"shared_bytes\":%llu,\"luid_low\":%lu,\"luid_high\":%ld,\"scope\":\"DXGI budget belongs to this observer process, not viewperf\"}\n",(unsigned long long)d.DedicatedVideoMemory,(unsigned long long)d.SharedSystemMemory,(unsigned long)d.AdapterLuid.LowPart,(long)d.AdapterLuid.HighPart);}a->Release();if(ad)break;}
 HMODULE nv=LoadLibraryW(L"nvapi64.dll");using QI=void*(__cdecl*)(unsigned);QI qi=nv?(QI)GetProcAddress(nv,"nvapi_QueryInterface"):nullptr;
 using Init=int(__cdecl*)(void);using Enum=int(__cdecl*)(void**,unsigned*);using Get=int(__cdecl*)(void*,NvMem*);Get get=nullptr;void *handles[64]={};unsigned count=0;int nr=-1;
 if(qi){auto init=(Init)qi(0x0150e828);auto en=(Enum)qi(0xe5ac921f);get=(Get)qi(0xc0599498);if(init&&en&&get){nr=init();if(!nr)nr=en(handles,&count);}}
 printf("{\"kind\":\"nvapi_support\",\"status\":%d,\"gpu_count\":%u,\"get_memory_ex\":%s}\n",nr,count,get?"true":"false");
 PDH_HQUERY query=nullptr;PdhOpenQueryW(nullptr,0,&query);
 struct Counter{const wchar_t *path;const char *label;PDH_HCOUNTER h;};
 Counter cs[]={
 {L"\\GPU Adapter Memory(*)\\Dedicated Usage","adapter_dedicated",nullptr},
 {L"\\GPU Adapter Memory(*)\\Shared Usage","adapter_shared",nullptr},
 {L"\\GPU Adapter Memory(*)\\Total Committed","adapter_committed",nullptr},
 {L"\\GPU Process Memory(*)\\Dedicated Usage","process_dedicated",nullptr},
 {L"\\GPU Process Memory(*)\\Shared Usage","process_shared",nullptr},
 {L"\\GPU Engine(*)\\Utilization Percentage","engine_util",nullptr}};
 for(auto &c:cs){auto r=PdhAddEnglishCounterW(query,c.path,0,&c.h);printf("{\"kind\":\"counter_support\",\"counter\":\"%s\",\"status\":%lu}\n",c.label,(unsigned long)r);}
 PdhCollectQueryData(query);uint64_t end=epoch()+(uint64_t)seconds*1000;unsigned tick=0;
 while(epoch()<end){uint64_t begin=epoch();
  if(ad)for(unsigned group=0;group<2;group++){DXGI_QUERY_VIDEO_MEMORY_INFO m={};hr=ad->QueryVideoMemoryInfo(0,(DXGI_MEMORY_SEGMENT_GROUP)group,&m);printf("{\"epoch_ms\":%llu,\"kind\":\"dxgi_budget\",\"group\":%u,\"hr\":%ld,\"budget\":%llu,\"observer_usage\":%llu,\"available_reservation\":%llu,\"reservation\":%llu}\n",(unsigned long long)begin,group,(long)hr,(unsigned long long)m.Budget,(unsigned long long)m.CurrentUsage,(unsigned long long)m.AvailableForReservation,(unsigned long long)m.CurrentReservation);}
  if(tick%10==0 && !nr && count && get){NvMem m={};m.version=sizeof(m)|(1u<<16);int rc=get(handles[0],&m);printf("{\"epoch_ms\":%llu,\"kind\":\"nvapi_memory\",\"status\":%d,\"dedicated\":%llu,\"available\":%llu,\"shared_allowed\":%llu,\"current_available\":%llu,\"evicted_bytes\":%llu,\"evicted_count\":%llu,\"promoted_bytes\":%llu,\"promoted_count\":%llu}\n",(unsigned long long)begin,rc,(unsigned long long)m.dedicated,(unsigned long long)m.available,(unsigned long long)m.shared,(unsigned long long)m.current,(unsigned long long)m.evicted_bytes,(unsigned long long)m.evicted_count,(unsigned long long)m.promoted_bytes,(unsigned long long)m.promoted_count);}
  PdhCollectQueryData(query);for(auto &c:cs)if(c.h)pdh_rows(c.h,c.label,begin);fflush(stdout);tick++;uint64_t used=epoch()-begin;if(used<1000)Sleep((DWORD)(1000-used));
 }
 PdhCloseQuery(query);if(ad)ad->Release();fac->Release();if(nv)FreeLibrary(nv);return 0;
}
