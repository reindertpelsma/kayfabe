/* SPDX-License-Identifier: MIT
 * Q4 diagnostic sampler. NVML public C API, no device mutations.
 * Counter values are valid only when the adjacent status is zero.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
static void *sym(void *h,const char *s){return (void*)GetProcAddress((HMODULE)h,s);}
static uint64_t now_ms(void){FILETIME f;GetSystemTimePreciseAsFileTime(&f);ULARGE_INTEGER t;t.LowPart=f.dwLowDateTime;t.HighPart=f.dwHighDateTime;return (t.QuadPart-116444736000000000ULL)/10000;}
static void pause_ms(unsigned n){Sleep(n);}
#else
#include <dlfcn.h>
#include <time.h>
static void *sym(void *h,const char *s){return dlsym(h,s);}
static uint64_t now_ms(void){struct timespec t;clock_gettime(CLOCK_REALTIME,&t);return (uint64_t)t.tv_sec*1000+t.tv_nsec/1000000;}
static void pause_ms(unsigned n){struct timespec t={n/1000,(n%1000)*1000000};nanosleep(&t,0);}
#endif
struct meminfo{unsigned long long total,free,used;};
struct utilinfo{unsigned gpu,memory;};
static void *lib,*dev;
static void value(const char *fn,int arg,int has_arg){unsigned v=0;int rc=12;void *f=sym(lib,fn);if(f){if(has_arg)rc=((int(*)(void*,unsigned,unsigned*))f)(dev,arg,&v);else rc=((int(*)(void*,unsigned*))f)(dev,&v);}printf(",%u,%d",v,rc);}
int main(int argc,char **argv){
 unsigned period=argc>1?(unsigned)atoi(argv[1]):200,seconds=argc>2?(unsigned)atoi(argv[2]):1800;
 if(period<100 || period>1000 || seconds>7200 || !seconds)return 64;
#ifdef _WIN32
 lib=(void*)LoadLibraryA("nvml.dll");
 if(!lib)lib=(void*)LoadLibraryA("C:\\Windows\\System32\\nvml.dll");
#else
 lib=dlopen("libnvidia-ml.so.1",RTLD_NOW|RTLD_LOCAL);
#endif
 if(!lib){fprintf(stderr,"NVML library unavailable\n");return 2;}
 int(*init)(void)=(int(*)(void))sym(lib,"nvmlInit_v2");
 int(*get)(unsigned,void**)=(int(*)(unsigned,void**))sym(lib,"nvmlDeviceGetHandleByIndex_v2");
 if(!init||!get)return 2;
 int rc=init();if(rc){fprintf(stderr,"NVML init rc=%d\n",rc);return 2;}
 rc=get(0,&dev);if(rc){fprintf(stderr,"NVML device rc=%d\n",rc);return 2;}
 puts("epoch_ms,util_gpu,util_gpu_rc,util_mem,util_mem_rc,clock_sm_mhz,clock_sm_mhz_rc,clock_mem_mhz,clock_mem_mhz_rc,pstate,pstate_rc,power_mw,power_mw_rc,power_limit_mw,power_limit_mw_rc,temp_c,temp_c_rc,throttle_active,throttle_active_rc,pcie_gen,pcie_gen_rc,pcie_width,pcie_width_rc,pcie_max_gen,pcie_max_gen_rc,pcie_max_width,pcie_max_width_rc,pcie_tx_kb_s,pcie_tx_kb_s_rc,pcie_rx_kb_s,pcie_rx_kb_s_rc,fb_total_bytes,fb_total_bytes_rc,fb_used_bytes,fb_used_bytes_rc,bar1_used_bytes,bar1_used_bytes_rc,enc_util,enc_util_rc,dec_util,dec_util_rc,pcie_replays,pcie_replays_rc,query_end_epoch_ms");
 uint64_t end=now_ms()+(uint64_t)seconds*1000;
 while(now_ms()<end){
  uint64_t begin=now_ms();printf("%llu",(unsigned long long)begin);
  struct utilinfo u={0};int ur=12;void *f=sym(lib,"nvmlDeviceGetUtilizationRates");if(f)ur=((int(*)(void*,struct utilinfo*))f)(dev,&u);printf(",%u,%d,%u,%d",u.gpu,ur,u.memory,ur);
  value("nvmlDeviceGetClockInfo",1,1);value("nvmlDeviceGetClockInfo",2,1);value("nvmlDeviceGetPerformanceState",0,0);
  value("nvmlDeviceGetPowerUsage",0,0);value("nvmlDeviceGetPowerManagementLimit",0,0);value("nvmlDeviceGetTemperature",0,1);
  unsigned long long th=0;int tr=12;f=sym(lib,"nvmlDeviceGetCurrentClocksEventReasons");if(!f)f=sym(lib,"nvmlDeviceGetCurrentClocksThrottleReasons");if(f)tr=((int(*)(void*,unsigned long long*))f)(dev,&th);printf(",%llu,%d",th,tr);
  value("nvmlDeviceGetCurrPcieLinkGeneration",0,0);value("nvmlDeviceGetCurrPcieLinkWidth",0,0);value("nvmlDeviceGetMaxPcieLinkGeneration",0,0);value("nvmlDeviceGetMaxPcieLinkWidth",0,0);
  value("nvmlDeviceGetPcieThroughput",0,1);value("nvmlDeviceGetPcieThroughput",1,1);
  struct meminfo m={0};int mr=12;f=sym(lib,"nvmlDeviceGetMemoryInfo");if(f)mr=((int(*)(void*,struct meminfo*))f)(dev,&m);printf(",%llu,%d,%llu,%d",m.total,mr,m.used,mr);
  memset(&m,0,sizeof m);mr=12;f=sym(lib,"nvmlDeviceGetBAR1MemoryInfo");if(f)mr=((int(*)(void*,struct meminfo*))f)(dev,&m);printf(",%llu,%d",m.used,mr);
  const char *engines[]={"nvmlDeviceGetEncoderUtilization","nvmlDeviceGetDecoderUtilization"};
  for(unsigned i=0;i<2;i++){unsigned v=0,sample=0;int er=12;f=sym(lib,engines[i]);if(f)er=((int(*)(void*,unsigned*,unsigned*))f)(dev,&v,&sample);printf(",%u,%d",v,er);}
  value("nvmlDeviceGetPcieReplayCounter",0,0);uint64_t finish=now_ms();printf(",%llu\n",(unsigned long long)finish);fflush(stdout);
  if(finish-begin<period)pause_ms((unsigned)(period-(finish-begin)));
 }
 int(*shutdown)(void)=(int(*)(void))sym(lib,"nvmlShutdown");if(shutdown)shutdown();return 0;
}
