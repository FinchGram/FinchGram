cmake_minimum_required(VERSION 4.0.2)

set(CMAKE_EXPORT_COMPILE_COMMANDS ON)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

project(FinchGram LANGUAGES CXX)

if(APPLE)
    # Point CMake at Homebrew packages on macOS
    list(APPEND CMAKE_PREFIX_PATH 
        "/opt/homebrew/opt/qt"
        "/opt/homebrew/opt/zlib"
    )
endif()

# Find the Widgets module
find_package(Qt6 COMPONENTS Widgets REQUIRED)

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

# === Utils Library ===
set(UTILS_SOURCES 
    "src/utils/SettingsManager.h"
    "src/utils/SettingsManager.cpp"
    "src/utils/Constants.h"
    "src/utils/Constants.cpp"
    # Add .cpp files here when you create them
)
if(UTILS_SOURCES)
    add_library(FinchGramUtils STATIC ${UTILS_SOURCES})
    target_link_libraries(FinchGramUtils PRIVATE Qt6::Widgets)
endif()

# === Main Executable ===
set(MAIN_SOURCES 
    "src/main.cpp"
    "src/MainWindow.cpp" 
    "src/MainWindow.h"
    "src/MainWindow.ui"
)
add_executable(${PROJECT_NAME} ${MAIN_SOURCES})

# Link against Qt Widgets and TDLib
target_link_libraries(${PROJECT_NAME} PRIVATE 
    Qt6::Widgets
    Td::TdStatic
)

# Link against Utils if the target exists
if(TARGET FinchGramUtils)
    target_link_libraries(${PROJECT_NAME} PRIVATE FinchGramUtils)
endif()
