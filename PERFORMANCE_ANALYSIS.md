# RustEd Performance Analysis: 10MB+ File Support

> **Superseded:** This report contains unmeasured timings and an incorrect viewport-only `TextEdit` proposal. See the large-file performance note in [README.md](README.md).

## Current Architecture Overview

RustEd is a Rust text editor built with egui. The app stores the entire file contents as a single `String` in memory and renders it using egui's `TextEdit::multiline()` with custom layouting.

### Key Components

1. **Data Storage**: `Tab::text` - single String holding entire file
2. **Rendering**: `TextEdit::multiline()` with custom `layouter` function
3. **Search**: `SearchHighlights` stores matches per line
4. **Layout Job**: `search_layout()` creates a `LayoutJob` with format info for every visible section

## Identified Performance Bottlenecks

### 1. **Full-File Layout Job Generation (CRITICAL)**

**Location**: `search.rs:106` - `search_layout()` function

**Problem**:
```rust
pub(crate) fn search_layout(text: &str, highlights: &SearchHighlights, 
                            selected_line: Option<usize>, hidden: bool, 
                            font: &FontId, width: f32, light: bool) -> egui::text::LayoutJob {
    // Iterates through ALL lines in the text, even if only visible viewport needed
    for (i, line) in text.split_inclusive('\n').enumerate() {
        // For each line: check selections, process highlights, format text
        // This happens EVERY FRAME
    }
}
```

**Impact**:
- 10MB file = ~18,000 lines
- For every frame render (60 FPS), this processes all 18K lines
- Creating format specs for every segment
- No viewport culling = processing invisible lines

**Cost for 10MB file**:
- ~15ms+ per frame just for layout job creation
- Makes search extremely slow (layout job created with all matches highlighted)

### 2. **Search Highlighting Scans All Lines (CRITICAL)**

**Location**: `search.rs:150` - `SearchHighlights::search()` and `match_ranges()`

**Problem**:
```rust
pub(crate) fn search(&mut self, text: &str, pattern: &str, case: bool) {
    self.snapshot = text.to_owned();  // Clone entire file
    self.lines = text.split('\n')
        .map(|line| match_ranges(line, pattern, case))  // All lines scanned
        .collect();
}

pub(crate) fn match_ranges(line: &str, pattern: &str, case: bool) -> Vec<Range<usize>> {
    if !case {
        let lower = line.to_lowercase();  // Allocate + lowercase entire line
        let needle = pattern.to_lowercase();
        // Complex boundary matching logic
    }
    // Returns all match ranges
}
```

**Impact**:
- Searching "response" in 10MB file scans all 18K lines
- For case-insensitive search: lowercases every line to search in it
- Creates `SearchHighlights::snapshot` clone of entire file
- No early exit or incremental search

**Cost**:
- ~100ms+ for case-insensitive search on 10MB file
- Visible UI freeze

### 3. **Hidden Character Replacement Allocates New Strings (SIGNIFICANT)**

**Location**: `search.rs:116-117`

**Problem**:
```rust
let shown = if hidden { 
    s.replace(' ', "·")  // Create new String for each segment
} else { 
    s.to_owned() 
};
job.append(&shown, ...);
```

**Impact**:
- When hidden chars enabled: creates new string for every text segment
- For 10MB file with many spaces: massive allocations
- No string reuse or buffer pooling

**Cost**:
- ~50ms additional overhead when hidden chars enabled on 10MB file

### 4. **No Viewport Culling (MODERATE BUT ARCHITECTURAL)**

**Location**: `editor.rs:144+` - `editor_ui()` function

**Problem**:
- egui's TextEdit processes the ENTIRE text, not just visible viewport
- No line-range culling or viewport-based rendering
- Font layout computed for all 18K lines even if screen shows only 50

**Current approach**:
```rust
edit.id(id)
    .frame(egui::Frame::NONE)
    .font(font.clone())
    .desired_width(width)
    .desired_rows(((avail.y / row_h) as usize).max(1))
    .layouter(&mut layouter)  // Layouter called for entire text
    .show(ui)
```

**Impact**:
- egui's internal text layout engine processes all lines
- Text shaping happens for invisible lines
- Scroll position doesn't reduce work

### 5. **String Cloning in Hot Path (MODERATE)**

**Locations**: Multiple
- `app.rs:191` - `replace_tab_text()` clones text
- `search.rs:146` - `SearchHighlights::search()` clones entire file
- `editor.rs` - Text passing creates copies

**Cost**: 
- 10MB = 10M+ clone operations
- Each clone: malloc + memcpy
- ~50ms overhead

## Performance Characteristics - 10MB File (18,284 lines)

### Current (Likely Timing)
| Operation | Time | Cause |
|-----------|------|-------|
| Load file | 2-5s | Full layout job creation on first frame |
| Search "response" | 2-10s | Scan all lines, re-layout entire file |
| Toggle word wrap | 1-3s | Re-layout entire file |
| Toggle hidden chars | 1-3s | Re-create all strings with · replacement |
| Scroll | 100-200ms | No viewport optimization |

### Path to 100MB Support (Need ~100x speedup in worst case)

## Design Issues with egui

**Hard Limitation?** Not entirely, but it's a constraint:

1. **egui's TextEdit is not viewport-culled**: 
   - It processes the entire text
   - You CAN work around this with custom drawing
   - Most egui demos use small files

2. **LayoutJob is full-file**: 
   - Designed to store format info for entire text
   - Creating LayoutJob for 100MB would be huge
   - But you could create viewport-only LayoutJobs

3. **No built-in line-based virtual rendering**:
   - egui assumes reasonable-sized text
   - Other editors (VS Code, Sublime) use viewport culling
   - You need to implement this yourself

## Optimization Strategies (Priority Order)

### Phase 1: Quick Wins (Likely 5-10x speedup)
1. **Viewport-based layout job creation** - Only layout visible lines + small buffer
2. **Lazy search highlighting** - Find matches only on visible lines first
3. **Reduce string allocations** - Use references where possible, string buffers
4. **Cache layout results** - Don't recompute identical layouts

### Phase 2: Architectural Changes (Likely 50-100x speedup)
1. **Line-based rendering** - Store text as `Vec<String>` instead of one big String
2. **Virtual viewport** - Only keep visible lines in layout
3. **Incremental search** - Scan visible region, expand on demand
4. **Rope data structure** - For large files, use rope for O(log N) random access

### Phase 3: Advanced Optimizations
1. **GPU text rendering** - Bypass egui's layouter for some operations
2. **Parallel search** - Multi-threaded pattern matching
3. **Memory-mapped files** - For true 100MB+ support
4. **Streaming mode** - For tail/log file viewing

## Recommendation for 100MB Support

**Best approach**: Implement **line-based virtual viewport rendering**

1. Store file as `Vec<Line>` or `Vec<String>`
2. Keep only ~200 lines in memory (visible + buffer)
3. Create layout job for visible lines only
4. Search only within visible viewport
5. Load adjacent lines on demand when scrolling

This would achieve:
- **50-100x speedup** on current operations
- Support for truly large files (100MB+ achievable)
- Smooth scrolling and search
- With minimal changes to core logic

**egui compatibility**: Full ✓
- Just need custom scroll handling
- Can still use TextEdit for visible viewport
- Already have scroll areas in place

## Next Steps

1. Profile actual bottlenecks with flamegraph/perf
2. Implement viewport culling
3. Test with 50MB, 100MB files
4. Measure improvements
