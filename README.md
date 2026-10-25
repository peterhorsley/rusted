# rusted

A minimal, fast, cross-platform GUI text editor written in Rust (egui/eframe).

## Building a Windows exe

### Prerequisites

1. **Rust toolchain** (stable, MSVC target). Install with winget or from <https://rustup.rs>:

   ```powershell
   winget install --id Rustlang.Rustup -e
   rustup default stable
   ```

2. **MSVC build tools**: Visual Studio or Build Tools with the
   "Desktop development with C++" workload (provides the linker).

Open a new terminal afterwards so `cargo` is on your `PATH`.

### Build

From the project folder:

```powershell
cargo build --release
```

The executable is written to:

```
target\release\rusted.exe
```

It is a single self-contained file: copy it anywhere and run it. The release
profile uses LTO and strips symbols for fast startup and a small binary. The
first build downloads and compiles dependencies and takes a few minutes;
later builds are much faster.

A debug build (`cargo build`) goes to `target\debug\rusted.exe` and is slower
at runtime.

If the build fails with "Access is denied" removing `rusted.exe`, close any
running copy of the editor first.

### Building in VS Code

Install the **rust-analyzer** extension, open the integrated terminal
(Ctrl+`) and run `cargo build --release`. Alternatively run
**Tasks: Configure Default Build Task**, choose `rust: cargo build`, and add
`"args": ["--release"]` to the task for a Ctrl+Shift+B release build.

## Running

```powershell
rusted.exe [file]
```

An optional single argument is the path of a file to open; without it an empty
editor opens.

Settings and history are stored in `%APPDATA%\rusted` (recent files and window
position).
