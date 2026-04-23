//! Plugin context and message sender.

use std::ffi::CString;
use std::os::raw::c_void;

use opencarrier_types::plugin::{
    FfiContent, FfiContentType, FfiMessage, FfiMessageCallback, PluginContent, PluginMessage,
};

/// Opaque context given to plugins at initialization.
///
/// Wraps the host's message callback so channel adapters can send
/// inbound messages to the host.
#[derive(Clone)]
pub struct PluginContext {
    callback: FfiMessageCallback,
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
    pub fn new(callback: FfiMessageCallback, user_data: *mut c_void) -> Self {
        Self {
            callback,
            user_data,
        }
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
    callback: FfiMessageCallback,
    user_data: *mut c_void,
}

unsafe impl Send for MessageSender {}
unsafe impl Sync for MessageSender {}

impl MessageSender {
    /// Send a message to the host via the FFI callback.
    ///
    /// The message data is copied by the host inside the callback,
    /// so it is safe to drop the message after this returns.
    pub fn send(&self, msg: PluginMessage) {
        let holder = FfiMessageHolder::from_message(msg);
        unsafe {
            (self.callback)(self.user_data, &holder.ffi_message);
        }
    }
}

// ---------------------------------------------------------------------------
// FfiMessageHolder — keeps CString allocations alive during FFI call
// ---------------------------------------------------------------------------

/// Owns all CString allocations for a single FfiMessage.
///
/// The `FfiMessage` struct contains raw `*const c_char` pointers that must
/// remain valid for the duration of the host callback. This struct owns all
/// those CStrings and the `FfiMessage` borrows from them.
pub(crate) struct FfiMessageHolder {
    channel_type: CString,
    platform_message_id: CString,
    sender_id: CString,
    sender_name: CString,
    tenant_id: CString,
    // Content — only the variant in use is populated
    text: Option<CString>,
    image_url: Option<CString>,
    image_caption: Option<CString>,
    file_url: Option<CString>,
    file_name: Option<CString>,
    voice_url: Option<CString>,
    command_name: Option<CString>,
    command_args_json: Option<CString>,
    // Optional fields
    thread_id: Option<CString>,
    metadata_json: CString,
    // The FFI message (borrows from above CStrings)
    ffi_message: FfiMessage,
    ffi_content: FfiContent,
}

impl FfiMessageHolder {
    pub(crate) fn from_message(msg: PluginMessage) -> Self {
        let channel_type = CString::new(msg.channel_type).unwrap_or_default();
        let platform_message_id = CString::new(msg.platform_message_id).unwrap_or_default();
        let sender_id = CString::new(msg.sender_id).unwrap_or_default();
        let sender_name = CString::new(msg.sender_name).unwrap_or_default();
        let tenant_id = CString::new(msg.tenant_id).unwrap_or_default();
        let thread_id = msg
            .thread_id
            .map(|t| CString::new(t).unwrap_or_default());
        let metadata_json =
            CString::new(serde_json::to_string(&msg.metadata).unwrap_or_default())
                .unwrap_or_default();

        let mut holder = FfiMessageHolder {
            channel_type,
            platform_message_id,
            sender_id,
            sender_name,
            tenant_id,
            text: None,
            image_url: None,
            image_caption: None,
            file_url: None,
            file_name: None,
            voice_url: None,
            command_name: None,
            command_args_json: None,
            thread_id,
            metadata_json,
            ffi_content: FfiContent {
                type_tag: FfiContentType::Text,
                text: std::ptr::null(),
                image_url: std::ptr::null(),
                image_caption: std::ptr::null(),
                file_url: std::ptr::null(),
                file_name: std::ptr::null(),
                voice_url: std::ptr::null(),
                voice_duration_secs: 0,
                location_lat: 0.0,
                location_lon: 0.0,
                command_name: std::ptr::null(),
                command_args_json: std::ptr::null(),
            },
            ffi_message: unsafe { std::mem::zeroed() },
        };

        match msg.content {
            PluginContent::Text(t) => {
                holder.text = Some(CString::new(t).unwrap_or_default());
                holder.ffi_content.type_tag = FfiContentType::Text;
                holder.ffi_content.text = holder.text.as_ref().unwrap().as_ptr();
            }
            PluginContent::Image { url, caption } => {
                holder.image_url = Some(CString::new(url).unwrap_or_default());
                holder.image_caption = caption.map(|c| CString::new(c).unwrap_or_default());
                holder.ffi_content.type_tag = FfiContentType::Image;
                holder.ffi_content.image_url = holder.image_url.as_ref().unwrap().as_ptr();
                holder.ffi_content.image_caption = holder
                    .image_caption
                    .as_ref()
                    .map(|c| c.as_ptr())
                    .unwrap_or(std::ptr::null());
            }
            PluginContent::File { url, filename } => {
                holder.file_url = Some(CString::new(url).unwrap_or_default());
                holder.file_name = Some(CString::new(filename).unwrap_or_default());
                holder.ffi_content.type_tag = FfiContentType::File;
                holder.ffi_content.file_url = holder.file_url.as_ref().unwrap().as_ptr();
                holder.ffi_content.file_name = holder.file_name.as_ref().unwrap().as_ptr();
            }
            PluginContent::Voice {
                url,
                duration_seconds,
            } => {
                holder.voice_url = Some(CString::new(url).unwrap_or_default());
                holder.ffi_content.type_tag = FfiContentType::Voice;
                holder.ffi_content.voice_url = holder.voice_url.as_ref().unwrap().as_ptr();
                holder.ffi_content.voice_duration_secs = duration_seconds;
            }
            PluginContent::Location { lat, lon } => {
                holder.ffi_content.type_tag = FfiContentType::Location;
                holder.ffi_content.location_lat = lat;
                holder.ffi_content.location_lon = lon;
            }
            PluginContent::Command { name, args } => {
                holder.command_name = Some(CString::new(name).unwrap_or_default());
                let args_json =
                    serde_json::to_string(&args).unwrap_or_else(|_| "[]".to_string());
                holder.command_args_json = Some(CString::new(args_json).unwrap_or_default());
                holder.ffi_content.type_tag = FfiContentType::Command;
                holder.ffi_content.command_name = holder.command_name.as_ref().unwrap().as_ptr();
                holder.ffi_content.command_args_json =
                    holder.command_args_json.as_ref().unwrap().as_ptr();
            }
        }

        holder.ffi_message = FfiMessage {
            channel_type: holder.channel_type.as_ptr(),
            platform_message_id: holder.platform_message_id.as_ptr(),
            sender_id: holder.sender_id.as_ptr(),
            sender_name: holder.sender_name.as_ptr(),
            tenant_id: holder.tenant_id.as_ptr(),
            content: holder.ffi_content.clone(),
            timestamp_ms: msg.timestamp_ms,
            is_group: if msg.is_group { 1 } else { 0 },
            thread_id: holder
                .thread_id
                .as_ref()
                .map(|t| t.as_ptr())
                .unwrap_or(std::ptr::null()),
            metadata_json: holder.metadata_json.as_ptr(),
        };

        holder
    }
}
