cmake_minimum_required(VERSION 3.16)

set(CMAKE_EXPORT_COMPILE_COMMANDS ON)


project(finchgram LANGUAGES CXX)


# Use C++17
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Point CMake at your Homebrew Qt 6
set(CMAKE_PREFIX_PATH "/opt/homebrew/opt/qt")

# Find the Widgets module
find_package(Qt6 COMPONENTS Widgets REQUIRED)

if(APPLE)
    # Help CMake find Homebrew zlib on macOS
    set(CMAKE_PREFIX_PATH ${CMAKE_PREFIX_PATH} "/opt/homebrew/opt/zlib")
endif()

# Add TDLib using FetchContent
include(FetchContent)
FetchContent_Declare(
  td
  GIT_REPOSITORY https://github.com/tdlib/td.git
  GIT_TAG        master
)

# Set TDLib options before making it available
set(TD_ENABLE_JNI OFF CACHE BOOL "")
set(TD_ENABLE_DOTNET OFF CACHE BOOL "")

# Make TDLib available
FetchContent_MakeAvailable(td)

# Turn on standard Qt project layout (optional but handy)
qt_standard_project_setup()

# Our executable and its sources
add_executable(${PROJECT_NAME}
    src/main.cpp
    src/MainWindow.h
    src/MainWindow.cpp
    src/MainWindow.ui
)

# Link against Qt Widgets and TDLib
target_link_libraries(${PROJECT_NAME} PRIVATE 
    Qt6::Widgets
    Td::TdStatic
)
