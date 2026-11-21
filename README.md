# rusted

A minimal, fast, cross-platform GUI text editor written in Rust (egui/eframe).

## Large-file performance

The normal editor still lays out the entire document with egui's `TextEdit`,
so large files can block the UI. Program > Enable virtual preview switches to
an experimental read-only viewport that lays out only visible lines.
Use Word wrap (or Ctrl+W) to wrap the preview. Wrapped scroll geometry is
estimated for unvisited lines and refined as they are viewed, so the scrollbar
and distant drag positions can shift. Resizing or changing the font rebuilds
that geometry, retaining the current logical line.
Use the wheel, scrollbar, arrow keys, Page Up/Down, Ctrl+Home/End or Go to line
to navigate. Find results can be searched and opened, and tailing still reads
the original file; the preview does not show search highlights, selection,
or hidden characters. It shows only the first 2048 bytes of a
long line, with an ellipsis, and is not an editor. The existing editor remains
available via Program > Disable virtual preview. In the preview, Find results
also wrap when Word wrap is enabled. Wrapped Find-result heights are estimated
until viewed, so scrollbar positions can shift. Wrapped results show at most
the first 2048 bytes of each matching line, with an ellipsis.

The sparse document line index is rebuilt after text changes, including tail
appends. Wrapped Find results in the normal editor can still require whole-result layout. Responsive
editing, wrapping and tailing of 100 MB files are not yet established.

To profile the optional local sample, run
`cargo test --release --locked profile_large_file_editor -- --ignored --nocapture`.
Use `cargo test --release --locked profile_virtual_preview -- --ignored --nocapture`
to measure the experimental preview, including its Find results panel.
Use `cargo test --release --locked profile_wrapped_virtual_preview -- --ignored --nocapture`
to measure the wrapped preview.
Use `cargo test --release --locked profile_preview_wrapped_find -- --ignored --nocapture`
to measure the wrapped Find results.
The normal-editor benchmark checks that wheel input changes the visible gutter lines and
reports both the input frame and the subsequent scrolled frame. It measures
headless egui frames, not GPU rendering or end-to-end UI latency.
To profile a different sufficiently long file, set `RUSTED_PROFILE_FILE` to
its path before running the command. The search workload uses `response` and
reports its match count.

## Building a Windows exe

### Prerequisites

1. **Rust toolchain** (stable, MSVC target). Install with winget or from <https://rustup.rs>:

   ```powershell
   winget install --id Rustlang.Rustup -e
   rustup default stable
   ```

2. **MSVC build tools**: Visual Studio or Build Tools with the
   "Desktop development with C++" workload (provides the linker).

3. **Git for Windows**, including Git Bash. Builds run `gitversion-fast.sh`
   from a Git checkout to determine the version.

Open a new terminal afterwards so `cargo` is on your `PATH`.

### Build

From the project folder:

```powershell
cargo build --release
```

The executable is written to:

```
target\release\RustEd.exe
```

It is a single self-contained file: copy it anywhere and run it. The release
profile uses LTO and strips symbols for fast startup and a small binary. The
first build downloads and compiles dependencies and takes a few minutes;
later builds are much faster.

The build uses `gitversion-fast.sh` to set the About dialog and executable
file and product versions from Git tags and the current branch. Windows' numeric
version fields contain the major, minor and patch numbers; the full version,
including any prerelease suffix, is stored in the version strings.

The Windows executable embeds `src\assets\app.ico`, derived from
`src\assets\app.png`. Regenerate the ICO if the PNG changes; Pillow can do so:

```powershell
py -c "from PIL import Image; im=Image.open(r'src\assets\app.png').convert('RGBA'); sizes=[16,24,32,48,64,128,256]; frames=[im.resize((n,n), Image.Resampling.LANCZOS) for n in sizes]; frames[-1].save(r'src\assets\app.ico', sizes=[(n,n) for n in sizes], append_images=frames[:-1])"
```

A debug build (`cargo build`) goes to `target\debug\RustEd.exe` and is slower
at runtime.

If the build fails with "Access is denied" removing `RustEd.exe`, close any
running copy of the editor first.

### Running tests

From the project folder, run the same tests used by GitHub Actions:

```powershell
cargo test --locked
```

To run tests matching a name, pass a filter, for example:

```powershell
cargo test --locked search
```

### Building in VS Code

Install the **rust-analyzer** extension, open the integrated terminal
(Ctrl+`) and run `cargo build --release`. Alternatively run
**Tasks: Configure Default Build Task**, choose `rust: cargo build`, and add
`"args": ["--release"]` to the task for a Ctrl+Shift+B release build.

### Automatic Windows builds

Pushes to `main` run the tests and build a Windows executable using GitHub
Actions. A successful build publishes `RustEd.exe` as a GitHub release
tagged `build-<commit SHA>`, titled `RustEd v<major>.<minor>.<patch>`, with
commit-based change notes since the previous published build. Numeric titles
may repeat until a new version tag is created. Pushes containing multiple
commits build the latest commit, and only the current `main` tip is marked
as the latest release.
The workflow uses the built-in `GITHUB_TOKEN` with contents write permission,
so no personal access token is needed. GitHub Actions must be enabled and
repository or organization policies must allow that permission.

After committing and pushing the workflow to `main`, check the repository's
Actions tab for the run and its Releases page for the executable. Standard
GitHub-hosted runners are free for public repositories. The executable is
unsigned, so Windows SmartScreen may warn before running it.

## Running

```powershell
RustEd.exe [-t] [file]
```

An optional single argument is the path of a file to open; without it an empty
editor opens. Files can also be dragged from another application onto the window;
multiple dropped files open in separate tabs.

Use `RustEd.exe -t path\to\file` to open a file with tailing enabled. The `-t`
switch can also follow the file path. Tailing requires a file path; use a path
such as `.\-t` to open a file literally named `-t` without enabling tailing.
On Windows, launching RustEd again activates the existing window and opens the
requested file in it. If `-t` is used on an already-open file, tailing reloads
it and discards any unsaved edits.

An asterisk on a file tab marks unsaved changes; pathless Untitled tabs never
show an asterisk. Tabs wrap onto additional rows when they do not fit across
the window. Right-click a tab to close it,
close all other tabs, or copy its file path. Middle-click also closes a tab.
Closing a file discards unsaved changes without prompting.
Use Ctrl+Tab and Ctrl+Shift+Tab to cycle through open tabs.

The Edit menu provides undo, redo, cut, copy and paste. The Program > About
dialog shows the application version, description, designer and command-line usage.
The toolbar beneath the file tabs opens Filter / Find and controls word wrap,
hidden characters and tailing for the current tab.
Word wrap and hidden-character markers also apply to the filter results.
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
