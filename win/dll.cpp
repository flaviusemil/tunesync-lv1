// dll.cpp - TuneRemote_Control.dll: the 18 WMSD exports LV1 resolves by name in every
// Modules\*Control.dll (interface version 6, docs: Headlink LV1-SURFACE-DRIVER.md).
//
// Every export returns at once. LV1 calls them on its own threads, the UI thread among
// them, so nothing here waits on MIDI, the network or the window: Create starts the
// App's threads, ShowConfigWindow posts to the window thread, and the broadcasts LV1
// sends every surface (controls, params, assignments, messages) are ignored without
// being read.
//
// Before acting, Create checks what LV1 handed over: the callback table must be
// MixerCoreSG's own exported wrappers, as the probes verified. On a mismatch (a future
// LV1 that changed the ABI) Tune Remote stays passive and says why in its log.
#ifndef WIN32_LEAN_AND_MEAN
#  define WIN32_LEAN_AND_MEAN
#endif
#include <winsock2.h>
#include <windows.h>
#include <string.h>
#include <string>
#include "app.h"
#include "lv1_mixer.h"
#include "version.h"
#include "wmsd_abi.h"

using namespace wmsd;

#define EXPORT extern "C" __declspec(dllexport)

namespace {

struct Controller {
    trw::App* app = nullptr;   // nullptr: this controller is passive
};

// A line in the log file even when no App runs (a passive controller, a refused ABI).
void early_log(const std::string& line) {
    wchar_t pd[MAX_PATH] = L"C:\\ProgramData";
    GetEnvironmentVariableW(L"ProgramData", pd, MAX_PATH);
    std::wstring dir = std::wstring(pd) + L"\\Tune Remote";
    CreateDirectoryW(dir.c_str(), nullptr);
    HANDLE h = CreateFileW((dir + L"\\tune-remote.log").c_str(), FILE_APPEND_DATA,
                           FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_ALWAYS,
                           FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h == INVALID_HANDLE_VALUE) return;
    SYSTEMTIME t;
    GetLocalTime(&t);
    char head[48];
    _snprintf_s(head, sizeof head, _TRUNCATE, "%04d-%02d-%02d %02d:%02d:%02d.%03d [!!] ", t.wYear, t.wMonth, t.wDay,
                t.wHour, t.wMinute, t.wSecond, t.wMilliseconds);
    const std::string s = head + line + "\r\n";
    DWORD w = 0;
    WriteFile(h, s.data(), (DWORD)s.size(), &w, nullptr);
    CloseHandle(h);
}

// The table LV1 passed must be MixerCoreSG's own exported wrappers.
bool verify_callbacks(const CoreCallbacks& cb, std::string* why) {
    HMODULE m = GetModuleHandleW(L"MixerCoreSG.dll");
    if (!m) { *why = "MixerCoreSG.dll is not loaded in this process"; return false; }
    static const char* const names[7] = {"SetMixerControlWrapper",     "GetMixerControlWrapper",
                                         "SetMixerAssignmentsWrapper", "GetMixerAssignmentsWrapper",
                                         "SetMixerParamWrapper",       "GetMixerParamWrapper",
                                         "SendMixerCommandWrapper"};
    void* const* t = reinterpret_cast<void* const*>(&cb);
    for (int i = 0; i < 7; ++i) {
        void* want = reinterpret_cast<void*>(GetProcAddress(m, names[i]));
        if (!want || t[i] != want) { *why = std::string("callback ") + names[i] + " does not match MixerCoreSG's export"; return false; }
    }
    return true;
}

}  // namespace

EXPORT int WMSD_QueryInterfaceVersion() { return kInterfaceVersion; }

EXPORT int WMSD_GetAvailableControllerInfo(ControllerInfo* info) {
    if (!info) return -1;
    strcpy_s(info->name, TR_PRODUCT);
    strcpy_s(info->category, "Surface");
    strcpy_s(info->type, "TuneRemote");
    info->flag200 = 1;
    return 0;
}

EXPORT int WMSD_TestControllerForType(void*, void*, void*) { return 0; }

EXPORT void* WMSD_CreateControllerForType(const char* /*app*/, void* core, const CoreCallbacks* cb, void* /*engine*/,
                                          const void* /*config*/, bool* /*out*/) {
    Controller* c = new Controller;
    CoreCallbacks table{};
    std::string why;
    if (!core || !cb || !trw::safe_copy(&table, cb, sizeof table)) {
        early_log("LV1 passed no surface callbacks: Tune Remote stays passive");
        return c;
    }
    if (!verify_callbacks(table, &why)) {
        early_log("LV1's surface interface is not the one Tune Remote knows (" + why + "): it stays passive");
        return c;
    }
    c->app = trw::App::create(core, table, &why);
    if (!c->app) early_log(why);
    return c;
}

EXPORT int WMSD_InitControllerForType(void*) { return 0; }
EXPORT int WMSD_GetTypeForController(void*, char*) { return 0; }

EXPORT int WMSD_DestroyController(void* ctrl, bool) {
    Controller* c = static_cast<Controller*>(ctrl);
    if (c && c->app) c->app->stop();
    // c and the App are left allocated on purpose: LV1 may still deliver a late callback.
    return 0;
}

EXPORT int WMSD_ShowConfigWindow(void* ctrl) {
    Controller* c = static_cast<Controller*>(ctrl);
    if (c && c->app) c->app->show_window();
    return 0;
}

EXPORT int WMSD_LockConfigWindow(void*) { return 0; }
EXPORT int WMSD_UnlockConfigWindow(void*) { return 0; }
EXPORT int WMSD_IdentifyController(void*, bool) { return 0; }

// LV1 keeps a preset per surface in its preferences, but writes that file only when it
// closes cleanly, so the settings live in settings.ini; the preset is a version marker.
EXPORT int WMSD_GetPreset(void*, void* buf, unsigned size, unsigned* ioSize, char* ver, unsigned verLen) {
    static const char kPreset[8] = {'T', 'R', '0', '1', 0, 0, 0, 0};
    if (ioSize) *ioSize = sizeof kPreset;
    if (buf && size >= sizeof kPreset) memcpy(buf, kPreset, sizeof kPreset);
    if (ver && verLen > 4) strcpy_s(ver, verLen, "1.0");
    return 0;
}

EXPORT int WMSD_SetPreset(void*, const void*, unsigned, const char*) { return 0; }
EXPORT int WMSD_CommandStatusUpdate(void*, void*) { return 0; }
EXPORT int WMSD_MixerParamWasChanged(void*, const void*) { return 0; }      // meters ~23/s: not ours
EXPORT int WMSD_MixerAssignmentsChanged(void*, const void*) { return 0; }
EXPORT int WMSD_ControllerDisplayUpdate(void*, const void*) { return 0; }   // every control LV1 broadcasts
EXPORT int WMSD_MixerMessageSent(void*, const void*) { return 0; }

BOOL WINAPI DllMain(HINSTANCE, DWORD, LPVOID) { return TRUE; }
