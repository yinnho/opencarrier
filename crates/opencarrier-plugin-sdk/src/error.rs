//! Plugin error types.

use std::fmt;

/// Error type returned by plugin trait methods.
pub struct PluginError {
    message: String,
}

impl PluginError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self::new(msg)
    }

    pub fn channel(msg: impl Into<String>) -> Self {
        Self::new(msg)
    }

    pub fn tool(msg: impl Into<String>) -> Self {
        Self::new(msg)
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl fmt::Debug for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PluginError({:?})", self.message)
    }
}

impl std::error::Error for PluginError {}

impl From<String> for PluginError {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for PluginError {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}
