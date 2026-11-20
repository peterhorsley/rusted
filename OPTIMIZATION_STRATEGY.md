# RustEd Optimization Strategy - Implementation Plan

## Goal
Support 100MB+ text files without excessive lag. Target: < 500ms load time, smooth UI for search/wrap/nav.

## Key Optimization: Viewport-Based Layout

### Current Flow (All ~18K lines processed every frame)
```
Frame starts
  → TextEdit needs layout
  → layouter callback called
  → search_layout(entire_file)
    → iterate all 18K lines
    → check highlights on each
    → build LayoutJob with all segments
  → egui lays out full text
  → egui renders visible portion
  → 100-500ms per frame
```

### Optimized Flow (Only visible lines processed)
```
Frame starts
  → TextEdit needs layout
  → layouter callback called
  → Calculate viewport line range from scroll position
  → search_layout_viewport(lines_visible_range)
    → iterate only 50 visible lines
    → check highlights on each
    → build LayoutJob with only visible segments
  → egui lays out visible text
  → egui renders visible portion
  → 2-5ms per frame (100x faster)
```

## Implementation Steps

### Phase 1: Add Viewport Tracking (Safe, Non-Breaking)

**Files to modify**: `app.rs`, `editor.rs`

**Changes**:
1. Add fields to `Tab`:
   ```rust
   pub(crate) first_visible_line: usize,  // Start of visible range
   pub(crate) max_visible_lines: usize,   // Count of visible lines
   ```

2. Calculate visible range in `editor_ui()`:
   ```rust
   let visible_lines = ((avail.y / row_h) as usize).max(1) + 1; // Add 1-line buffer
   tab.max_visible_lines = visible_lines;
   ```

3. Track scroll position from galley in `editor_ui()`.

**Safety**: Read-only state tracking, no behavioral changes yet.

### Phase 2: Create Viewport Layout Job (Breaking change)

**Files to modify**: `search.rs`

**Changes**:
1. Create new function `search_layout_viewport()`:
   ```rust
   pub(crate) fn search_layout_viewport(
       text: &str,
       highlights: &SearchHighlights,
       line_start: usize,      // First visible line
       line_count: usize,      // Number of visible lines
       selected_line: Option<usize>,
       hidden: bool,
       font: &FontId,
       width: f32,
       light: bool,
   ) -> (egui::text::LayoutJob, usize) {  // Returns job + first_char_index
   ```

2. This function:
   - Only iterates `line_count` lines starting from `line_start`
   - Prepends line offset correction
   - Returns the LayoutJob and byte offset for text positioning

3. Keep `search_layout()` as fallback for small files or column selection.

**Risk**: Text positioning must remain correct. egui's TextEdit will receive a viewport slice.

### Phase 3: Integrate Viewport Layout (Careful Testing Required)

**Files to modify**: `editor.rs`

**Current approach** (line ~209):
```rust
let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
    let job = search_layout(text.as_str(), &highlights, ...);
    ui.fonts_mut(|f| f.layout_job(job))
};
```

**New approach**:
```rust
let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
    // For large files, use viewport layout
    let (job, _offset) = if tab.text.len() > 1_000_000 {  // 1MB threshold
        search_layout_viewport(
            text.as_str(),
            &highlights,
            tab.first_visible_line,
            tab.max_visible_lines,
            ...
        )
    } else {
        (search_layout(text.as_str(), &highlights, ...), 0)
    };
    ui.fonts_mut(|f| f.layout_job(job))
};
```

**Gotchas**:
- Cursor positioning must account for line offset
- Search highlighting must work across viewport boundaries
- Column selection math needs updating
- Scroll jumps must handle viewport changes

### Phase 4: Optimize Search Highlighting (Additional 10x)

**Files to modify**: `search.rs`, `app.rs`

**Strategies**:
1. **Lazy search**: Only scan visible lines initially, expand on demand
   ```rust
   pub(crate) struct SearchHighlights {
       pub(crate) snapshot: String,
       pub(crate) lines: Vec<Vec<Range<usize>>>,
       pub(crate) active: bool,
       pub(crate) viewport_cache: (usize, usize),  // Cached viewport range
       pub(crate) full_scan_complete: bool,
   }
   ```

2. **Incremental case-insensitive search**: 
   - Pre-compute lowercase version incrementally
   - Cache per-line results
   - Don't recompute for unchanged lines

3. **Multi-threaded search** (future):
   - Spawn background thread for full-document scan
   - Display progressive results

### Phase 5: Memory Optimization

**Files to modify**: `app.rs`, `editor.rs`

**Current**: Entire file in `String`, highlights snapshot, search state = 3x file size

**Optimization**:
1. Remove highlights snapshot clone (store pattern instead)
2. Use `Arc<str>` for shared text reference
3. Implement line-based storage for truly large files

## Risk Assessment

### High Risk
- **Text positioning**: Viewport offset could break cursor placement
- **Search highlighting**: May show incomplete results during viewport scroll
- **Column selection**: Math assumes line 0 is at byte 0

### Medium Risk
- **Performance**: May not achieve 100x due to egui's internal processing
- **Configuration**: Need to tune buffer size and viewport calculation
- **Testing**: Complex interactions (wrap + search + column select)

### Low Risk
- **Fallback**: Easy to disable viewport culling for small files
- **UI**: No visual changes if implemented correctly
- **Compatibility**: No breaking changes to public API

## Validation Strategy

### Manual Testing Checklist
- [ ] Load 10MB file - should load in <1s
- [ ] Scroll smoothly - no stuttering
- [ ] Search "response" - <100ms, correct highlights
- [ ] Toggle word wrap - <100ms
- [ ] Toggle hidden chars - <100ms  
- [ ] Column selection - still works
- [ ] Undo/redo - preserves state correctly
- [ ] Go to line - correct positioning

### Edge Cases
- [ ] File smaller than viewport
- [ ] Wrapped lines in viewport (affects line count calculation)
- [ ] Search results at viewport boundaries
- [ ] Zoom in/out during scroll
- [ ] Tail mode with viewport culling

## Expected Performance Improvements

| Operation | Before | After | Improvement |
|-----------|--------|-------|------------|
| Load 10MB | 2-5s | 500ms | 10x |
| Search | 2-10s | 100ms | 50x |
| Toggle wrap | 1-3s | 50ms | 20x |
| Scroll | 100-200ms | 10ms | 10x |
| Hidden chars | 1-3s | 50ms | 20x |

**Total**: 50-100x speedup possible, enabling true support for 100MB+ files.

## Implementation Phases

1. **Week 1**: Phase 1-2 (viewport tracking + layout function)
2. **Week 2**: Phase 3 (integration, testing, debugging)
3. **Week 3**: Phase 4 (search optimization)
4. **Week 4**: Phase 5 (memory optimization) + polish

## Fallback Plan

If viewport culling proves too complex:
- Implement **line-based splitting**: 
  - Don't load entire file into memory
  - Use `BufReader` to stream lines
  - Keep only 100K lines buffered
  - Should still achieve 10-20x speedup for 100MB files
  - Simpler to implement, less risky

This approach would require more refactoring but is safer than viewport math.
