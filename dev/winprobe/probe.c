// A stand-in Windows "game" for the game manager (Epic 10): a borderless window the size of the screen that clears
// with Direct3D 11 (DXVK under Proton) in a slowly changing color and logs what it sees to `probe.log` in the current
// directory: the D3D adapter, key presses and XInput buttons. Proves that a launch reaches the GPU and that no input
// leaks to it while the shell is in front. Build: dev/winprobe/build.sh
#define COBJMACROS
#include <windows.h>
#include <d3d11.h>
#include <dxgi.h>
#include <xinput.h>
#include <stdio.h>
#include <math.h>

static FILE *logfile;

static void logline(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    fprintf(logfile, "%lu ", (unsigned long)GetTickCount());
    vfprintf(logfile, fmt, ap);
    fputc('\n', logfile);
    fflush(logfile);
    va_end(ap);
}

static LRESULT CALLBACK wndproc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
    case WM_KEYDOWN:
        logline("key %u", (unsigned)wp);
        return 0;
    case WM_SETFOCUS:
        logline("focus in");
        return 0;
    case WM_KILLFOCUS:
        logline("focus out");
        return 0;
    case WM_CLOSE:
        logline("close");
        DestroyWindow(hwnd);
        return 0;
    case WM_DESTROY:
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

int WINAPI WinMain(HINSTANCE inst, HINSTANCE prev, LPSTR cmd, int show) {
    (void)prev;
    (void)cmd;
    (void)show;
    logfile = fopen("probe.log", "a");
    if (!logfile) return 1;
    logline("start");

    WNDCLASSW wc = {0};
    wc.lpfnWndProc = wndproc;
    wc.hInstance = inst;
    wc.hCursor = LoadCursor(NULL, IDC_ARROW);
    wc.lpszClassName = L"winprobe";
    RegisterClassW(&wc);
    int w = GetSystemMetrics(SM_CXSCREEN), h = GetSystemMetrics(SM_CYSCREEN);
    HWND hwnd = CreateWindowExW(0, L"winprobe", L"winprobe", WS_POPUP | WS_VISIBLE, 0, 0, w, h, NULL, NULL, inst, NULL);
    logline("window %dx%d", w, h);

    DXGI_SWAP_CHAIN_DESC sd = {0};
    sd.BufferCount = 2;
    sd.BufferDesc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    sd.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
    sd.OutputWindow = hwnd;
    sd.SampleDesc.Count = 1;
    sd.Windowed = TRUE;
    sd.SwapEffect = DXGI_SWAP_EFFECT_FLIP_DISCARD;
    IDXGISwapChain *swap = NULL;
    ID3D11Device *dev = NULL;
    ID3D11DeviceContext *ctx = NULL;
    HRESULT hr = D3D11CreateDeviceAndSwapChain(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL, 0, NULL, 0, D3D11_SDK_VERSION,
                                               &sd, &swap, &dev, NULL, &ctx);
    if (FAILED(hr)) {
        logline("d3d11 failed 0x%08lx", (unsigned long)hr);
        return 2;
    }
    IDXGIDevice *dxgi = NULL;
    IDXGIAdapter *adapter = NULL;
    DXGI_ADAPTER_DESC ad;
    if (SUCCEEDED(ID3D11Device_QueryInterface(dev, &IID_IDXGIDevice, (void **)&dxgi)) &&
        SUCCEEDED(IDXGIDevice_GetAdapter(dxgi, &adapter)) && SUCCEEDED(IDXGIAdapter_GetDesc(adapter, &ad))) {
        logline("adapter %ls vendor %04x", ad.Description, ad.VendorId);
    }
    ID3D11Texture2D *back = NULL;
    ID3D11RenderTargetView *rtv = NULL;
    IDXGISwapChain_GetBuffer(swap, 0, &IID_ID3D11Texture2D, (void **)&back);
    ID3D11Device_CreateRenderTargetView(dev, (ID3D11Resource *)back, NULL, &rtv);

    WORD buttons[XUSER_MAX_COUNT] = {0};
    unsigned long frames = 0;
    DWORD second = GetTickCount();
    MSG msg;
    for (;;) {
        while (PeekMessageW(&msg, NULL, 0, 0, PM_REMOVE)) {
            if (msg.message == WM_QUIT) {
                logline("quit after %lu frames", frames);
                return 0;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for (DWORD i = 0; i < XUSER_MAX_COUNT; i++) {
            XINPUT_STATE st;
            if (XInputGetState(i, &st) == ERROR_SUCCESS && st.Gamepad.wButtons != buttons[i]) {
                buttons[i] = st.Gamepad.wButtons;
                logline("pad %lu buttons %04x", (unsigned long)i, buttons[i]);
            }
        }
        float t = GetTickCount() / 1000.0f;
        float color[4] = {0.5f + 0.4f * sinf(t), 0.3f, 0.5f + 0.4f * cosf(t * 0.7f), 1.0f};
        ID3D11DeviceContext_OMSetRenderTargets(ctx, 1, &rtv, NULL);
        ID3D11DeviceContext_ClearRenderTargetView(ctx, rtv, color);
        IDXGISwapChain_Present(swap, 1, 0);
        frames++;
        if (GetTickCount() - second >= 5000) {
            logline("frames %lu", frames);
            second = GetTickCount();
        }
    }
}
