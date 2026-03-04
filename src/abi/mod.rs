//! ABI-related definitions.
//!
//! This module contains primitive C ABI type aliases that are used across
//! exported `extern "C"` interfaces, plus Linux errno constants for the
//! primary target.

pub mod errno;
pub mod types;
