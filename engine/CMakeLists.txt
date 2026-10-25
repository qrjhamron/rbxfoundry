find_package(Vulkan REQUIRED)
find_package(PNG REQUIRED)
find_program(GLSLANG_VALIDATOR glslangValidator REQUIRED)

# Compile GLSL shaders to SPIR-V and embed them into the engine binary.
set(BLOX_SHADER_SOURCES
    shaders/scene.vert shaders/scene.frag
    shaders/sky.vert shaders/sky.frag
    shaders/shadow.vert
    shaders/ui.vert shaders/ui.frag
)
set(BLOX_SPV_FILES "")
set(BLOX_SPV_NAMES "")
set(BLOX_SHADER_VARS
    "scene.vert=kSceneVertSpv" "scene.frag=kSceneFragSpv"
    "sky.vert=kSkyVertSpv" "sky.frag=kSkyFragSpv"
    "shadow.vert=kShadowVertSpv"
    "ui.vert=kUiVertSpv" "ui.frag=kUiFragSpv")
foreach(shader ${BLOX_SHADER_SOURCES})
    get_filename_component(base ${shader} NAME)
    set(var "")
    foreach(pair ${BLOX_SHADER_VARS})
        string(REGEX MATCH "^${base}=(.*)$" _m ${pair})
        if(CMAKE_MATCH_1)
            set(var ${CMAKE_MATCH_1})
        endif()
    endforeach()
    if(NOT var)
        message(FATAL_ERROR "no SPIR-V variable name mapped for shader ${shader}")
    endif()
    set(spv "${CMAKE_CURRENT_BINARY_DIR}/${base}.spv")
    add_custom_command(
        OUTPUT ${spv}
        COMMAND ${GLSLANG_VALIDATOR} -V ${CMAKE_CURRENT_SOURCE_DIR}/${shader} -o ${spv}
        DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/${shader}
        COMMENT "Compiling shader ${shader}")
    list(APPEND BLOX_SPV_FILES ${spv})
    list(APPEND BLOX_SPV_NAMES ${var})
endforeach()
string(REPLACE ";" "," BLOX_SPV_FILES_JOINED "${BLOX_SPV_FILES}")
string(REPLACE ";" "," BLOX_SPV_NAMES_JOINED "${BLOX_SPV_NAMES}")
set(BLOX_SPV_HEADER "${CMAKE_CURRENT_BINARY_DIR}/shaders_spv.h")
add_custom_command(
    OUTPUT ${BLOX_SPV_HEADER}
    COMMAND ${CMAKE_COMMAND} "-DSPV_FILES=${BLOX_SPV_FILES_JOINED}" "-DNAMES=${BLOX_SPV_NAMES_JOINED}"
            "-DOUT=${BLOX_SPV_HEADER}" -P ${CMAKE_CURRENT_SOURCE_DIR}/cmake/embed_spv.cmake
    DEPENDS ${BLOX_SPV_FILES} ${CMAKE_CURRENT_SOURCE_DIR}/cmake/embed_spv.cmake
    COMMENT "Embedding SPIR-V shaders")

# --- Domain libraries: dependencies below are the enforced architecture
# --- boundaries (see docs/ARCHITECTURE.md). A domain may only link the
# --- domains listed for it; main.cpp links only blox_ipc.
find_package(Freetype QUIET)

add_library(blox_core STATIC src/core/json.cpp)
target_include_directories(blox_core PUBLIC include)
target_compile_features(blox_core PUBLIC cxx_std_20)

add_library(blox_datamodel STATIC src/datamodel/datamodel.cpp src/datamodel/reflection.cpp src/datamodel/transforms.cpp)
target_include_directories(blox_datamodel PUBLIC include PRIVATE src/datamodel)
target_compile_features(blox_datamodel PUBLIC cxx_std_20)
target_link_libraries(blox_datamodel PUBLIC blox_core)

add_library(blox_terrain STATIC src/terrain/terrain.cpp)
target_include_directories(blox_terrain PUBLIC include)
target_compile_features(blox_terrain PUBLIC cxx_std_20)
target_link_libraries(blox_terrain PUBLIC blox_datamodel)

add_library(blox_physics STATIC src/physics/physics.cpp)
# Jolt is vendored with C++ RTTI off; the physics TU derives from
# JPH::GroupFilter and must match (no dynamic_cast/typeid used there).
set_source_files_properties(src/physics/physics.cpp PROPERTIES COMPILE_OPTIONS "-fno-rtti")
target_include_directories(blox_physics PUBLIC include)
target_compile_features(blox_physics PUBLIC cxx_std_20)
target_link_libraries(blox_physics PUBLIC blox_datamodel blox_terrain Jolt)

add_library(blox_gui STATIC src/gui/text.cpp src/gui/layout.cpp)
target_include_directories(blox_gui PUBLIC include PRIVATE src/gui)
target_compile_features(blox_gui PUBLIC cxx_std_20)
target_link_libraries(blox_gui PUBLIC blox_datamodel PNG::PNG)
if(Freetype_FOUND)
    target_link_libraries(blox_gui PUBLIC Freetype::Freetype)
    target_compile_definitions(blox_gui PUBLIC BLOX_HAVE_FREETYPE=1)
endif()

add_library(blox_rendering STATIC src/rendering/scene.cpp src/rendering/mesh.cpp src/rendering/renderer.cpp src/rendering/image.cpp ${BLOX_SPV_HEADER})
target_include_directories(blox_rendering PUBLIC include PRIVATE ${CMAKE_CURRENT_BINARY_DIR} src/rendering)
target_compile_features(blox_rendering PUBLIC cxx_std_20)
target_link_libraries(blox_rendering PUBLIC blox_datamodel blox_terrain blox_gui Vulkan::Vulkan PNG::PNG)

add_library(blox_scripting STATIC src/scripting/runtime.cpp src/scripting/bindings.cpp src/scripting/script_tools.cpp)
target_include_directories(blox_scripting PUBLIC include PRIVATE src/scripting)
target_compile_features(blox_scripting PUBLIC cxx_std_20)
target_link_libraries(blox_scripting PUBLIC blox_datamodel blox_physics blox_terrain blox_gui Luau.Compiler Luau.VM Luau.Analysis)

add_library(blox_ipc STATIC src/ipc/server.cpp src/ipc/handlers_script.cpp src/ipc/handlers_datamodel_newload.cpp src/ipc/handlers_datamodel.cpp src/ipc/handlers_runtime.cpp src/ipc/handlers_debug.cpp src/ipc/handlers_physics.cpp src/ipc/handlers_terrain.cpp src/ipc/handlers_render.cpp)
target_include_directories(blox_ipc PUBLIC include PRIVATE src/ipc)
target_compile_features(blox_ipc PUBLIC cxx_std_20)
target_link_libraries(blox_ipc PUBLIC blox_core blox_datamodel blox_terrain blox_physics blox_gui blox_rendering blox_scripting)

# Umbrella for tests and external consumers (interface only).
add_library(blox_engine_lib INTERFACE)
target_link_libraries(blox_engine_lib INTERFACE blox_ipc)

add_executable(blox-engine src/main.cpp)
target_link_libraries(blox-engine PRIVATE blox_ipc)

if(BLOX_BUILD_TESTS)
    add_executable(test_datamodel tests/test_datamodel.cpp)
    target_link_libraries(test_datamodel PRIVATE blox_engine_lib)
    add_test(NAME engine_datamodel COMMAND test_datamodel)
    add_executable(test_scene tests/test_scene.cpp)
    target_link_libraries(test_scene PRIVATE blox_engine_lib)
    target_compile_definitions(test_scene PRIVATE BLOX_FIXTURE_DIR="${CMAKE_CURRENT_SOURCE_DIR}/../tests/fixtures")
    add_test(NAME engine_scene COMMAND test_scene WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}/..)
endif()
