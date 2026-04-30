//! Plugin context and message sender.

use std::ffi::CString;
use std::os::raw::c_void;

use crate::types::{FfiJsonCallback, PluginMessage};

/// Opaque context given to plugins at initialization.
///
/// Wraps the host's message callback so channel adapters can send
/// inbound messages to the host.
#[derive(Clone)]
pub struct PluginContext {
    callback: FfiJsonCallback,
    user_data: *mut c_void,
}

// SAFETY: The host guarantees single-threaded access to plugin callbacks.
unsafe impl Send for PluginContext {}
unsafe impl Sync for PluginContext {}

impl PluginContext {
    /// Create a new context from the host-provided callback.
    ///
    /// Called internally by `declare_plugin!`. Plugin developers should not call this directly.
    #[doc(hidden)]
    pub fn new(callback: FfiJsonCallback, user_data: *mut c_void) -> Self {
        Self { callback, user_data }
    }

    /// Create a message sender from this context.
    pub fn sender(&self) -> MessageSender {
        MessageSender {
            callback: self.callback,
            user_data: self.user_data,
        }
    }
}

/// Used by channel adapters to send inbound messages to the host.
///
/// Clone this and store it in your channel adapter when `start()` is called.
#[derive(Clone)]
pub struct MessageSender {
    callback: FfiJsonCallback,
    user_data: *mut c_void,
}

unsafe impl Send for MessageSender {}
unsafe impl Sync for MessageSender {}

impl MessageSender {
    /// Send a message to the host via the FFI callback.
    ///
    /// Serializes the message to JSON and passes it as a C string.
    /// The host copies the data inside the callback, so it is safe
    /// to drop the message after this returns.
    pub fn send(&self, msg: PluginMessage) {
        let json = serde_json::to_string(&msg).unwrap_or_default();
        let c_str = CString::new(json).unwrap_or_default();
        unsafe {
            (self.callback)(self.user_data, c_str.as_ptr());
        }
    }
}
