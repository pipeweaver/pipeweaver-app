# Pipeweaver Wrapper App

This is a simple app designed to be used with [Pipeweaver](https://github.com/pipeweaver/pipeweaver). It serves to
provide a more integrated 'Desktop' app experience for users who want to interact with Pipeweaver without using a web
browser.

# Building

This package requires the QT6 WebEngine development libraries to be installed. On debian based systems, you can install
them with:

```bash
sudo apt-get install qt6-webengine-dev
```

On Fedora-based systems, you can install them with:

```bash
sudo dnf install qt6-qtwebengine-devel
```

On Arch-based systems, you can install them with:

```bash
sudo pacman -S qt6-webengine
```

---
Then for building, you'll need cargo installed and functional. You can build the app with:

```bash
cargo build --release
```

## Installation

After building, you can find the executable in the `target/release` directory. To use this with Pipeweaver you need
to move the `pipweaver-app` binary to either a location in your `$PATH`, or next to the `pipeweaver-daemon` binary.

When you click on the Pipeweaver tray icon, it will then launch this app instead of the default web browser.

## MORE COMING SOON :p

Longer term, this binary will be distributed directly with Pipeweaver, to prevent the need to compile it yourself.