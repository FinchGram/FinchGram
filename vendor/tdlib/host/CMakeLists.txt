# finchgram-tdlib: our small host (main.cpp) with TDLib linked in statically.
# Configured by vendor/tdlib/build.sh, which passes TD_SOURCE_DIR and our own static OpenSSL.
cmake_minimum_required(VERSION 3.16)
project(finchgram-tdlib LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if (NOT TD_SOURCE_DIR)
  message(FATAL_ERROR "TD_SOURCE_DIR is not set: build with vendor/tdlib/build.sh")
endif()

# Only what finchgram-tdlib needs is built: none of TDLib's own programs, tests or benchmarks.
add_subdirectory("${TD_SOURCE_DIR}" td EXCLUDE_FROM_ALL)

add_executable(finchgram-tdlib main.cpp)
target_link_libraries(finchgram-tdlib PRIVATE Td::TdJsonStatic)
