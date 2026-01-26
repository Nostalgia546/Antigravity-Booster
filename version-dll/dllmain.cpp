#include "pch.h"
#include "proxy.h"
#include "fakeip.h"
#include <detours.h>

#pragma comment(lib, "ws2_32.lib")
#pragma comment(lib, "winhttp.lib")

// 使用 version.def 文件和 version_forwarder.cpp 来处理导出
// 不再使用链接器转发，避免硬编码路径问题

// 转发器清理函数声明
extern void UnloadRealVersionDll();

typedef int (WSAAPI* LPFN_CONNECT)(SOCKET s, const sockaddr* name, int namelen);
typedef int (WSAAPI* LPFN_WSACONNECT)(SOCKET s, const sockaddr* name, int namelen, LPWSABUF lpCallerData, LPWSABUF lpCalleeData, LPQOS lpSQOS, LPQOS lpGQOS);
typedef int (WSAAPI* LPFN_WSAIOCTL)(SOCKET s, DWORD dwIoControlCode, LPVOID lpvInBuffer, DWORD cbInBuffer, LPVOID lpvOutBuffer, DWORD cbOutBuffer, LPDWORD lpcbBytesReturned, LPWSAOVERLAPPED lpOverlapped, LPWSAOVERLAPPED_COMPLETION_ROUTINE lpCompletionRoutine);
typedef int (WSAAPI* LPFN_SENDTO)(SOCKET s, const char* buf, int len, int flags, const sockaddr* to, int tolen);
typedef int (WSAAPI* LPFN_GETADDRINFO)(PCSTR pNodeName, PCSTR pServiceName, const ADDRINFOA* pHints, PADDRINFOA* ppResult);
typedef int (WSAAPI* LPFN_GETADDRINFOW)(PCWSTR pNodeName, PCWSTR pServiceName, const ADDRINFOW* pHints, PADDRINFOW* ppResult);
typedef BOOL (WINAPI* LPFN_CREATEPROCESSW)(LPCWSTR, LPWSTR, LPSECURITY_ATTRIBUTES, LPSECURITY_ATTRIBUTES, BOOL, DWORD, LPVOID, LPCWSTR, LPSTARTUPINFOW, LPPROCESS_INFORMATION);

static LPFN_CONNECT TrueConnect = NULL;
static LPFN_WSACONNECT TrueWSAConnect = NULL;
static LPFN_WSAIOCTL TrueWSAIoctl = NULL;
static LPFN_SENDTO TrueSendTo = NULL;
static LPFN_GETADDRINFO TrueGetAddrInfo = NULL;
static LPFN_GETADDRINFOW TrueGetAddrInfoW = NULL;
static LPFN_CREATEPROCESSW TrueCreateProcessW = NULL;

BOOL PASCAL HookedConnectEx(SOCKET s, const struct sockaddr* name, int namelen, PVOID lpSendBuffer, DWORD dwSendDataLength, LPDWORD lpdwBytesSent, LPOVERLAPPED lpOverlapped) {
    // Check if this is localhost - if so, use original ConnectEx
    if (name && name->sa_family == AF_INET) {
        sockaddr_in* addr = (sockaddr_in*)name;
        unsigned long ip = ntohl(addr->sin_addr.s_addr);
        if ((ip & 0xFF000000) == 0x7F000000) { // 127.x.x.x
            if (TrueConnectEx) {
                return TrueConnectEx(s, name, namelen, lpSendBuffer, dwSendDataLength, lpdwBytesSent, lpOverlapped);
            }
        }
    } else if (name && name->sa_family == AF_INET6) {
        sockaddr_in6* addr = (sockaddr_in6*)name;
        if (IN6_IS_ADDR_LOOPBACK(&addr->sin6_addr)) {
            if (TrueConnectEx) {
                return TrueConnectEx(s, name, namelen, lpSendBuffer, dwSendDataLength, lpdwBytesSent, lpOverlapped);
            }
        }
    }
    
    Log("ConnectEx called (converting to sync proxy)");
    int result = ProxyConnect(s, name, namelen, TrueConnect);
    
    if (result == 0) {
        if (lpOverlapped) {
            lpOverlapped->Internal = 0;
            lpOverlapped->InternalHigh = 0;
            if (lpdwBytesSent) *lpdwBytesSent = 0;
            if (lpOverlapped->hEvent) SetEvent(lpOverlapped->hEvent);
        }
        Log("  ConnectEx -> Success");
        return TRUE;
    }
    
    Log("  ConnectEx -> Failed");
    return FALSE;
}

int WSAAPI MySendTo(SOCKET s, const char* buf, int len, int flags, const sockaddr* to, int tolen) {
    return HookedSendTo(s, buf, len, flags, to, tolen, TrueSendTo);
}

int WSAAPI MyConnect(SOCKET s, const sockaddr* name, int namelen) {
    return ProxyConnect(s, name, namelen, TrueConnect);
}

int WSAAPI MyWSAConnect(SOCKET s, const sockaddr* name, int namelen, LPWSABUF lpCallerData, LPWSABUF lpCalleeData, LPQOS lpSQOS, LPQOS lpGQOS) {
    return ProxyConnect(s, name, namelen, TrueConnect);
}

int WSAAPI MyWSAIoctl(SOCKET s, DWORD dwIoControlCode, LPVOID lpvInBuffer, DWORD cbInBuffer, LPVOID lpvOutBuffer, DWORD cbOutBuffer, LPDWORD lpcbBytesReturned, LPWSAOVERLAPPED lpOverlapped, LPWSAOVERLAPPED_COMPLETION_ROUTINE lpCompletionRoutine) {
    int res = TrueWSAIoctl(s, dwIoControlCode, lpvInBuffer, cbInBuffer, lpvOutBuffer, cbOutBuffer, lpcbBytesReturned, lpOverlapped, lpCompletionRoutine);
    if (dwIoControlCode == SIO_GET_EXTENSION_FUNCTION_POINTER && cbInBuffer >= sizeof(GUID)) {
        GUID connectExGuid = WSAID_CONNECTEX;
        if (memcmp(lpvInBuffer, &connectExGuid, sizeof(GUID)) == 0) {
            if (lpvOutBuffer && cbOutBuffer >= sizeof(PVOID)) {
                Log("WSAIoctl: Redirecting ConnectEx");
                TrueConnectEx = *(LPFN_MY_CONNECTEX*)lpvOutBuffer; 
                *(LPFN_MY_CONNECTEX*)lpvOutBuffer = HookedConnectEx;
            }
        }
    }
    return res;
}

// Hook getaddrinfo to return FakeIP for domains
int WSAAPI MyGetAddrInfo(PCSTR pNodeName, PCSTR pServiceName, const ADDRINFOA* pHints, PADDRINFOA* ppResult) {
    if (!TrueGetAddrInfo) return EAI_FAIL;
    
    // 只对域名进行 FakeIP 处理（跳过 IP 地址和 localhost）
    if (pNodeName && pNodeName[0] != '\0') {
        std::string node = pNodeName;
        
        // 检查是否是 IP 地址或 localhost
        bool isIp = (inet_addr(pNodeName) != INADDR_NONE);
        bool isLocalhost = (node == "localhost" || node.find("127.") == 0);
        
        if (!isIp && !isLocalhost) {
            // 分配虚拟 IP
            uint32_t fakeIp = SimpleFakeIP::Instance().Allocate(node);
            
            // 将虚拟 IP 转换为字符串
            char fakeIpStr[16];
            sprintf_s(fakeIpStr, "%d.%d.%d.%d", 
                (fakeIp >> 24) & 0xFF,
                (fakeIp >> 16) & 0xFF,
                (fakeIp >> 8) & 0xFF,
                fakeIp & 0xFF);
            
            Log("DNS: " + node + " -> " + std::string(fakeIpStr));
            
            // 使用虚拟 IP 调用原始函数
            return TrueGetAddrInfo(fakeIpStr, pServiceName, pHints, ppResult);
        }
    }
    
    return TrueGetAddrInfo(pNodeName, pServiceName, pHints, ppResult);
}

// Hook CreateProcessW to inject DLL into child processes
BOOL WINAPI MyCreateProcessW(
    LPCWSTR lpApplicationName,
    LPWSTR lpCommandLine,
    LPSECURITY_ATTRIBUTES lpProcessAttributes,
    LPSECURITY_ATTRIBUTES lpThreadAttributes,
    BOOL bInheritHandles,
    DWORD dwCreationFlags,
    LPVOID lpEnvironment,
    LPCWSTR lpCurrentDirectory,
    LPSTARTUPINFOW lpStartupInfo,
    LPPROCESS_INFORMATION lpProcessInformation) {
    
    // 获取当前 DLL 的路径
    char dllPath[MAX_PATH];
    HMODULE hModule = NULL;
    GetModuleHandleExA(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                       (LPCSTR)&MyCreateProcessW, &hModule);
    GetModuleFileNameA(hModule, dllPath, MAX_PATH);
    
    Log("CreateProcess: Injecting DLL into child process");
    
    // 使用 Detours 创建进程并注入 DLL
    return DetourCreateProcessWithDllExW(
        lpApplicationName,
        lpCommandLine,
        lpProcessAttributes,
        lpThreadAttributes,
        bInheritHandles,
        dwCreationFlags,
        lpEnvironment,
        lpCurrentDirectory,
        lpStartupInfo,
        lpProcessInformation,
        dllPath,
        TrueCreateProcessW
    );
}

void SetupHooks() {
    HMODULE hWs2 = GetModuleHandleA("ws2_32.dll");
    if (!hWs2) hWs2 = LoadLibraryA("ws2_32.dll");
    if (hWs2) {
        TrueConnect = (LPFN_CONNECT)GetProcAddress(hWs2, "connect");
        TrueWSAConnect = (LPFN_WSACONNECT)GetProcAddress(hWs2, "WSAConnect");
        TrueWSAIoctl = (LPFN_WSAIOCTL)GetProcAddress(hWs2, "WSAIoctl");
        TrueSendTo = (LPFN_SENDTO)GetProcAddress(hWs2, "sendto");
        TrueGetAddrInfo = (LPFN_GETADDRINFO)GetProcAddress(hWs2, "getaddrinfo");
        
        // Hook kernel32 CreateProcessW for child process injection
        HMODULE hKernel32 = GetModuleHandleA("kernel32.dll");
        if (hKernel32) {
            TrueCreateProcessW = (LPFN_CREATEPROCESSW)GetProcAddress(hKernel32, "CreateProcessW");
        }
        
        DetourTransactionBegin();
        DetourUpdateThread(GetCurrentThread());
        if (TrueConnect) DetourAttach(&(PVOID&)TrueConnect, MyConnect);
        if (TrueWSAConnect) DetourAttach(&(PVOID&)TrueWSAConnect, MyWSAConnect);
        if (TrueWSAIoctl) DetourAttach(&(PVOID&)TrueWSAIoctl, MyWSAIoctl);
        if (TrueSendTo) DetourAttach(&(PVOID&)TrueSendTo, MySendTo);
        if (TrueGetAddrInfo) DetourAttach(&(PVOID&)TrueGetAddrInfo, MyGetAddrInfo);
        if (TrueCreateProcessW) DetourAttach(&(PVOID&)TrueCreateProcessW, MyCreateProcessW);
        DetourTransactionCommit();
        Log("Full Hooks (TCP/UDP/Async/DNS/ChildProcess) initialized.");
    }
}

BOOL APIENTRY DllMain(HMODULE hModule, DWORD ul_reason_for_call, LPVOID lpReserved) {
    if (DetourIsHelperProcess()) return TRUE;
    switch (ul_reason_for_call) {
    case DLL_PROCESS_ATTACH:
        DisableThreadLibraryCalls(hModule);
        InitializeProxy();
        SetupHooks();
        break;
    case DLL_PROCESS_DETACH:
        CleanupProxy();
        UnloadRealVersionDll();
        break;
    }
    return TRUE;
}
