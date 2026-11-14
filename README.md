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
rusted.exe [-t] [file]
```

An optional single argument is the path of a file to open; without it an empty
editor opens. Files can also be dragged from another application onto the window;
multiple dropped files open in separate tabs.

Use `rusted.exe -t path\to\file` to open a file with tailing enabled. The `-t`
switch can also follow the file path. Tailing requires a file path; use `--`
before the path to open a file literally named `-t` without enabling tailing.
On Windows, launching RustEd again activates the existing window and opens the
requested file in it. If `-t` is used on an already-open file, tailing reloads
it and discards any unsaved edits.

An asterisk on a file tab marks unsaved changes; pathless Untitled tabs never
show an asterisk. Tabs wrap onto additional rows when they do not fit across
the window. Right-click a tab to close it,
close all other tabs, or copy its file path. Middle-click also closes a tab.
Closing a file discards unsaved changes without prompting.
Use Ctrl+Tab and Ctrl+Shift+Tab to cycle through open tabs.

The Edit menu provides undo, redo, cut, copy and paste. The About > Program
menu shows the application version. The toolbar beneath the file tabs opens
Filter / Find and controls word wrap, hidden characters and tailing for the current tab.
File > Find / Replace (F4 or Ctrl+F) also offers Replace all, which uses the
current Find text and case-sensitivity setting. An empty replacement deletes
matches; an empty Find text does nothing. Replacements can be undone and are
disabled while tailing. Find and Replace each offer a dropdown of their last
10 nonempty values, shared across tabs for the current session.
The Find bar offers five background colors for matching lines, defaulting to Dark Green. Search replaces
the previous highlights; changing the color updates the current highlights.
Matching text is white. Editing a highlighted line clears its highlight until
the next search; newly tailed matches are highlighted automatically. Dark Gray
uses the normal editor background. The main editor is read-only while tailing.
Double-click a search result to jump to its line, shown in yellow text.
Use File > Go to line (Ctrl+G) to jump to a line and mark its line number white.
Use Ctrl+W, Ctrl+E and Ctrl+T respectively to toggle them.
Enabling tail reloads the file and discards any unsaved edits; editing actions
are disabled while tailing. Tail scrolling moves to the bottom-left of the editor.
Toolbar icons are embedded in the executable.
For `.json` files, the toolbar also offers Format JSON, which formats valid JSON
with two-space indentation. Invalid JSON is left unchanged and an error is shown
in the status bar. Formatting is undoable and is disabled while tailing.

Settings and history are stored in `%APPDATA%\rusted` (recent files and window
position).

TODO
----
- Syntax highlighting
- Add link to github on about dialog
- add a "minimap" feature the same as the one in vscode.  it's a vertical bar that shows the content of the entire file in 
- linux/mac
- installer / open with in windows?
- check for updates?
- ci-cd?
- refactor to > 1 file, add detailed comments, re-do readme
- app icon