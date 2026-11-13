//! Antigravity subscription limits, read from the snapshot the
//! `antigravity-usage-bridge` helper stores while Antigravity CLI renders its statusline.
pub mod install;
// The statusline parsing, pool resolution and guard decision are used by the
// helper binary, which includes `shared.rs` directly.
#[allow(dead_code)]
pub mod shared;
pub mod source;
