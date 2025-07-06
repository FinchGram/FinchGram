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

# Turn on standard Qt project layout (optional but handy)
qt_standard_project_setup()

# Our executable and its sources
add_executable(${PROJECT_NAME}
    src/main.cpp
    src/MainWindow.h
    src/MainWindow.cpp
    src/MainWindow.ui
)

# Link against Qt Widgets
target_link_libraries(${PROJECT_NAME} PRIVATE Qt6::Widgets)
