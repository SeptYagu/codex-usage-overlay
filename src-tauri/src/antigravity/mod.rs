//! Antigravity subscription limits, read live via `agy -p "/usage"` or from
//! the snapshot captured by the `antigravity-usage-bridge` statusline helper.
pub mod install;
pub mod live;
pub use agent_bridge::antigravity as shared;
pub mod source;
