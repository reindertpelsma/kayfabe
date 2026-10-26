/* SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
 * Minimal native hardware D3D11 clear/copy/readback, no window or proprietary assets.
 * Build with MinGW: gcc d3d_minimal.c -ld3d11 -ldxgi -ldxguid -o d3d_minimal.exe
 */
#define COBJMACROS
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <d3d11.h>
#include <dxgi.h>
#include <stdio.h>
#include <string.h>

int main(void)
{
    IDXGIFactory1 *factory = NULL;
    IDXGIAdapter1 *adapter = NULL;
    ID3D11Device *device = NULL;
    ID3D11DeviceContext *context = NULL;
    ID3D11Texture2D *target = NULL, *staging = NULL;
    ID3D11RenderTargetView *view = NULL;
    D3D11_TEXTURE2D_DESC desc;
    HRESULT hr;
    UINT i, frames = 0;
    int rc = 1;
    hr = CreateDXGIFactory1(&IID_IDXGIFactory1, (void **)&factory);
    if (FAILED(hr)) goto done;
    for (i = 0; i < 16; i++) {
        DXGI_ADAPTER_DESC1 ad;
        hr = IDXGIFactory1_EnumAdapters1(factory, i, &adapter);
        if (FAILED(hr)) goto done;
        hr = IDXGIAdapter1_GetDesc1(adapter, &ad);
        if (SUCCEEDED(hr) && ad.VendorId == 0x10de && !(ad.Flags & DXGI_ADAPTER_FLAG_SOFTWARE))
            break;
        IDXGIAdapter1_Release(adapter);
        adapter = NULL;
    }
    if (!adapter) goto done;
    hr = D3D11CreateDevice((IDXGIAdapter *)adapter, D3D_DRIVER_TYPE_UNKNOWN, NULL, 0,
                          NULL, 0, D3D11_SDK_VERSION, &device, NULL, &context);
    if (FAILED(hr)) goto done;
    memset(&desc, 0, sizeof(desc));
    desc.Width = desc.Height = 16;
    desc.MipLevels = desc.ArraySize = 1;
    desc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET;
    hr = ID3D11Device_CreateTexture2D(device, &desc, NULL, &target);
    if (FAILED(hr)) goto done;
    hr = ID3D11Device_CreateRenderTargetView(device, (ID3D11Resource *)target, NULL, &view);
    if (FAILED(hr)) goto done;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    hr = ID3D11Device_CreateTexture2D(device, &desc, NULL, &staging);
    if (FAILED(hr)) goto done;
    for (frames = 0; frames < 60; frames++) {
        const FLOAT color[4] = {0.0f, 0.25f, 1.0f, 1.0f};
        D3D11_MAPPED_SUBRESOURCE mapped;
        const unsigned char *pixel;
        ID3D11DeviceContext_ClearRenderTargetView(context, view, color);
        ID3D11DeviceContext_CopyResource(context, (ID3D11Resource *)staging,
                                        (ID3D11Resource *)target);
        hr = ID3D11DeviceContext_Map(context, (ID3D11Resource *)staging, 0,
                                   D3D11_MAP_READ, 0, &mapped);
        if (FAILED(hr)) goto done;
        pixel = (const unsigned char *)mapped.pData;
        i = pixel[0] == 0 && pixel[1] >= 63 && pixel[1] <= 64 &&
            pixel[2] == 255 && pixel[3] == 255;
        ID3D11DeviceContext_Unmap(context, (ID3D11Resource *)staging, 0);
        if (!i) goto done;
    }
    rc = 0;
done:
    printf("{\"hardware\":%s,\"frames\":%u,\"pass\":%s,\"hresult\":\"0x%08lx\"}\n",
           device ? "true" : "false", (unsigned)frames, rc ? "false" : "true",
           (unsigned long)hr);
    if (view) ID3D11RenderTargetView_Release(view);
    if (staging) ID3D11Texture2D_Release(staging);
    if (target) ID3D11Texture2D_Release(target);
    if (context) ID3D11DeviceContext_Release(context);
    if (device) ID3D11Device_Release(device);
    if (adapter) IDXGIAdapter1_Release(adapter);
    if (factory) IDXGIFactory1_Release(factory);
    return rc;
}
