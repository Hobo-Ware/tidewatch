//! Core logic for Tidewatch: worktree layout, profiles and agent status. No UI.

pub mod agents;
pub mod delivery;
pub mod events;
mod fsutil;
pub mod history;
pub mod house_style;
pub mod launch;
pub mod listener;
pub mod mcp;
pub mod names;
pub mod notes;
pub mod path_env;
pub mod paths;
pub mod prepare;
pub mod profiles;
pub mod prompt;
mod registry;
pub mod runner;
pub mod status;
pub mod test_command;
pub mod tools;
pub mod workflows;
pub mod worktree;
