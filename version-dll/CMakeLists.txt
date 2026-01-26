cmake_minimum_required(VERSION 3.15)
project(version_dll)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Add source files
add_library(version SHARED
    dllmain.cpp
    proxy.cpp
    proxy.h
    pch.cpp
    pch.h
    version_forwarder.cpp
)

# Link libraries
target_link_libraries(version
    ws2_32
    winhttp
)

# 使用模块定义文件导出函数
set_target_properties(version PROPERTIES
    LINK_FLAGS "/DEF:\"${CMAKE_CURRENT_SOURCE_DIR}/version.def\""
)

# Set output name
set_target_properties(version PROPERTIES OUTPUT_NAME "version")

# Enable precompiled headers
target_precompile_headers(version PRIVATE pch.h)

# Add Detours library
# 这里的路径调整为：从当前 CMakeLists.txt 所在目录出发，找到并列的 Detours 文件夹
set(DETOURS_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../Detours/Detours-main" CACHE PATH "Path to Detours library")
target_include_directories(version PRIVATE ${DETOURS_DIR}/include)
target_link_libraries(version ${DETOURS_DIR}/lib.X64/detours.lib)
