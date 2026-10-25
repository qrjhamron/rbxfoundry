cmake_minimum_required(VERSION 3.20)
project(bloxcli VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

option(BLOX_BUILD_TESTS "Build engine tests" ON)
if(BLOX_BUILD_TESTS)
    enable_testing()
endif()

# Official Luau ( vendored, release/744 ): compiler + VM only, no CLI/tests.
set(LUAU_BUILD_CLI OFF CACHE BOOL "" FORCE)
set(LUAU_BUILD_TESTS OFF CACHE BOOL "" FORCE)
add_subdirectory(third_party/luau EXCLUDE_FROM_ALL)

# Jolt Physics (vendored v5.2.0): library only, no samples/tests/viewer.
set(TARGET_UNIT_TESTS OFF CACHE BOOL "" FORCE)
set(TARGET_HELLO_WORLD OFF CACHE BOOL "" FORCE)
set(TARGET_PERFORMANCE_TEST OFF CACHE BOOL "" FORCE)
set(TARGET_SAMPLES OFF CACHE BOOL "" FORCE)
set(TARGET_VIEWER OFF CACHE BOOL "" FORCE)
add_subdirectory(third_party/JoltPhysics/Build EXCLUDE_FROM_ALL)

add_subdirectory(engine)
