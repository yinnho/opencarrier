//! Plugin loader — dlopen-based shared library loading.
//!
//! Scans the plugins directory, loads shared libraries, binds C ABI symbols,
//! and initializes plugin instances.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};

use libloading::Library;
use opencarrier_types::plugin::{
    FfiChannelInfo, FfiMessage, FfiMessageCallback, FfiToolDef, PluginConfig, PluginToolDef,
    PLUGIN_ABI_VERSION,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Maximum size for tool execution result buffer.
const TOOL_RESULT_BUF_SIZE: u32 = 64 * 1024;

// ---------------------------------------------------------------------------
// C ABI function pointer types
// ---------------------------------------------------------------------------

type FnPluginName = unsafe extern "C" fn() -> *const c_char;
type FnPluginVersion = unsafe extern "C" fn() -> *const c_char;
type FnPluginAbiVersion = unsafe extern "C" fn() -> u32;
type FnPluginInit = unsafe extern "C" fn(
    config_json: *const c_char,
    message_cb: FfiMessageCallback,
    user_data: *mut std::os::raw::c_void,
) -> *mut std::os::raw::c_void;
type FnPluginStop = unsafe extern "C" fn(handle: *mut std::os::raw::c_void);
type FnPluginChannels = unsafe extern "C" fn(
    handle: *mut std::os::raw::c_void,
    out_channels: *mut *mut FfiChannelInfo,
) -> u32;
type FnPluginFreeChannels = unsafe extern "C" fn(ptr: *mut FfiChannelInfo, count: u32);
type FnPluginTools = unsafe extern "C" fn(
    handle: *mut std::os::raw::c_void,
    out_tools: *mut *mut FfiToolDef,
) -> u32;
type FnPluginFreeTools = unsafe extern "C" fn(ptr: *mut FfiToolDef, count: u32);
type FnChannelStart =
    unsafe extern "C" fn(channel_handle: *mut std::os::raw::c_void) -> i32;
type FnChannelSend = unsafe extern "C" fn(
    channel_handle: *mut std::os::raw::c_void,
    tenant_id: *const c_char,
    user_id: *const c_char,
    text: *const c_char,
) -> i32;
type FnToolExecute = unsafe extern "C" fn(
    plugin_handle: *mut std::os::raw::c_void,
    tool_name: *const c_char,
    args_json: *const c_char,
    context_json: *const c_char,
    result_buf: *mut c_char,
    result_buf_len: u32,
) -> i32;

// ---------------------------------------------------------------------------
// Loaded plugin
// ---------------------------------------------------------------------------

/// A successfully loaded plugin with its library, symbols, and metadata.
pub struct LoadedPlugin {
    /// Plugin name.
    pub name: String,
    /// Plugin version.
    pub version: String,
    /// Plugin directory path.
    pub path: PathBuf,
    /// Opaque handle returned by `oc_plugin_init`.
    pub handle: *mut std::os::raw::c_void,
    /// Loaded channels with their opaque handles.
    pub channels: Vec<LoadedChannel>,
    /// Tool definitions provided by this plugin.
    pub tools: Vec<PluginToolDef>,
    /// The loaded shared library (kept alive for symbol resolution).
    _library: Library,
    // Cached function pointers
    fn_stop: Option<FnPluginStop>,
    fn_channel_start: Option<FnChannelStart>,
    fn_channel_send: Option<FnChannelSend>,
    fn_tool_execute: Option<FnToolExecute>,
}

/// A loaded channel from a plugin.
#[derive(Clone)]
pub struct LoadedChannel {
    /// Channel type identifier (e.g. "wecom").
    pub channel_type: String,
    /// Human-readable name.
    pub name: String,
    /// Opaque channel handle from the plugin.
    pub handle: *mut std::os::raw::c_void,
}

// SAFETY: LoadedChannel contains an opaque pointer from the plugin.
// It is only used to pass back to the plugin's own functions.
unsafe impl Send for LoadedChannel {}
unsafe impl Sync for LoadedChannel {}

// SAFETY: LoadedPlugin owns the Library and all handles are from that library.
unsafe impl Send for LoadedPlugin {}
unsafe impl Sync for LoadedPlugin {}

impl LoadedPlugin {
    /// Start a channel (begin receiving messages).
    pub fn start_channel(&self, channel: &LoadedChannel) -> Result<(), String> {
        if let Some(fn_start) = self.fn_channel_start {
            let ret = unsafe { fn_start(channel.handle) };
            if ret != 0 {
                return Err(format!(
                    "Channel {} start returned error code {}",
                    channel.channel_type, ret
                ));
            }
            Ok(())
        } else {
            Err("Plugin does not export oc_channel_start".to_string())
        }
    }

    /// Send a text message through a channel.
    pub fn channel_send(
        &self,
        channel: &LoadedChannel,
        tenant_id: &str,
        user_id: &str,
        text: &str,
    ) -> Result<(), String> {
        if let Some(fn_send) = self.fn_channel_send {
            let c_tenant = CString::new(tenant_id).map_err(|e| e.to_string())?;
            let c_user = CString::new(user_id).map_err(|e| e.to_string())?;
            let c_text = CString::new(text).map_err(|e| e.to_string())?;
            let ret = unsafe {
                fn_send(
                    channel.handle,
                    c_tenant.as_ptr(),
                    c_user.as_ptr(),
                    c_text.as_ptr(),
                )
            };
            if ret != 0 {
                return Err(format!(
                    "Channel {} send returned error code {}",
                    channel.channel_type, ret
                ));
            }
            Ok(())
        } else {
            Err("Plugin does not export oc_channel_send".to_string())
        }
    }

    /// Execute a tool.
    pub fn tool_execute(
        &self,
        tool_name: &str,
        args_json: &str,
        context_json: &str,
    ) -> Result<String, String> {
        if let Some(fn_exec) = self.fn_tool_execute {
            let c_tool = CString::new(tool_name).map_err(|e| e.to_string())?;
            let c_args = CString::new(args_json).map_err(|e| e.to_string())?;
            let c_ctx = CString::new(context_json).map_err(|e| e.to_string())?;

            let mut buf = vec![0u8; TOOL_RESULT_BUF_SIZE as usize];
            let ret = unsafe {
                fn_exec(
                    self.handle,
                    c_tool.as_ptr(),
                    c_args.as_ptr(),
                    c_ctx.as_ptr(),
                    buf.as_mut_ptr() as *mut c_char,
                    TOOL_RESULT_BUF_SIZE,
                )
            };

            if ret < 0 {
                // Negative return = error, try to read error message from buf
                let error_msg = unsafe {
                    CStr::from_ptr(buf.as_ptr() as *const c_char)
                        .to_string_lossy()
                        .into_owned()
                };
                return Err(if error_msg.is_empty() {
                    format!("Tool {} returned error code {}", tool_name, ret)
                } else {
                    error_msg
                });
            }

            let len = ret as usize;
            if len > buf.len() {
                return Err(format!(
                    "Tool {} returned {} bytes, exceeding buffer size {}",
                    tool_name,
                    len,
                    buf.len()
                ));
            }

            let result = String::from_utf8_lossy(&buf[..len]).into_owned();
            Ok(result)
        } else {
            Err("Plugin does not export oc_plugin_tool_execute".to_string())
        }
    }

    /// Stop the plugin and release resources.
    pub fn stop(&self) {
        if let Some(fn_stop) = self.fn_stop {
            if !self.handle.is_null() {
                unsafe { fn_stop(self.handle) };
            }
        }
    }
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Plugin loader
// ---------------------------------------------------------------------------

/// Scans the plugins directory and loads all valid plugins.
pub struct PluginLoader;

impl PluginLoader {
    /// Load all plugins from the given directory.
    ///
    /// Each subdirectory should contain:
    /// - `plugin.toml` (metadata + configuration)
    /// - A shared library (`.so` / `.dylib` / `.dll`)
    pub fn load_all(
        plugins_dir: &Path,
        message_tx: mpsc::Sender<opencarrier_types::plugin::PluginMessage>,
    ) -> Vec<Result<LoadedPlugin, String>> {
        let mut results = Vec::new();

        let entries = match std::fs::read_dir(plugins_dir) {
            Ok(entries) => entries,
            Err(e) => {
                info!("Plugins directory not found or not readable: {}", e);
                return results;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let plugin_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            info!(plugin = %plugin_name, "Loading plugin");

            match Self::load_plugin(&path, &message_tx) {
                Ok(plugin) => {
                    info!(
                        plugin = %plugin.name,
                        version = %plugin.version,
                        channels = plugin.channels.len(),
                        tools = plugin.tools.len(),
                        "Plugin loaded successfully"
                    );
                    results.push(Ok(plugin));
                }
                Err(e) => {
                    warn!(plugin = %plugin_name, error = %e, "Failed to load plugin");
                    results.push(Err(e));
                }
            }
        }

        results
    }

    fn load_plugin(
        plugin_dir: &Path,
        message_tx: &mpsc::Sender<opencarrier_types::plugin::PluginMessage>,
    ) -> Result<LoadedPlugin, String> {
        // 1. Read plugin.toml
        let config_path = plugin_dir.join("plugin.toml");
        let config_content = std::fs::read_to_string(&config_path).map_err(|e| {
            format!(
                "Failed to read {}: {}",
                config_path.display(),
                e
            )
        })?;
        let config: PluginConfig =
            toml::from_str(&config_content).map_err(|e| format!("Invalid plugin.toml: {}", e))?;

        // 2. Check ABI version
        if config.meta.abi_version != 0 && config.meta.abi_version != PLUGIN_ABI_VERSION {
            return Err(format!(
                "ABI version mismatch: plugin expects {}, host provides {}",
                config.meta.abi_version, PLUGIN_ABI_VERSION
            ));
        }

        // 3. Find shared library
        let lib_path = Self::find_shared_library(plugin_dir).ok_or_else(|| {
            format!(
                "No shared library found in {}",
                plugin_dir.display()
            )
        })?;

        // 4. dlopen
        let library = unsafe { Library::new(&lib_path) }
            .map_err(|e| format!("Failed to load {}: {}", lib_path.display(), e))?;

        // 5. Load symbols
        let fn_name: FnPluginName = unsafe {
            *library
                .get(b"oc_plugin_name\0")
                .map_err(|e| format!("Missing oc_plugin_name: {}", e))?
        };
        let fn_version: FnPluginVersion = unsafe {
            *library
                .get(b"oc_plugin_version\0")
                .map_err(|e| format!("Missing oc_plugin_version: {}", e))?
        };
        let fn_abi: Option<FnPluginAbiVersion> = unsafe {
            library.get(b"oc_plugin_abi_version\0").ok().map(|s| *s)
        };
        let fn_init: FnPluginInit = unsafe {
            *library
                .get(b"oc_plugin_init\0")
                .map_err(|e| format!("Missing oc_plugin_init: {}", e))?
        };
        let fn_stop: Option<FnPluginStop> = unsafe {
            library.get(b"oc_plugin_stop\0").ok().map(|s| *s)
        };
        let fn_channels: Option<FnPluginChannels> = unsafe {
            library.get(b"oc_plugin_channels\0").ok().map(|s| *s)
        };
        let fn_free_channels: Option<FnPluginFreeChannels> = unsafe {
            library.get(b"oc_plugin_free_channels\0").ok().map(|s| *s)
        };
        let fn_tools: Option<FnPluginTools> = unsafe {
            library.get(b"oc_plugin_tools\0").ok().map(|s| *s)
        };
        let fn_free_tools: Option<FnPluginFreeTools> = unsafe {
            library.get(b"oc_plugin_free_tools\0").ok().map(|s| *s)
        };
        let fn_channel_start: Option<FnChannelStart> = unsafe {
            library.get(b"oc_channel_start\0").ok().map(|s| *s)
        };
        let fn_channel_send: Option<FnChannelSend> = unsafe {
            library.get(b"oc_channel_send\0").ok().map(|s| *s)
        };
        let fn_tool_execute: Option<FnToolExecute> = unsafe {
            library.get(b"oc_plugin_tool_execute\0").ok().map(|s| *s)
        };

        // 6. Verify ABI version via exported function
        if let Some(fn_abi) = fn_abi {
            let abi = unsafe { fn_abi() };
            if abi != PLUGIN_ABI_VERSION {
                return Err(format!(
                    "ABI version mismatch: plugin reports {}, host expects {}",
                    abi, PLUGIN_ABI_VERSION
                ));
            }
        }

        // 7. Read plugin name/version from library
        let name = unsafe { CStr::from_ptr(fn_name()) }
            .to_string_lossy()
            .into_owned();
        let version = unsafe { CStr::from_ptr(fn_version()) }
            .to_string_lossy()
            .into_owned();

        // 8. Initialize plugin
        let config_json_str =
            serde_json::to_string(&config).map_err(|e| format!("Config serialization: {}", e))?;
        let c_config = CString::new(config_json_str).map_err(|e| e.to_string())?;

        // Create a boxed sender that we pass as user_data
        let tx_box = Box::new(message_tx.clone());
        let user_data = Box::into_raw(tx_box) as *mut std::os::raw::c_void;

        let handle = unsafe { fn_init(c_config.as_ptr(), message_callback, user_data) };
        if handle.is_null() {
            return Err("oc_plugin_init returned null handle".to_string());
        }

        // 9. Load channels
        let channels = if let Some(fn_ch) = fn_channels {
            let mut ptr: *mut FfiChannelInfo = std::ptr::null_mut();
            let count = unsafe { fn_ch(handle, &mut ptr) };
            let loaded = Self::read_channels(ptr, count);
            if let Some(fn_free) = fn_free_channels {
                if !ptr.is_null() && count > 0 {
                    unsafe { fn_free(ptr, count) };
                }
            }
            loaded
        } else {
            Vec::new()
        };

        // 10. Load tools
        let tools = if let Some(fn_t) = fn_tools {
            let mut ptr: *mut FfiToolDef = std::ptr::null_mut();
            let count = unsafe { fn_t(handle, &mut ptr) };
            let loaded = Self::read_tools(ptr, count);
            if let Some(fn_free) = fn_free_tools {
                if !ptr.is_null() && count > 0 {
                    unsafe { fn_free(ptr, count) };
                }
            }
            loaded
        } else {
            Vec::new()
        };

        Ok(LoadedPlugin {
            name,
            version,
            path: plugin_dir.to_path_buf(),
            handle,
            channels,
            tools,
            _library: library,
            fn_stop,
            fn_channel_start,
            fn_channel_send,
            fn_tool_execute,
        })
    }

    /// Find the shared library file in a plugin directory.
    fn find_shared_library(plugin_dir: &Path) -> Option<PathBuf> {
        let extensions = if cfg!(target_os = "macos") {
            &["dylib"][..]
        } else if cfg!(target_os = "linux") {
            &["so"][..]
        } else if cfg!(target_os = "windows") {
            &["dll"][..]
        } else {
            &["so", "dylib", "dll"][..]
        };

        if let Ok(entries) = std::fs::read_dir(plugin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if extensions.contains(&ext) {
                        return Some(path);
                    }
                }
            }
        }

        // Also check for libopencarrier_plugin_*.so pattern in subdirs
        None
    }

    /// Convert FFI channel info array to Rust types.
    fn read_channels(ptr: *mut FfiChannelInfo, count: u32) -> Vec<LoadedChannel> {
        if ptr.is_null() || count == 0 {
            return Vec::new();
        }
        let mut channels = Vec::with_capacity(count as usize);
        let slice = unsafe { std::slice::from_raw_parts(ptr, count as usize) };
        for info in slice {
            let channel_type = unsafe { CStr::from_ptr(info.channel_type) }
                .to_string_lossy()
                .into_owned();
            let name = unsafe { CStr::from_ptr(info.name) }
                .to_string_lossy()
                .into_owned();
            channels.push(LoadedChannel {
                channel_type,
                name,
                handle: info.handle,
            });
        }
        channels
    }

    /// Convert FFI tool definition array to Rust types.
    fn read_tools(ptr: *mut FfiToolDef, count: u32) -> Vec<PluginToolDef> {
        if ptr.is_null() || count == 0 {
            return Vec::new();
        }
        let mut tools = Vec::with_capacity(count as usize);
        let slice = unsafe { std::slice::from_raw_parts(ptr, count as usize) };
        for def in slice {
            let name = unsafe { CStr::from_ptr(def.name) }
                .to_string_lossy()
                .into_owned();
            let description = unsafe { CStr::from_ptr(def.description) }
                .to_string_lossy()
                .into_owned();
            let parameters_json = unsafe { CStr::from_ptr(def.parameters_json) }
                .to_string_lossy()
                .into_owned();
            tools.push(PluginToolDef {
                name,
                description,
                parameters_json,
            });
        }
        tools
    }
}

// ---------------------------------------------------------------------------
// Message callback (C ABI → Rust)
// ---------------------------------------------------------------------------

/// Global callback that plugins call to deliver inbound messages.
///
/// The `user_data` pointer is a `Box<mpsc::Sender<PluginMessage>>` that we
/// leaked in `load_plugin`. This function converts the FFI message to a Rust
/// `PluginMessage` and sends it through the channel.
unsafe extern "C" fn message_callback(
    user_data: *mut std::os::raw::c_void,
    msg: *const FfiMessage,
) {
    if user_data.is_null() || msg.is_null() {
        return;
    }

    let tx = &*(user_data as *const mpsc::Sender<opencarrier_types::plugin::PluginMessage>);

    let ffi_msg = &*msg;

    let content = match ffi_msg.content.type_tag {
        opencarrier_types::plugin::FfiContentType::Text => {
            let text = CStr::from_ptr(ffi_msg.content.text)
                .to_string_lossy()
                .into_owned();
            opencarrier_types::plugin::PluginContent::Text(text)
        }
        opencarrier_types::plugin::FfiContentType::Image => {
            opencarrier_types::plugin::PluginContent::Image {
                url: CStr::from_ptr(ffi_msg.content.image_url)
                    .to_string_lossy()
                    .into_owned(),
                caption: if ffi_msg.content.image_caption.is_null() {
                    None
                } else {
                    Some(
                        CStr::from_ptr(ffi_msg.content.image_caption)
                            .to_string_lossy()
                            .into_owned(),
                    )
                },
            }
        }
        opencarrier_types::plugin::FfiContentType::File => {
            opencarrier_types::plugin::PluginContent::File {
                url: CStr::from_ptr(ffi_msg.content.file_url)
                    .to_string_lossy()
                    .into_owned(),
                filename: CStr::from_ptr(ffi_msg.content.file_name)
                    .to_string_lossy()
                    .into_owned(),
            }
        }
        opencarrier_types::plugin::FfiContentType::Voice => {
            opencarrier_types::plugin::PluginContent::Voice {
                url: CStr::from_ptr(ffi_msg.content.voice_url)
                    .to_string_lossy()
                    .into_owned(),
                duration_seconds: ffi_msg.content.voice_duration_secs,
            }
        }
        opencarrier_types::plugin::FfiContentType::Location => {
            opencarrier_types::plugin::PluginContent::Location {
                lat: ffi_msg.content.location_lat,
                lon: ffi_msg.content.location_lon,
            }
        }
        opencarrier_types::plugin::FfiContentType::Command => {
            let args_json = if ffi_msg.content.command_args_json.is_null() {
                "[]"
            } else {
                CStr::from_ptr(ffi_msg.content.command_args_json)
                    .to_str()
                    .unwrap_or("[]")
            };
            let args: Vec<String> = serde_json::from_str(args_json).unwrap_or_default();
            opencarrier_types::plugin::PluginContent::Command {
                name: CStr::from_ptr(ffi_msg.content.command_name)
                    .to_string_lossy()
                    .into_owned(),
                args,
            }
        }
    };

    let metadata = if ffi_msg.metadata_json.is_null() {
        HashMap::new()
    } else {
        let json_str = CStr::from_ptr(ffi_msg.metadata_json)
            .to_str()
            .unwrap_or("{}");
        serde_json::from_str(json_str).unwrap_or_default()
    };

    let message = opencarrier_types::plugin::PluginMessage {
        channel_type: CStr::from_ptr(ffi_msg.channel_type)
            .to_string_lossy()
            .into_owned(),
        platform_message_id: CStr::from_ptr(ffi_msg.platform_message_id)
            .to_string_lossy()
            .into_owned(),
        sender_id: CStr::from_ptr(ffi_msg.sender_id)
            .to_string_lossy()
            .into_owned(),
        sender_name: CStr::from_ptr(ffi_msg.sender_name)
            .to_string_lossy()
            .into_owned(),
        tenant_id: CStr::from_ptr(ffi_msg.tenant_id)
            .to_string_lossy()
            .into_owned(),
        content,
        timestamp_ms: ffi_msg.timestamp_ms,
        is_group: ffi_msg.is_group != 0,
        thread_id: if ffi_msg.thread_id.is_null() {
            None
        } else {
            Some(
                CStr::from_ptr(ffi_msg.thread_id)
                    .to_string_lossy()
                    .into_owned(),
            )
        },
        metadata,
    };

    // Non-blocking send — drop the message if the channel is full
    let _ = tx.try_send(message);
}
