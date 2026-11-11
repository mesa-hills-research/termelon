//! A terminal UI framework in pure Rust. It owns the whole path from the terminal's modes to the
//! bytes of each frame, which means raw mode, input decoding, a cell buffer, text and widgets, diffed
//! output in one synchronized write per frame, and capability detection with fallbacks.
