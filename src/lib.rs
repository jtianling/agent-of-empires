//! Agent of Empires library - Core functionality for the terminal session manager

pub mod agents;
pub mod cli;
mod codex_permissions;
mod codex_xats;
pub mod containers;
pub mod db;
pub mod git;
pub mod hooks;
pub mod kimi;
pub mod kimi_xats;
mod managed_codex;
pub mod managed_runtimes;
pub mod managed_xats;
pub mod migrations;
pub mod opencode_runtime;
pub mod opencode_xats;
pub mod process;
pub mod session;
pub mod sound;
pub mod terminal;
pub mod tmux;
pub mod tui;
pub mod update;
pub mod xats_control;
pub mod xats_identity;
