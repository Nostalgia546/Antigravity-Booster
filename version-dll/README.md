# version.dll - System Proxy DLL for Antigravity

## 功能
- 自动读取 Windows 系统代理设置
- Hook Socket API 重定向网络流量
- 支持 SOCKS5 和 HTTP CONNECT 代理协议

## 编译步骤

### 1. 安装依赖
- Visual Studio 2019/2022（需要 C++ 桌面开发工具）
- CMake 3.15+
- Microsoft Detours 库

### 2. 下载并编译 Detours
```powershell
# 下载 Detours
git clone https://github.com/microsoft/Detours.git
cd Detours

# 编译（使用 VS Developer Command Prompt）
nmake
```

### 3. 编译 version.dll
```powershell
# 创建 build 目录
mkdir build
cd build

# 配置 CMake（设置 Detours 路径）
cmake .. -DDETOURS_DIR="C:/path/to/Detours"

# 编译
cmake --build . --config Release
```

### 4. 输出文件
编译完成后，`version.dll` 会在 `build/Release/` 目录中。

## 使用方法
1. 将 `version.dll` 复制到 Antigravity 安装目录
2. 确保 Windows 系统代理已配置
3. 启动 Antigravity，DLL 会自动加载并使用系统代理

## 工作原理
1. DLL 加载时读取 Windows 系统代理设置（通过 WinHTTP API）
2. Hook `connect` 和 `WSAConnect` 函数
3. 拦截所有网络连接请求
4. 通过 SOCKS5 或 HTTP CONNECT 协议转发到代理服务器

## 注意事项
- 仅支持 x64 架构
- 需要管理员权限编译（Detours 需要）
- 本地回环地址（127.x.x.x）不会被代理
