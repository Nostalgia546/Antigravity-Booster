#pragma once
#include <string>
#include <unordered_map>
#include <mutex>
#include <winsock2.h>

// 简化的 FakeIP 实现
class SimpleFakeIP {
private:
    std::unordered_map<uint32_t, std::string> ipToHost;
    std::unordered_map<std::string, uint32_t> hostToIp;
    std::mutex mtx;
    uint32_t nextIp = 0xC6120001; // 198.18.0.1

public:
    // 为域名分配虚拟 IP
    uint32_t Allocate(const std::string& host) {
        std::lock_guard<std::mutex> lock(mtx);
        
        auto it = hostToIp.find(host);
        if (it != hostToIp.end()) {
            return it->second;
        }
        
        uint32_t fakeIp = nextIp++;
        ipToHost[fakeIp] = host;
        hostToIp[host] = fakeIp;
        return fakeIp;
    }
    
    // 根据虚拟 IP 获取域名
    std::string GetHost(uint32_t ip) {
        std::lock_guard<std::mutex> lock(mtx);
        auto it = ipToHost.find(ip);
        return (it != ipToHost.end()) ? it->second : "";
    }
    
    // 检查是否是虚拟 IP (198.18.0.0/16)
    bool IsFakeIP(uint32_t ip) {
        return (ip & 0xFFFF0000) == 0xC6120000;
    }
    
    static SimpleFakeIP& Instance() {
        static SimpleFakeIP instance;
        return instance;
    }
};
