# FinchGram

[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)

FinchGram is an open-source Telegram media-center desktop client built from scratch with Qt/C++ and TDLib. It extends standard messaging with rich media features while adhering to the GNU GPLv3 license for core components.

## 🚀 Features

- **Cross-Platform**: Native support for Windows, macOS, and Linux
- **Secure Core**: Powered by TDLib (Boost Software License 1.0) for MTProto, encryption, and local caching
- **Media-Center Enhancements**: In-chat media playback, file-browser integration, and custom media panels
- **Theming**: Finch-inspired warm color palette with flexible Qt theming
- **Extensible**: Plugin architecture planned for future proprietary and open-source extensions

## For Users

- Download Link: TODO

## For Developers

### 📋 Prerequisites

- C++17 compiler (GCC, Clang, or MSVC)
- CMake ≥ 3.16
- Qt 6 (LGPL 3.0, dynamic linking)
- Git

### 🛠️ Quickstart

```bash
# Clone the repo and init TDLib submodule
git clone https://github.com/FinchGram/FinchGram.git
cd FinchGram
git submodule update --init --recursive

# Build
mkdir build && cd build
cmake ..
cmake --build . --config Release

# Run
./finchgram
```

### 📂 Project Structure

```text
finchgram/
├── LICENSE               # GNU GPLv3 license text
├── README.md             # Project overview and quickstart
├── CMakeLists.txt        # Top-level build configuration
├── CONTRIBUTING.md       # Contribution guidelines
├── CODE_OF_CONDUCT.md    # Community Code of Conduct
├── third_party/tdlib/    # TDLib submodule (Boost 1.0 License)
└── src/
    ├── main.cpp          # QApplication entry point
    ├── UI/               # Qt widgets, resources, QML files
    └── Core/             # TDLib JSON-RPC wrapper and event loop
```

## 📝 License

This project is licensed under the **GNU General Public License v3.0**. See [LICENSE](LICENSE) for details.

TDLib is included as a git submodule under the **Boost Software License 1.0**; its source and license text reside in `third_party/tdlib/`.

Qt is used under **LGPL 3.0** via dynamic linking. Users may replace or update Qt shared libraries as permitted by the license.

## 🤝 Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) for detailed guidelines on code style, testing, and submitting pull requests.

## 📜 Code of Conduct

This project follows the [Contributor Covenant v2.1](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

---

_Built with ❤️ by the FinchGram community._
