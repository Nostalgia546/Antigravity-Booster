#include "pch.h"
#include "proxy.h"
#include "fakeip.h"
#include <winhttp.h>
#include <sstream>
#include <vector>
#include <algorithm>

static ProxyConfig g_proxyConfig = { "", 0, false, "http", "proxy", 5000 };
LPFN_MY_CONNECTEX TrueConnectEx = NULL; 

// Logging disabled for performance
void Log(const std::string& msg) {
    // No-op: logging disabled
}

// Global cleaner for proxy strings like "http=127.0.0.1:7890;https=127.0.0.1:7890"
bool RobustParseProxy(const std::wstring& raw, ProxyConfig& config) {
    std::wstring s = raw;
    
    // 1. If multiple proxies, take the first one
    size_t semi = s.find(L';');
    if (semi != std::wstring::npos) s = s.substr(0, semi);
    
    // 2. Remove "http=" or "socks=" labels
    size_t eq = s.find(L'=');
    if (eq != std::wstring::npos) s = s.substr(eq + 1);
    
    // 3. Remove "http://" prefixes
    size_t prot = s.find(L"://");
    if (prot != std::wstring::npos) s = s.substr(prot + 3);

    // 4. Split host and port
    size_t colon = s.rfind(L':');
    if (colon == std::wstring::npos) return false;
    
    std::wstring host = s.substr(0, colon);
    std::wstring port = s.substr(colon + 1);
    
    // Convert to narrow string
    int len = WideCharToMultiByte(CP_UTF8, 0, host.c_str(), -1, NULL, 0, NULL, NULL);
    if (len > 0) {
        std::vector<char> buf(len);
        WideCharToMultiByte(CP_UTF8, 0, host.c_str(), -1, buf.data(), len, NULL, NULL);
        config.host = buf.data();
    }
    config.port = _wtoi(port.c_str());
    config.enabled = !config.host.empty() && config.port > 0;
    return config.enabled;
}

bool InitializeProxy() {
    Log("--- Proxy Engine Reloaded ---");
    WINHTTP_CURRENT_USER_IE_PROXY_CONFIG cfg = { 0 };
    if (WinHttpGetIEProxyConfigForCurrentUser(&cfg)) {
        if (cfg.lpszProxy) {
            if (RobustParseProxy(cfg.lpszProxy, g_proxyConfig)) {
                Log("Active Proxy Config: " + g_proxyConfig.host + ":" + std::to_string(g_proxyConfig.port));
            }
            GlobalFree(cfg.lpszProxy);
        }
    }
    return g_proxyConfig.enabled;
}

void CleanupProxy() { 
    // No cleanup needed
}

ProxyConfig GetProxyConfig() {
    return g_proxyConfig;
}

bool Socks5Handshake(SOCKET s, const std::string& host, int port) {
    // 1. 认证协商
    char greet[] = { 0x05, 0x01, 0x00 };
    Log("  SOCKS5: Sending auth negotiation");
    if (send(s, greet, 3, 0) != 3) {
        Log("  SOCKS5: Failed to send auth");
        return false;
    }
    char resp[2];
    Log("  SOCKS5: Waiting for auth response");
    if (recv(s, resp, 2, 0) != 2 || resp[0] != 0x05) {
        Log("  SOCKS5: Invalid auth response");
        return false;
    }
    
    // 2. 构建 CONNECT 请求
    std::vector<char> req;
    req.push_back(0x05);  // VER
    req.push_back(0x01);  // CMD = CONNECT
    req.push_back(0x00);  // RSV
    
    // 检测地址类型并添加相应的 ATYP 和地址数据
    in_addr addr4;
    in6_addr addr6;
    
    if (inet_pton(AF_INET, host.c_str(), &addr4) == 1) {
        // IPv4 地址: ATYP=0x01, 4字节二进制地址
        Log("  SOCKS5: Using IPv4 address type");
        req.push_back(0x01);  // ATYP = IPv4
        unsigned char* bytes = (unsigned char*)&addr4;
        for (int i = 0; i < 4; i++) {
            req.push_back(bytes[i]);
        }
    } else if (inet_pton(AF_INET6, host.c_str(), &addr6) == 1) {
        // IPv6 地址: ATYP=0x04, 16字节二进制地址
        Log("  SOCKS5: Using IPv6 address type (ATYP=0x04)");
        req.push_back(0x04);  // ATYP = IPv6
        unsigned char* bytes = (unsigned char*)&addr6;
        for (int i = 0; i < 16; i++) {
            req.push_back(bytes[i]);
        }
    } else {
        // 域名: ATYP=0x03, 长度+字符串
        Log("  SOCKS5: Using domain name type");
        req.push_back(0x03);  // ATYP = DOMAIN
        req.push_back((unsigned char)host.length());
        req.insert(req.end(), host.begin(), host.end());
    }
    
    // 添加端口 (网络字节序)
    req.push_back((char)(port >> 8));
    req.push_back((char)(port & 0xFF));
    
    Log("  SOCKS5: Sending CONNECT request, size=" + std::to_string(req.size()));
    if (send(s, req.data(), (int)req.size(), 0) != (int)req.size()) {
        Log("  SOCKS5: Failed to send CONNECT");
        return false;
    }
    
    // 3. 接收响应
    Log("  SOCKS5: Waiting for CONNECT response");
    char final[10];
    int recvd = recv(s, final, 10, 0);
    Log("  SOCKS5: Received " + std::to_string(recvd) + " bytes, status=" + std::to_string((int)final[1]));
    return recvd >= 10 && final[1] == 0x00;
}

bool HttpConnectHandshake(SOCKET s, const std::string& host, int port) {
    Log("  HTTP: Sending CONNECT request");
    std::ostringstream ss;
    ss << "CONNECT " << host << ":" << port << " HTTP/1.1\r\nHost: " << host << "\r\n\r\n";
    std::string req = ss.str();
    if (send(s, req.c_str(), (int)req.length(), 0) != (int)req.length()) {
        Log("  HTTP: Failed to send request");
        return false;
    }
    Log("  HTTP: Waiting for response");
    char buf[1024];
    int r = recv(s, buf, 1023, 0);
    if (r <= 0) {
        Log("  HTTP: No response or error, recv=" + std::to_string(r));
        return false;
    }
    buf[r] = 0;
    bool success = strstr(buf, "200") != nullptr;
    Log("  HTTP: Response received, success=" + std::string(success ? "true" : "false"));
    return success;
}

int ProxyConnect(SOCKET s, const sockaddr* name, int namelen, 
                 int (WSAAPI* originalConnect)(SOCKET, const sockaddr*, int)) {
    if (!g_proxyConfig.enabled) return originalConnect(s, name, namelen);
    
    char addrStr[64] = { 0 };
    int port = 0;
    int family = name->sa_family;

    if (family == AF_INET) {
        sockaddr_in* a = (sockaddr_in*)name;
        inet_ntop(AF_INET, &a->sin_addr, addrStr, sizeof(addrStr));
        port = ntohs(a->sin_port);
        
        // 检查是否是 localhost
        if (strncmp(addrStr, "127.", 4) == 0) {
            Log("Skipping localhost -> " + std::string(addrStr) + ":" + std::to_string(port));
            return originalConnect(s, name, namelen);
        }
        
        // 检查是否是 FakeIP，如果是则还原为域名
        uint32_t ip = ntohl(a->sin_addr.s_addr);
        if (SimpleFakeIP::Instance().IsFakeIP(ip)) {
            std::string host = SimpleFakeIP::Instance().GetHost(ip);
            if (!host.empty()) {
                Log("FakeIP detected: " + std::string(addrStr) + " -> " + host);
                // 使用域名而不是 IP
                strncpy_s(addrStr, sizeof(addrStr), host.c_str(), _TRUNCATE);
            }
        }
    } else if (family == AF_INET6) {
        sockaddr_in6* a = (sockaddr_in6*)name;
        inet_ntop(AF_INET6, &a->sin6_addr, addrStr, sizeof(addrStr));
        port = ntohs(a->sin6_port);
        if (strcmp(addrStr, "::1") == 0) return originalConnect(s, name, namelen);
        
        // 特殊处理：允许 DNS 查询直连（包括 DoH）
        // Google DNS: 2001:4860:4860::8888, 2001:4860:4860::8844
        if (port == 53 || (port == 443 && strstr(addrStr, "2001:4860:4860") != nullptr)) {
            Log("Allowing DNS query (direct) -> " + std::string(addrStr) + ":" + std::to_string(port));
            return originalConnect(s, name, namelen);
        }
        
        // IPv6 策略处理
        if (g_proxyConfig.ipv6_mode == "block") {
            // 阻止 IPv6 连接，强制应用回退到 IPv4
            Log("Blocking IPv6 -> " + std::string(addrStr) + ":" + std::to_string(port) + " (forcing IPv4 fallback)");
            WSASetLastError(WSAEAFNOSUPPORT);
            return SOCKET_ERROR;
        } else if (g_proxyConfig.ipv6_mode == "direct") {
            // 直连 IPv6
            Log("IPv6 direct connection -> " + std::string(addrStr) + ":" + std::to_string(port));
            return originalConnect(s, name, namelen);
        }
        // ipv6_mode == "proxy" 时继续走代理逻辑
        Log("IPv6 connection detected -> " + std::string(addrStr) + ":" + std::to_string(port) + " (proxying)");
    } else return originalConnect(s, name, namelen);

    Log("Proxying -> " + std::string(addrStr) + ":" + std::to_string(port));

    // Switch to blocking for handshake
    unsigned long blocking = 0;
    ioctlsocket(s, FIONBIO, &blocking);

    // 根据 socket 类型选择合适的代理地址格式
    int connectResult = SOCKET_ERROR;
    if (family == AF_INET) {
        // IPv4 socket: 使用 IPv4 地址连接代理
        sockaddr_in proxyAddr = { 0 };
        proxyAddr.sin_family = AF_INET;
        proxyAddr.sin_port = htons((u_short)g_proxyConfig.port);
        inet_pton(AF_INET, g_proxyConfig.host.c_str(), &proxyAddr.sin_addr);
        connectResult = originalConnect(s, (sockaddr*)&proxyAddr, sizeof(proxyAddr));
    } else if (family == AF_INET6) {
        // IPv6 socket: 使用 IPv4-mapped IPv6 地址连接代理
        
        // 禁用 IPV6_V6ONLY，允许 IPv6 socket 连接到 IPv4-mapped 地址
        DWORD v6only = 0;
        setsockopt(s, IPPROTO_IPV6, IPV6_V6ONLY, (char*)&v6only, sizeof(v6only));
        Log("  IPv6 socket: Disabled IPV6_V6ONLY");
        
        sockaddr_in6 proxyAddr6 = { 0 };
        proxyAddr6.sin6_family = AF_INET6;
        proxyAddr6.sin6_port = htons((u_short)g_proxyConfig.port);
        
        // 将 IPv4 代理地址映射为 IPv6 格式 (::ffff:127.0.0.1)
        in_addr proxyIpv4;
        inet_pton(AF_INET, g_proxyConfig.host.c_str(), &proxyIpv4);
        
        // 构造 IPv4-mapped IPv6 地址: ::ffff:x.x.x.x
        unsigned char* bytes = (unsigned char*)&proxyAddr6.sin6_addr;
        memset(bytes, 0, 10);  // 前10字节为0
        bytes[10] = 0xff;      // 第11字节 = 0xff
        bytes[11] = 0xff;      // 第12字节 = 0xff
        memcpy(bytes + 12, &proxyIpv4, 4);  // 后4字节是IPv4地址
        
        Log("  IPv6 socket: Connecting to proxy via IPv4-mapped address");
        connectResult = originalConnect(s, (sockaddr*)&proxyAddr6, sizeof(proxyAddr6));
        if (connectResult == 0) {
            Log("  IPv6 socket: Successfully connected to proxy");
        } else {
            int err = WSAGetLastError();
            Log("  IPv6 socket: Failed to connect to proxy, error=" + std::to_string(err));
        }
    }

    if (connectResult != 0) {
        int err = WSAGetLastError();
        Log("  [Error] Proxy unreachable: " + std::to_string(err));
        return SOCKET_ERROR;
    }

    Log("  Connected to proxy, starting handshake");

    // 设置接收超时，避免永久阻塞
    int timeout = 5000; // 5秒超时
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (char*)&timeout, sizeof(timeout));

    // 使用目标地址字符串进行握手
    // 先尝试 HTTP CONNECT（大多数代理支持），失败再尝试 SOCKS5
    bool ok = HttpConnectHandshake(s, addrStr, port) || Socks5Handshake(s, addrStr, port);

    // Restore non-blocking
    unsigned long nonBlocking = 1;
    ioctlsocket(s, FIONBIO, &nonBlocking);

    if (ok) {
        Log("  [Success] Proxied via " + g_proxyConfig.host);
        return 0;
    }
    
    Log("  [Error] Handshake failed");
    closesocket(s);
    return SOCKET_ERROR;
}

int WSAAPI HookedSendTo(SOCKET s, const char* buf, int len, int flags, const sockaddr* to, int tolen,
                        int (WSAAPI* originalSendTo)(SOCKET, const char*, int, int, const sockaddr*, int)) {
    if (to) {
        int port = (to->sa_family == AF_INET) ? ntohs(((sockaddr_in*)to)->sin_port) : (to->sa_family == AF_INET6 ? ntohs(((sockaddr_in6*)to)->sin6_port) : 0);
        if (port == 443 || port == 53) {
            // Block UDP DNS and QUIC to force TCP fallback
            WSASetLastError(WSAECONNRESET);
            return SOCKET_ERROR;
        }
    }
    return originalSendTo(s, buf, len, flags, to, tolen);
}
