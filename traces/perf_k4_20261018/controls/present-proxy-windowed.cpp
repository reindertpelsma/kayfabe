// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// Q4 K4 generic D3D11 present/offscreen proxy, public own app-matrix helpers.
// Usage: present-proxy.exe present|offscreen 3000|300000 seconds
// NVIDIA hardware only. Generated grid, known-pixel readback, real GPU queries.
// No proprietary workload command/shader capture, no software fallback.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <d3d11.h>
#include <d3d12.h>
#include <dxgi1_5.h>
#include <algorithm>
#include <dwmapi.h>
#include <d3dcompiler.h>
#include <stdio.h>
#include <string.h>
#include <string>
#include <vector>

static int g_fail = 0;
static std::string g_why;
static void say(const char *k, const char *fmt, ...) {
  va_list ap; va_start(ap, fmt); printf("KFDX %s ", k); vprintf(fmt, ap); printf("\n"); fflush(stdout); va_end(ap);
}
static bool check(bool ok, const char *what, HRESULT hr = 0) {
  if (ok) { say("ok", "%s", what); return true; }
  say("FAILED", "%s hr=0x%08lx", what, (unsigned long)hr);
  if (!g_fail) { g_fail = 1; g_why = what; }
  return false;
}
template <class T> static void rel(T *&p) { if (p) { p->Release(); p = nullptr; } }

typedef HRESULT(WINAPI *PFN_D3DCompile_t)(LPCVOID, SIZE_T, LPCSTR, const D3D_SHADER_MACRO *, ID3DInclude *, LPCSTR, LPCSTR, UINT, UINT, ID3DBlob **, ID3DBlob **);
static ID3DBlob *compile(const char *src, const char *entry, const char *target) {
  static PFN_D3DCompile_t fn = nullptr;
  if (!fn) {
    HMODULE m = LoadLibraryA("d3dcompiler_47.dll");
    if (m) fn = (PFN_D3DCompile_t)GetProcAddress(m, "D3DCompile");
  }
  if (!fn) { say("compile", "d3dcompiler_47.dll missing"); return nullptr; }
  ID3DBlob *code = nullptr, *err = nullptr;
  HRESULT hr = fn(src, strlen(src), "kf", nullptr, nullptr, entry, target, 0, 0, &code, &err);
  if (FAILED(hr)) { say("compile", "%s %s failed 0x%08lx: %s", entry, target, (unsigned long)hr, err ? (const char *)err->GetBufferPointer() : "?"); }
  rel(err);
  return code;
}

static std::string narrow(const wchar_t *w) {
  char b[256]; WideCharToMultiByte(CP_UTF8, 0, w, -1, b, sizeof b, nullptr, nullptr); return b;
}

// the NVIDIA adapter (or, with --any-vendor, the first non-Basic-Render one)
static IDXGIAdapter1 *pick_adapter(bool any, std::string &name) {
  IDXGIFactory1 *f = nullptr;
  if (!check(SUCCEEDED(CreateDXGIFactory1(__uuidof(IDXGIFactory1), (void **)&f)), "CreateDXGIFactory1")) return nullptr;
  IDXGIAdapter1 *pick = nullptr;
  for (UINT i = 0;; i++) {
    IDXGIAdapter1 *a = nullptr;
    if (f->EnumAdapters1(i, &a) == DXGI_ERROR_NOT_FOUND) break;
    DXGI_ADAPTER_DESC1 d; a->GetDesc1(&d);
    std::string n = narrow(d.Description);
    bool soft = (d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE) || d.VendorId == 0x1414;
    say("adapter", "index=%u vendor=0x%04x device=0x%04x luid=0x%08lx_0x%08lx dedicated_mb=%llu software=%d name=\"%s\"", i, d.VendorId,
        d.DeviceId, (unsigned long)d.AdapterLuid.HighPart, (unsigned long)d.AdapterLuid.LowPart, (unsigned long long)(d.DedicatedVideoMemory >> 20), soft ? 1 : 0, n.c_str());
    if (!pick && !soft && (d.VendorId == 0x10DE || any)) { pick = a; name = n; } else a->Release();
  }
  f->Release();
  if (!pick) check(false, "an NVIDIA hardware adapter exists (WARP/Basic Render excluded)");
  return pick;
}

// Generic grid geometry proxy, not a replay of CATIA/Creo or their OpenGL path.
static const char *VS =
 "cbuffer C : register(b0) { uint cols; uint rows; uint frame; uint pad; };"
 "float4 main(uint id:SV_VertexID):SV_Position {"
 " uint k=id%6;uint c=(k==0?0:k==1?1:k==2?2:k==3?1:k==4?3:2);"
 " uint cell=id/6;float2 p=float2(cell%cols,cell/cols)+float2(c&1,c>>1);"
 " return float4(p.x/cols*2-1,1-p.y/rows*2,0,1); }";
static const char *PS =
 "cbuffer C : register(b0) { uint cols; uint rows; uint frame; uint pad; };"
 "float4 main(float4 p:SV_Position):SV_Target {return float4(.25+(.25*(frame&1)),.5,.75,1);}";
static double seconds(){LARGE_INTEGER n,f;QueryPerformanceCounter(&n);QueryPerformanceFrequency(&f);return double(n.QuadPart)/double(f.QuadPart);}
static unsigned long long epoch_ms(){FILETIME f;GetSystemTimePreciseAsFileTime(&f);ULARGE_INTEGER t;t.LowPart=f.dwLowDateTime;t.HighPart=f.dwHighDateTime;return (t.QuadPart-116444736000000000ULL)/10000;}
static bool drain(ID3D11DeviceContext *ctx,ID3D11Query *event){ctx->End(event);ctx->Flush();double end=seconds()+30;for(;;){BOOL ready=FALSE;HRESULT hr=ctx->GetData(event,&ready,sizeof ready,D3D11_ASYNC_GETDATA_DONOTFLUSH);if(hr==S_OK&&ready)return true;if(FAILED(hr)||seconds()>end)return false;Sleep(0);}}
static bool pixels(ID3D11DeviceContext *ctx,ID3D11Texture2D *rt,ID3D11Texture2D *stg,unsigned frame,bool clear){ctx->CopyResource(stg,rt);D3D11_MAPPED_SUBRESOURCE m={};HRESULT hr=ctx->Map(stg,0,D3D11_MAP_READ,0,&m);if(FAILED(hr))return false;bool ok=true;int expected[]={clear?0:(frame&1?128:64),clear?0:128,clear?0:191,255};const unsigned xy[][2]={{32,32},{960,540},{480,360},{1800,1000}};for(auto &v:xy){auto p=(const unsigned char*)m.pData+v[1]*m.RowPitch+v[0]*4;int bad=0;for(unsigned c=0;c<4;c++)if(abs(int(p[c])-expected[c])>1)bad++;printf("PROXY_PIXEL x=%u y=%u actual=%u,%u,%u,%u expected=%d,%d,%d,%d bad=%d\n",v[0],v[1],p[0],p[1],p[2],p[3],expected[0],expected[1],expected[2],expected[3],bad);ok&=(bad==0);}ctx->Unmap(stg,0);return ok;}
static LRESULT CALLBACK proc(HWND w,UINT m,WPARAM p,LPARAM l){return DefWindowProcW(w,m,p,l);}
int main(int argc,char **argv){
 if(argc!=4 || (strcmp(argv[1],"present")&&strcmp(argv[1],"offscreen"))){fprintf(stderr,"present-proxy present|offscreen 3000|300000 seconds\n");return 64;}
 unsigned occluded=0,other_present=0;
 bool present=!strcmp(argv[1],"present");unsigned vertices=(unsigned)atoi(argv[2]),duration=(unsigned)atoi(argv[3]);
 if((vertices!=3000&&vertices!=300000)||duration<1||duration>30)return 64;
 SetProcessDPIAware();DWORD session=0;ProcessIdToSessionId(GetCurrentProcessId(),&session);printf("PROXY_DISPLAY primary_width=%d primary_height=%d virtual_width=%d virtual_height=%d session=%lu\n",GetSystemMetrics(SM_CXSCREEN),GetSystemMetrics(SM_CYSCREEN),GetSystemMetrics(SM_CXVIRTUALSCREEN),GetSystemMetrics(SM_CYVIRTUALSCREEN),(unsigned long)session);std::string name;auto *ad=pick_adapter(false,name);if(!ad)return 2;
 ID3D11Device *dev=nullptr;ID3D11DeviceContext *ctx=nullptr;D3D_FEATURE_LEVEL got;D3D_FEATURE_LEVEL want[]={D3D_FEATURE_LEVEL_11_0};
 HRESULT hr=D3D11CreateDevice(ad,D3D_DRIVER_TYPE_UNKNOWN,nullptr,0,want,1,D3D11_SDK_VERSION,&dev,&got,&ctx);if(!check(SUCCEEDED(hr),"hardware D3D11",hr))return 2;
 IDXGIAdapter3 *a3=nullptr;ad->QueryInterface(__uuidof(IDXGIAdapter3),(void**)&a3);
 if(a3){DXGI_QUERY_VIDEO_MEMORY_INFO m={};hr=a3->QueryVideoMemoryInfo(0,DXGI_MEMORY_SEGMENT_GROUP_LOCAL,&m);printf("PROXY_BUDGET hr=%ld budget=%llu usage=%llu scope=this_actual_proxy_process\n",(long)hr,(unsigned long long)m.Budget,(unsigned long long)m.CurrentUsage);}
 ID3D11Texture2D *rt=nullptr,*stg=nullptr,*back=nullptr;ID3D11RenderTargetView *view=nullptr;
 D3D11_TEXTURE2D_DESC td={};td.Width=1920;td.Height=1080;td.MipLevels=td.ArraySize=1;td.Format=DXGI_FORMAT_R8G8B8A8_UNORM;td.SampleDesc.Count=1;td.Usage=D3D11_USAGE_DEFAULT;td.BindFlags=D3D11_BIND_RENDER_TARGET;
 if(FAILED(dev->CreateTexture2D(&td,nullptr,&rt))||FAILED(dev->CreateRenderTargetView(rt,nullptr,&view)))return 2;
 td.Usage=D3D11_USAGE_STAGING;td.BindFlags=0;td.CPUAccessFlags=D3D11_CPU_ACCESS_READ;if(FAILED(dev->CreateTexture2D(&td,nullptr,&stg)))return 2;
 ID3DBlob *vb=compile(VS,"main","vs_5_0"),*pb=compile(PS,"main","ps_5_0");if(!vb||!pb)return 2;
 ID3D11VertexShader *vs=nullptr;ID3D11PixelShader *ps=nullptr;if(FAILED(dev->CreateVertexShader(vb->GetBufferPointer(),vb->GetBufferSize(),nullptr,&vs))||FAILED(dev->CreatePixelShader(pb->GetBufferPointer(),pb->GetBufferSize(),nullptr,&ps)))return 2;
 ID3D11Buffer *cb=nullptr;D3D11_BUFFER_DESC bd={};bd.ByteWidth=16;bd.Usage=D3D11_USAGE_DEFAULT;bd.BindFlags=D3D11_BIND_CONSTANT_BUFFER;if(FAILED(dev->CreateBuffer(&bd,nullptr,&cb)))return 2;
 ID3D11RasterizerState *raster=nullptr;D3D11_RASTERIZER_DESC rd={};rd.FillMode=D3D11_FILL_SOLID;rd.CullMode=D3D11_CULL_NONE;rd.DepthClipEnable=TRUE;if(FAILED(dev->CreateRasterizerState(&rd,&raster)))return 2;
 D3D11_VIEWPORT vp={0,0,1920,1080,0,1};ctx->RSSetState(raster);ctx->RSSetViewports(1,&vp);ctx->OMSetRenderTargets(1,&view,nullptr);ctx->IASetInputLayout(nullptr);ctx->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);ctx->VSSetShader(vs,nullptr,0);ctx->PSSetShader(ps,nullptr,0);ctx->VSSetConstantBuffers(0,1,&cb);ctx->PSSetConstantBuffers(0,1,&cb);
 IDXGISwapChain1 *swap=nullptr;HWND window=nullptr;bool tearing=false;
 if(present){WNDCLASSW wc={};wc.lpfnWndProc=proc;wc.hInstance=GetModuleHandleW(nullptr);wc.lpszClassName=L"Q4K4Proxy";RegisterClassW(&wc);RECT wr={0,0,1280,720};AdjustWindowRect(&wr,WS_OVERLAPPEDWINDOW,FALSE);window=CreateWindowExW(WS_EX_TOPMOST,wc.lpszClassName,L"Q4 K4 own windowed proxy",WS_OVERLAPPEDWINDOW|WS_VISIBLE,100,100,wr.right-wr.left,wr.bottom-wr.top,nullptr,nullptr,wc.hInstance,nullptr);if(window){SetWindowPos(window,HWND_TOPMOST,0,0,0,0,SWP_NOMOVE|SWP_NOSIZE|SWP_SHOWWINDOW);SetForegroundWindow(window);RECT c={};GetClientRect(window,&c);DWORD cloak=0;HRESULT ch=DwmGetWindowAttribute(window,DWMWA_CLOAKED,&cloak,sizeof cloak);printf("PROXY_WINDOW client_width=%ld client_height=%ld visible=%d iconic=%d foreground=%d cloak_hr=%ld cloaked=%lu\n",(long)(c.right-c.left),(long)(c.bottom-c.top),IsWindowVisible(window)?1:0,IsIconic(window)?1:0,GetForegroundWindow()==window?1:0,(long)ch,(unsigned long)cloak);}if(!window)return 2;IDXGIFactory2 *f=nullptr;if(FAILED(CreateDXGIFactory1(__uuidof(IDXGIFactory2),(void**)&f)))return 2;IDXGIFactory5 *f5=nullptr;if(SUCCEEDED(f->QueryInterface(__uuidof(IDXGIFactory5),(void**)&f5))){BOOL t=FALSE;hr=f5->CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING,&t,sizeof t);tearing=SUCCEEDED(hr)&&t;f5->Release();}DXGI_SWAP_CHAIN_DESC1 sd={};sd.Width=1920;sd.Height=1080;sd.Format=DXGI_FORMAT_R8G8B8A8_UNORM;sd.SampleDesc.Count=1;sd.BufferUsage=DXGI_USAGE_RENDER_TARGET_OUTPUT;sd.BufferCount=2;sd.SwapEffect=DXGI_SWAP_EFFECT_FLIP_DISCARD;sd.Flags=tearing?DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING:0;hr=f->CreateSwapChainForHwnd(dev,window,&sd,nullptr,nullptr,&swap);f->Release();if(!check(SUCCEEDED(hr),"create flip discard swapchain",hr))return 2;}
 ID3D11Query *event=nullptr,*disjoint=nullptr,*t0=nullptr,*t1=nullptr;D3D11_QUERY_DESC qd={D3D11_QUERY_EVENT,0};dev->CreateQuery(&qd,&event);qd.Query=D3D11_QUERY_TIMESTAMP_DISJOINT;dev->CreateQuery(&qd,&disjoint);qd.Query=D3D11_QUERY_TIMESTAMP;dev->CreateQuery(&qd,&t0);dev->CreateQuery(&qd,&t1);if(!event||!disjoint||!t0||!t1)return 2;
 unsigned params[4]={vertices==3000?25u:250u,vertices==3000?20u:200u,0,0};float black[4]={0,0,0,1};ctx->ClearRenderTargetView(view,black);if(!check(pixels(ctx,rt,stg,0,true),"clear readback control"))return 3;
 ctx->UpdateSubresource(cb,0,nullptr,params,0,0);ctx->Draw(vertices,0);if(!check(pixels(ctx,rt,stg,0,false),"real draw readback control"))return 3;
 const unsigned batch_frames=8,draws=16;unsigned frames=0,batches=0;double gpu_total=0;unsigned disjoint_count=0;unsigned long long start_epoch=epoch_ms();double start=seconds();
 while(seconds()-start<duration){ctx->Begin(disjoint);ctx->End(t0);
  for(unsigned f=0;f<batch_frames;f++){params[2]=frames;ctx->UpdateSubresource(cb,0,nullptr,params,0,0);for(unsigned d=0;d<draws;d++)ctx->Draw(vertices,0);if(present){hr=swap->GetBuffer(0,__uuidof(ID3D11Texture2D),(void**)&back);if(FAILED(hr))return 4;ctx->CopyResource(back,rt);back->Release();back=nullptr;hr=swap->Present(0,tearing?DXGI_PRESENT_ALLOW_TEARING:0);if(hr==DXGI_STATUS_OCCLUDED)occluded++;else if(hr!=S_OK)other_present++;if(FAILED(hr))return 4;}frames++;}
  ctx->End(t1);ctx->End(disjoint);if(!check(drain(ctx,event),"batch GPU completion"))return 4;
  D3D11_QUERY_DATA_TIMESTAMP_DISJOINT dj={};UINT64 a=0,b=0;hr=ctx->GetData(disjoint,&dj,sizeof dj,0);if(hr!=S_OK||ctx->GetData(t0,&a,sizeof a,0)!=S_OK||ctx->GetData(t1,&b,sizeof b,0)!=S_OK)return 4;if(dj.Disjoint)disjoint_count++;else if(dj.Frequency && b>=a)gpu_total+=double(b-a)/double(dj.Frequency);else return 4;batches++;
  if(window){MSG m;while(PeekMessageW(&m,nullptr,0,0,PM_REMOVE)){TranslateMessage(&m);DispatchMessageW(&m);}}
 }
 printf("PROXY_PRESENT_STATUS occluded=%u other_nonzero_status=%u\n",occluded,other_present);
 double elapsed=seconds()-start;bool correct=pixels(ctx,rt,stg,frames-1,false);HRESULT reason=dev->GetDeviceRemovedReason();
 printf("PROXY_RESULT mode=%s vertices=%u draws_per_frame=%u batch_frames=%u frames=%u batches=%u wall_s=%.9f context_timestamp_s=%.9f disjoint_batches=%u fps=%.6f start_epoch_ms=%llu end_epoch_ms=%llu tearing=%d real_pixels=%d device_reason=0x%08lx width=1920 height=1080 no_present_calls=%d\n",argv[1],vertices,draws,batch_frames,frames,batches,elapsed,gpu_total,disjoint_count,double(frames)/elapsed,start_epoch,epoch_ms(),tearing?1:0,correct?1:0,(unsigned long)reason,present?0:1);fflush(stdout);
 if(swap){swap->Release();}
 if(window){DestroyWindow(window);}
 rel(t0);rel(t1);rel(disjoint);rel(event);rel(cb);rel(raster);rel(view);rel(rt);rel(stg);rel(vs);rel(ps);rel(vb);rel(pb);rel(a3);rel(ctx);rel(dev);rel(ad);
 return correct&&SUCCEEDED(reason)?0:5;
}
