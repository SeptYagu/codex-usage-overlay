//! Claude subscription limits, read from the snapshot the `claude-usage-bridge`
//! helper stores while Claude Code renders its statusline.
pub mod install;
// The statusline parsing and guard decision are used by the helper binary,
// which includes this file directly; the overlay only reads their results.
#[allow(dead_code)]
pub mod shared;
pub mod source;
