cmake_minimum_required(VERSION 3.5.0)
project(photocraft)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(NATIVERENDER_ROOT_PATH ${CMAKE_CURRENT_SOURCE_DIR})
set(PHOTOCRAFT_LIB ${NATIVERENDER_ROOT_PATH}/../../../libs/${OHOS_ARCH}/libphotocraft_core.so)

include_directories(${NATIVERENDER_ROOT_PATH})

add_library(entry SHARED napi_init.cpp)

target_link_libraries(entry PUBLIC libace_napi.z.so)
target_link_libraries(entry PUBLIC ${PHOTOCRAFT_LIB})

set_target_properties(entry PROPERTIES INSTALL_RPATH "$ORIGIN")