//! Antigravity subscription limits, read live via `agy -p "/usage"` or from
//! the snapshot captured by the `antigravity-usage-bridge` statusline helper.
pub mod install;
pub mod live;
// The statusline parsing, pool resolution and guard decision are used by the
// helper binary, which includes `shared.rs` directly.
#[allow(dead_code)]
pub mod shared;
pub mod source;
