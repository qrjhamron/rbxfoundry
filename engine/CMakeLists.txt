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

add_library(blox_engine_lib STATIC src/json.cpp src/datamodel.cpp src/scene.cpp src/mesh.cpp src/renderer.cpp src/ui.cpp src/runtime.cpp src/physics.cpp src/terrain.cpp src/script_tools.cpp ${BLOX_SPV_HEADER})
# Jolt is vendored with C++ RTTI off; the physics TU derives from
# JPH::GroupFilter and must match (no dynamic_cast/typeid used there).
set_source_files_properties(src/physics.cpp PROPERTIES COMPILE_OPTIONS "-fno-rtti")
target_include_directories(blox_engine_lib PUBLIC include PRIVATE ${CMAKE_CURRENT_BINARY_DIR})
target_compile_features(blox_engine_lib PUBLIC cxx_std_20)
target_link_libraries(blox_engine_lib PUBLIC Vulkan::Vulkan PNG::PNG Luau.Compiler Luau.VM Luau.Analysis Jolt)

find_package(Freetype QUIET)
if(Freetype_FOUND)
    target_link_libraries(blox_engine_lib PUBLIC Freetype::Freetype)
    target_compile_definitions(blox_engine_lib PUBLIC BLOX_HAVE_FREETYPE=1)
endif()

add_executable(blox-engine src/main.cpp)
target_link_libraries(blox-engine PRIVATE blox_engine_lib)

if(BLOX_BUILD_TESTS)
    add_executable(test_datamodel tests/test_datamodel.cpp)
    target_link_libraries(test_datamodel PRIVATE blox_engine_lib)
    add_test(NAME engine_datamodel COMMAND test_datamodel)
    add_executable(test_scene tests/test_scene.cpp)
    target_link_libraries(test_scene PRIVATE blox_engine_lib)
    target_compile_definitions(test_scene PRIVATE BLOX_FIXTURE_DIR="${CMAKE_CURRENT_SOURCE_DIR}/../tests/fixtures")
    add_test(NAME engine_scene COMMAND test_scene WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}/..)
endif()
