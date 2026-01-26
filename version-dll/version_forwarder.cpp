#include "pch.h"
#include <windows.h>
#include <string>
#include <mutex>

// 真实 version.dll 的句柄
static HMODULE g_realVersionDll = nullptr;
static std::once_flag g_loadOnce;

// 函数指针类型定义
typedef BOOL (WINAPI *PFN_GetFileVersionInfoA)(LPCSTR, DWORD, DWORD, LPVOID);
typedef DWORD (WINAPI *PFN_GetFileVersionInfoByHandle)(DWORD, HANDLE, DWORD, LPVOID);
typedef BOOL (WINAPI *PFN_GetFileVersionInfoExA)(DWORD, LPCSTR, DWORD, DWORD, LPVOID);
typedef BOOL (WINAPI *PFN_GetFileVersionInfoExW)(DWORD, LPCWSTR, DWORD, DWORD, LPVOID);
typedef DWORD (WINAPI *PFN_GetFileVersionInfoSizeA)(LPCSTR, LPDWORD);
typedef DWORD (WINAPI *PFN_GetFileVersionInfoSizeExA)(DWORD, LPCSTR, LPDWORD);
typedef DWORD (WINAPI *PFN_GetFileVersionInfoSizeExW)(DWORD, LPCWSTR, LPDWORD);
typedef DWORD (WINAPI *PFN_GetFileVersionInfoSizeW)(LPCWSTR, LPDWORD);
typedef BOOL (WINAPI *PFN_GetFileVersionInfoW)(LPCWSTR, DWORD, DWORD, LPVOID);
typedef DWORD (WINAPI *PFN_VerFindFileA)(DWORD, LPCSTR, LPCSTR, LPCSTR, LPSTR, PUINT, LPSTR, PUINT);
typedef DWORD (WINAPI *PFN_VerFindFileW)(DWORD, LPCWSTR, LPCWSTR, LPCWSTR, LPWSTR, PUINT, LPWSTR, PUINT);
typedef DWORD (WINAPI *PFN_VerInstallFileA)(DWORD, LPCSTR, LPCSTR, LPCSTR, LPCSTR, LPCSTR, LPSTR, PUINT);
typedef DWORD (WINAPI *PFN_VerInstallFileW)(DWORD, LPCWSTR, LPCWSTR, LPCWSTR, LPCWSTR, LPCWSTR, LPWSTR, PUINT);
typedef DWORD (WINAPI *PFN_VerLanguageNameA)(DWORD, LPSTR, DWORD);
typedef DWORD (WINAPI *PFN_VerLanguageNameW)(DWORD, LPWSTR, DWORD);
typedef BOOL (WINAPI *PFN_VerQueryValueA)(LPCVOID, LPCSTR, LPVOID*, PUINT);
typedef BOOL (WINAPI *PFN_VerQueryValueW)(LPCVOID, LPCWSTR, LPVOID*, PUINT);

// 函数指针实例
static PFN_GetFileVersionInfoA pfnGetFileVersionInfoA = nullptr;
static PFN_GetFileVersionInfoByHandle pfnGetFileVersionInfoByHandle = nullptr;
static PFN_GetFileVersionInfoExA pfnGetFileVersionInfoExA = nullptr;
static PFN_GetFileVersionInfoExW pfnGetFileVersionInfoExW = nullptr;
static PFN_GetFileVersionInfoSizeA pfnGetFileVersionInfoSizeA = nullptr;
static PFN_GetFileVersionInfoSizeExA pfnGetFileVersionInfoSizeExA = nullptr;
static PFN_GetFileVersionInfoSizeExW pfnGetFileVersionInfoSizeExW = nullptr;
static PFN_GetFileVersionInfoSizeW pfnGetFileVersionInfoSizeW = nullptr;
static PFN_GetFileVersionInfoW pfnGetFileVersionInfoW = nullptr;
static PFN_VerFindFileA pfnVerFindFileA = nullptr;
static PFN_VerFindFileW pfnVerFindFileW = nullptr;
static PFN_VerInstallFileA pfnVerInstallFileA = nullptr;
static PFN_VerInstallFileW pfnVerInstallFileW = nullptr;
static PFN_VerLanguageNameA pfnVerLanguageNameA = nullptr;
static PFN_VerLanguageNameW pfnVerLanguageNameW = nullptr;
static PFN_VerQueryValueA pfnVerQueryValueA = nullptr;
static PFN_VerQueryValueW pfnVerQueryValueW = nullptr;

// 延迟加载真实的 version.dll
static void LoadRealVersionDll() {
    std::call_once(g_loadOnce, []() {
        wchar_t sysPath[MAX_PATH];
        GetSystemDirectoryW(sysPath, MAX_PATH);
        std::wstring dllPath = std::wstring(sysPath) + L"\\version.dll";
        
        g_realVersionDll = LoadLibraryW(dllPath.c_str());
        if (!g_realVersionDll) return;
        
        // 获取所有函数指针
        pfnGetFileVersionInfoA = (PFN_GetFileVersionInfoA)GetProcAddress(g_realVersionDll, "GetFileVersionInfoA");
        pfnGetFileVersionInfoByHandle = (PFN_GetFileVersionInfoByHandle)GetProcAddress(g_realVersionDll, "GetFileVersionInfoByHandle");
        pfnGetFileVersionInfoExA = (PFN_GetFileVersionInfoExA)GetProcAddress(g_realVersionDll, "GetFileVersionInfoExA");
        pfnGetFileVersionInfoExW = (PFN_GetFileVersionInfoExW)GetProcAddress(g_realVersionDll, "GetFileVersionInfoExW");
        pfnGetFileVersionInfoSizeA = (PFN_GetFileVersionInfoSizeA)GetProcAddress(g_realVersionDll, "GetFileVersionInfoSizeA");
        pfnGetFileVersionInfoSizeExA = (PFN_GetFileVersionInfoSizeExA)GetProcAddress(g_realVersionDll, "GetFileVersionInfoSizeExA");
        pfnGetFileVersionInfoSizeExW = (PFN_GetFileVersionInfoSizeExW)GetProcAddress(g_realVersionDll, "GetFileVersionInfoSizeExW");
        pfnGetFileVersionInfoSizeW = (PFN_GetFileVersionInfoSizeW)GetProcAddress(g_realVersionDll, "GetFileVersionInfoSizeW");
        pfnGetFileVersionInfoW = (PFN_GetFileVersionInfoW)GetProcAddress(g_realVersionDll, "GetFileVersionInfoW");
        pfnVerFindFileA = (PFN_VerFindFileA)GetProcAddress(g_realVersionDll, "VerFindFileA");
        pfnVerFindFileW = (PFN_VerFindFileW)GetProcAddress(g_realVersionDll, "VerFindFileW");
        pfnVerInstallFileA = (PFN_VerInstallFileA)GetProcAddress(g_realVersionDll, "VerInstallFileA");
        pfnVerInstallFileW = (PFN_VerInstallFileW)GetProcAddress(g_realVersionDll, "VerInstallFileW");
        pfnVerLanguageNameA = (PFN_VerLanguageNameA)GetProcAddress(g_realVersionDll, "VerLanguageNameA");
        pfnVerLanguageNameW = (PFN_VerLanguageNameW)GetProcAddress(g_realVersionDll, "VerLanguageNameW");
        pfnVerQueryValueA = (PFN_VerQueryValueA)GetProcAddress(g_realVersionDll, "VerQueryValueA");
        pfnVerQueryValueW = (PFN_VerQueryValueW)GetProcAddress(g_realVersionDll, "VerQueryValueW");
    });
}

void UnloadRealVersionDll() {
    if (g_realVersionDll) {
        FreeLibrary(g_realVersionDll);
        g_realVersionDll = nullptr;
    }
}

// 导出函数实现 - 转发到真实 DLL
extern "C" {

BOOL WINAPI GetFileVersionInfoA(LPCSTR lptstrFilename, DWORD dwHandle, DWORD dwLen, LPVOID lpData) {
    LoadRealVersionDll();
    return pfnGetFileVersionInfoA ? pfnGetFileVersionInfoA(lptstrFilename, dwHandle, dwLen, lpData) : FALSE;
}

DWORD WINAPI GetFileVersionInfoByHandle(DWORD dwFlags, HANDLE hFile, DWORD dwLen, LPVOID lpData) {
    LoadRealVersionDll();
    return pfnGetFileVersionInfoByHandle ? pfnGetFileVersionInfoByHandle(dwFlags, hFile, dwLen, lpData) : 0;
}

BOOL WINAPI GetFileVersionInfoExA(DWORD dwFlags, LPCSTR lpwstrFilename, DWORD dwHandle, DWORD dwLen, LPVOID lpData) {
    LoadRealVersionDll();
    return pfnGetFileVersionInfoExA ? pfnGetFileVersionInfoExA(dwFlags, lpwstrFilename, dwHandle, dwLen, lpData) : FALSE;
}

BOOL WINAPI GetFileVersionInfoExW(DWORD dwFlags, LPCWSTR lpwstrFilename, DWORD dwHandle, DWORD dwLen, LPVOID lpData) {
    LoadRealVersionDll();
    return pfnGetFileVersionInfoExW ? pfnGetFileVersionInfoExW(dwFlags, lpwstrFilename, dwHandle, dwLen, lpData) : FALSE;
}

DWORD WINAPI GetFileVersionInfoSizeA(LPCSTR lptstrFilename, LPDWORD lpdwHandle) {
    LoadRealVersionDll();
    return pfnGetFileVersionInfoSizeA ? pfnGetFileVersionInfoSizeA(lptstrFilename, lpdwHandle) : 0;
}

DWORD WINAPI GetFileVersionInfoSizeExA(DWORD dwFlags, LPCSTR lpwstrFilename, LPDWORD lpdwHandle) {
    LoadRealVersionDll();
    return pfnGetFileVersionInfoSizeExA ? pfnGetFileVersionInfoSizeExA(dwFlags, lpwstrFilename, lpdwHandle) : 0;
}

DWORD WINAPI GetFileVersionInfoSizeExW(DWORD dwFlags, LPCWSTR lpwstrFilename, LPDWORD lpdwHandle) {
    LoadRealVersionDll();
    return pfnGetFileVersionInfoSizeExW ? pfnGetFileVersionInfoSizeExW(dwFlags, lpwstrFilename, lpdwHandle) : 0;
}

DWORD WINAPI GetFileVersionInfoSizeW(LPCWSTR lptstrFilename, LPDWORD lpdwHandle) {
    LoadRealVersionDll();
    return pfnGetFileVersionInfoSizeW ? pfnGetFileVersionInfoSizeW(lptstrFilename, lpdwHandle) : 0;
}

BOOL WINAPI GetFileVersionInfoW(LPCWSTR lptstrFilename, DWORD dwHandle, DWORD dwLen, LPVOID lpData) {
    LoadRealVersionDll();
    return pfnGetFileVersionInfoW ? pfnGetFileVersionInfoW(lptstrFilename, dwHandle, dwLen, lpData) : FALSE;
}

DWORD WINAPI VerFindFileA(DWORD uFlags, LPCSTR szFileName, LPCSTR szWinDir, LPCSTR szAppDir, LPSTR szCurDir, PUINT puCurDirLen, LPSTR szDestDir, PUINT puDestDirLen) {
    LoadRealVersionDll();
    return pfnVerFindFileA ? pfnVerFindFileA(uFlags, szFileName, szWinDir, szAppDir, szCurDir, puCurDirLen, szDestDir, puDestDirLen) : 0;
}

DWORD WINAPI VerFindFileW(DWORD uFlags, LPCWSTR szFileName, LPCWSTR szWinDir, LPCWSTR szAppDir, LPWSTR szCurDir, PUINT puCurDirLen, LPWSTR szDestDir, PUINT puDestDirLen) {
    LoadRealVersionDll();
    return pfnVerFindFileW ? pfnVerFindFileW(uFlags, szFileName, szWinDir, szAppDir, szCurDir, puCurDirLen, szDestDir, puDestDirLen) : 0;
}

DWORD WINAPI VerInstallFileA(DWORD uFlags, LPCSTR szSrcFileName, LPCSTR szDestFileName, LPCSTR szSrcDir, LPCSTR szDestDir, LPCSTR szCurDir, LPSTR szTmpFile, PUINT puTmpFileLen) {
    LoadRealVersionDll();
    return pfnVerInstallFileA ? pfnVerInstallFileA(uFlags, szSrcFileName, szDestFileName, szSrcDir, szDestDir, szCurDir, szTmpFile, puTmpFileLen) : 0;
}

DWORD WINAPI VerInstallFileW(DWORD uFlags, LPCWSTR szSrcFileName, LPCWSTR szDestFileName, LPCWSTR szSrcDir, LPCWSTR szDestDir, LPCWSTR szCurDir, LPWSTR szTmpFile, PUINT puTmpFileLen) {
    LoadRealVersionDll();
    return pfnVerInstallFileW ? pfnVerInstallFileW(uFlags, szSrcFileName, szDestFileName, szSrcDir, szDestDir, szCurDir, szTmpFile, puTmpFileLen) : 0;
}

DWORD WINAPI VerLanguageNameA(DWORD wLang, LPSTR szLang, DWORD cchLang) {
    LoadRealVersionDll();
    return pfnVerLanguageNameA ? pfnVerLanguageNameA(wLang, szLang, cchLang) : 0;
}

DWORD WINAPI VerLanguageNameW(DWORD wLang, LPWSTR szLang, DWORD cchLang) {
    LoadRealVersionDll();
    return pfnVerLanguageNameW ? pfnVerLanguageNameW(wLang, szLang, cchLang) : 0;
}

BOOL WINAPI VerQueryValueA(LPCVOID pBlock, LPCSTR lpSubBlock, LPVOID* lplpBuffer, PUINT puLen) {
    LoadRealVersionDll();
    return pfnVerQueryValueA ? pfnVerQueryValueA(pBlock, lpSubBlock, lplpBuffer, puLen) : FALSE;
}

BOOL WINAPI VerQueryValueW(LPCVOID pBlock, LPCWSTR lpSubBlock, LPVOID* lplpBuffer, PUINT puLen) {
    LoadRealVersionDll();
    return pfnVerQueryValueW ? pfnVerQueryValueW(pBlock, lpSubBlock, lplpBuffer, puLen) : FALSE;
}

} // extern "C"
