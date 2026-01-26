#pragma once
#include <winsock2.h>
#include <ws2tcpip.h>
#include <mswsock.h> 
#include <windows.h>
#include <string>

struct ProxyConfig {
    std::string host;
    int port;
    bool enabled;
    std::string type;  // "http" or "socks5"
    std::string ipv6_mode;  // "proxy", "direct", or "block"
    int timeout_ms;  // 超时时间（毫秒）
};

bool InitializeProxy();
void CleanupProxy();

// Proxy implementation
int ProxyConnect(SOCKET s, const sockaddr* name, int namelen, 
                 int (WSAAPI* originalConnect)(SOCKET, const sockaddr*, int));

// UDP Interceptor
int WSAAPI HookedSendTo(SOCKET s, const char* buf, int len, int flags, const sockaddr* to, int tolen,
                        int (WSAAPI* originalSendTo)(SOCKET, const char*, int, int, const sockaddr*, int));

typedef BOOL (PASCAL *LPFN_MY_CONNECTEX)(SOCKET s, const struct sockaddr* name, int namelen, PVOID lpSendBuffer, DWORD dwSendDataLength, LPDWORD lpdwBytesSent, LPOVERLAPPED lpOverlapped);
extern LPFN_MY_CONNECTEX TrueConnectEx;

ProxyConfig GetProxyConfig();
void Log(const std::string& msg);
