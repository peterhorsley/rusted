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
editor opens. Files can also be dragged from another application onto the window;
multiple dropped files open in separate tabs.

An asterisk on a file tab marks unsaved changes; pathless Untitled tabs never
show an asterisk. Right-click a tab to close it,
close all other tabs, or copy its file path. Middle-click also closes a tab.
Closing a file discards unsaved changes without prompting.
Use Ctrl+Tab and Ctrl+Shift+Tab to cycle through open tabs.

The Edit menu provides undo, redo, cut, copy and paste. The About > Program
menu shows the application version. The toolbar beneath the file tabs opens
Filter / Find and controls word wrap, hidden characters and tailing for the current tab.
Use Ctrl+W, Ctrl+E and Ctrl+T respectively to toggle them.
Enabling tail reloads the file and discards any unsaved edits; editing actions
are disabled while tailing. Toolbar icons are embedded in the executable.
For `.json` files, the toolbar also offers Format JSON, which formats valid JSON
with two-space indentation. Invalid JSON is left unchanged and an error is shown
in the status bar. Formatting is undoable and is disabled while tailing.

Settings and history are stored in `%APPDATA%\rusted` (recent files and window
position).

TODO
----
- Syntax highlighting
- Add link to github on about dialog
- Row highlighting on filtered matches with custom color
- add a "minimap" feature the same as the one in vscode.  it's a vertical bar that shows the content of the entire file in 