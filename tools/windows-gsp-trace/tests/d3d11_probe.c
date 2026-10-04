/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Small headless hardware trigger. No swap chain, NVIDIA private IOCTL or hook.
 * Run under an external 60-second process timeout: driver API calls can block.
 */
#define COBJMACROS
#define INITGUID
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <dxgi1_2.h>
#include <d3d11.h>
#include <stdio.h>
#include <string.h>

static int report(const char *step,HRESULT hr) {
    printf("{\"step\":\"%s\",\"hresult\":\"0x%08lx\"}\n",step,(unsigned long)hr);
    return fflush(stdout)==0 && SUCCEEDED(hr);
}
int main(int argc,char **argv) {
    IDXGIFactory1 *factory=NULL; IDXGIAdapter1 *adapter=NULL,*selected=NULL;
    ID3D11Device *device=NULL; ID3D11DeviceContext *context=NULL;
    ID3D11Texture2D *texture=NULL,*staging=NULL;
    ID3D11RenderTargetView *view=NULL; ID3D11Query *query=NULL;
    DXGI_ADAPTER_DESC1 desc; D3D11_TEXTURE2D_DESC td;
    D3D11_QUERY_DESC qd={D3D11_QUERY_EVENT,0}; D3D11_MAPPED_SUBRESOURCE mapped;
    D3D_FEATURE_LEVEL levels[]={D3D_FEATURE_LEVEL_11_1,D3D_FEATURE_LEVEL_11_0},actual;
    const FLOAT color[4]={0.25f,0.5f,0.75f,1.0f};
    const unsigned char expected[4]={64,128,191,255};
    HRESULT hr; UINT i,count=0,round; int result=1,list=0;
    if(argc==2 && strcmp(argv[1],"--list")==0) list=1;
    else if(argc!=1) { fprintf(stderr,"Usage: d3d11_probe.exe [--list]\n");return 2; }
    hr=CreateDXGIFactory1(&IID_IDXGIFactory1,(void **)&factory);
    if(!report("factory",hr)) goto done;
    for(i=0;;i++) {
        hr=IDXGIFactory1_EnumAdapters1(factory,i,&adapter);
        if(hr==DXGI_ERROR_NOT_FOUND) break;
        if(!report("enumerate_adapter",hr)) goto done;
        hr=IDXGIAdapter1_GetDesc1(adapter,&desc);
        if(!report("adapter_description",hr)) goto done;
        printf("{\"adapter_index\":%u,\"vendor\":\"0x%04x\",\"device\":\"0x%04x\","
               "\"subsystem\":\"0x%08x\",\"revision\":%u,\"flags\":%u,"
               "\"luid_high\":%ld,\"luid_low\":%lu}\n",i,desc.VendorId,desc.DeviceId,
               desc.SubSysId,desc.Revision,desc.Flags,(long)desc.AdapterLuid.HighPart,
               (unsigned long)desc.AdapterLuid.LowPart);
        if(fflush(stdout)!=0) goto done;
        if(desc.VendorId==0x10de && !(desc.Flags&DXGI_ADAPTER_FLAG_SOFTWARE)) {
            count++;
            if(!selected) { selected=adapter;adapter=NULL; }
        }
        if(adapter) { IDXGIAdapter1_Release(adapter);adapter=NULL; }
    }
    if(list) { result=0;goto done; }
    if(count!=1) { fprintf(stderr,"REFUSED: expected exactly one NVIDIA hardware adapter; found %u\n",count);goto done; }
    /* An explicit DXGI adapter requires UNKNOWN, not HARDWARE. No WARP fallback. */
    hr=D3D11CreateDevice((IDXGIAdapter *)selected,D3D_DRIVER_TYPE_UNKNOWN,NULL,0,
                         levels,2,D3D11_SDK_VERSION,&device,&actual,&context);
    if(!report("create_nvidia_device_and_context",hr)) goto done;
    printf("{\"feature_level\":\"0x%x\"}\n",(unsigned)actual);
    ZeroMemory(&td,sizeof(td));td.Width=256;td.Height=256;td.MipLevels=1;
    td.ArraySize=1;td.Format=DXGI_FORMAT_R8G8B8A8_UNORM;td.SampleDesc.Count=1;
    td.Usage=D3D11_USAGE_DEFAULT;td.BindFlags=D3D11_BIND_RENDER_TARGET;
    hr=ID3D11Device_CreateTexture2D(device,&td,NULL,&texture);
    if(!report("create_gpu_texture",hr)) goto done;
    hr=ID3D11Device_CreateRenderTargetView(device,(ID3D11Resource *)texture,NULL,&view);
    if(!report("create_render_target",hr)) goto done;
    td.Usage=D3D11_USAGE_STAGING;td.BindFlags=0;td.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
    hr=ID3D11Device_CreateTexture2D(device,&td,NULL,&staging);
    if(!report("create_readback_texture",hr)) goto done;
    hr=ID3D11Device_CreateQuery(device,&qd,&query);
    if(!report("create_completion_query",hr)) goto done;
    for(round=0;round<4;round++) {
        ULONGLONG deadline; BOOL complete=FALSE; unsigned char *pixel; UINT component;
        ID3D11DeviceContext_ClearRenderTargetView(context,view,color);
        ID3D11DeviceContext_CopyResource(context,(ID3D11Resource *)staging,(ID3D11Resource *)texture);
        ID3D11DeviceContext_End(context,(ID3D11Asynchronous *)query);
        ID3D11DeviceContext_Flush(context);deadline=GetTickCount64()+10000;
        do {
            hr=ID3D11DeviceContext_GetData(context,(ID3D11Asynchronous *)query,&complete,sizeof(complete),D3D11_ASYNC_GETDATA_DONOTFLUSH);
            if(hr!=S_FALSE && !(hr==S_OK && !complete)) break;
            if(GetTickCount64()>=deadline) { fprintf(stderr,"REFUSED: GPU event exceeded ten seconds\n");goto done; }
            Sleep(1);
        } while(1);
        if(!report("gpu_event_complete",hr) || hr!=S_OK || !complete) goto done;
        hr=ID3D11DeviceContext_Map(context,(ID3D11Resource *)staging,0,D3D11_MAP_READ,0,&mapped);
        if(!report("map_completed_readback",hr)) goto done;
        pixel=(unsigned char *)mapped.pData;
        for(component=0;component<4;component++) {
            int difference=(int)pixel[component]-(int)expected[component];
            if(difference < -1 || difference > 1) break;
        }
        printf("{\"round\":%u,\"pixel_rgba\":[%u,%u,%u,%u],\"verified\":%s}\n",
               round,pixel[0],pixel[1],pixel[2],pixel[3],component==4?"true":"false");
        ID3D11DeviceContext_Unmap(context,(ID3D11Resource *)staging,0);
        if(component!=4 || fflush(stdout)!=0) goto done;
    }
    hr=ID3D11Device_GetDeviceRemovedReason(device);
    if(!report("device_removed_reason",hr) || hr!=S_OK) goto done;
    result=0;
done:
    if(context) { ID3D11DeviceContext_ClearState(context);ID3D11DeviceContext_Flush(context); }
    if(query) ID3D11Query_Release(query);
    if(view) ID3D11RenderTargetView_Release(view);
    if(staging) ID3D11Texture2D_Release(staging);
    if(texture) ID3D11Texture2D_Release(texture);
    if(context) ID3D11DeviceContext_Release(context);
    if(device) ID3D11Device_Release(device);
    if(adapter) IDXGIAdapter1_Release(adapter);
    if(selected) IDXGIAdapter1_Release(selected);
    if(factory) IDXGIFactory1_Release(factory);
    if(fflush(stdout)!=0 || ferror(stdout)) result=1;
    return result;
}
