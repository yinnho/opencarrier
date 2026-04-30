//! The `declare_plugin!` macro — auto-generates all C ABI exports.
//!
//! Plugin developers call `declare_plugin!(MyPluginType)` at the bottom of
//! their `lib.rs` to generate the `oc_*` exported functions.
//!
//! All data crossing the FFI boundary uses JSON C strings for ABI stability.

/// Declare a plugin type, generating all required `#[no_mangle] extern "C"` exports.
///
/// Place this at the bottom of your plugin's `lib.rs`:
///
/// ```ignore
/// declare_plugin!(MyPlugin);
/// ```
#[macro_export]
macro_rules! declare_plugin {
    ($plugin_type:ty) => {
        use std::ffi::{CStr, CString};
        use std::os::raw::{c_char, c_void};
        use std::ptr;
        use std::sync::OnceLock;

        // ------------------------------------------------------------------
        // Internal state types
        // ------------------------------------------------------------------

        struct _OcChannelEntry {
            adapter: Box<dyn $crate::ChannelAdapter>,
        }

        struct _OcToolEntry {
            provider: Box<dyn $crate::ToolProvider>,
            name: String,
        }

        struct _OcState {
            #[allow(dead_code)]
            plugin: $plugin_type,
            channels: Vec<_OcChannelEntry>,
            tools: Vec<_OcToolEntry>,
            context: $crate::PluginContext,
            channels_json: CString,
            tools_json: CString,
        }

        static mut _OC_STATE: *mut _OcState = ptr::null_mut();

        // ------------------------------------------------------------------
        // 1. oc_plugin_name
        // ------------------------------------------------------------------
        #[no_mangle]
        pub extern "C" fn oc_plugin_name() -> *const c_char {
            static NAME: &str = <$plugin_type>::NAME;
            static NAME_C: OnceLock<CString> = OnceLock::new();
            NAME_C
                .get_or_init(|| CString::new(NAME).unwrap_or_default())
                .as_ptr()
        }

        // ------------------------------------------------------------------
        // 2. oc_plugin_version
        // ------------------------------------------------------------------
        #[no_mangle]
        pub extern "C" fn oc_plugin_version() -> *const c_char {
            static VERSION: &str = <$plugin_type>::VERSION;
            static VERSION_C: OnceLock<CString> = OnceLock::new();
            VERSION_C
                .get_or_init(|| CString::new(VERSION).unwrap_or_default())
                .as_ptr()
        }

        // ------------------------------------------------------------------
        // 3. oc_plugin_abi_version
        // ------------------------------------------------------------------
        #[no_mangle]
        pub extern "C" fn oc_plugin_abi_version() -> u32 {
            $crate::PLUGIN_ABI_VERSION
        }

        // ------------------------------------------------------------------
        // 4. oc_plugin_init
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_plugin_init(
            config_json: *const c_char,
            message_cb: $crate::FfiJsonCallback,
            user_data: *mut c_void,
        ) -> *mut c_void {
            if config_json.is_null() {
                return ptr::null_mut();
            }

            let config_str = unsafe {
                CStr::from_ptr(config_json)
                    .to_string_lossy()
                    .into_owned()
            };

            let config: $crate::PluginConfig =
                match serde_json::from_str(&config_str) {
                    Ok(c) => c,
                    Err(_) => return ptr::null_mut(),
                };

            let ctx = $crate::PluginContext::new(message_cb, user_data);

            let plugin: $plugin_type = match <$plugin_type as $crate::Plugin>::new(config, ctx.clone()) {
                Ok(p) => p,
                Err(_) => return ptr::null_mut(),
            };

            // Collect channels
            let channel_adapters = <$plugin_type as $crate::Plugin>::channels(&plugin);
            let mut channels = Vec::with_capacity(channel_adapters.len());
            let mut channel_descs = Vec::with_capacity(channel_adapters.len());
            for adapter in channel_adapters {
                channel_descs.push($crate::ChannelDescriptor {
                    channel_type: adapter.channel_type().to_string(),
                    name: adapter.name().to_string(),
                });
                channels.push(_OcChannelEntry { adapter });
            }

            let channels_json = CString::new(
                serde_json::to_string(&channel_descs).unwrap_or_else(|_| "[]".to_string())
            ).unwrap_or_default();

            // Collect tools
            let tool_providers = <$plugin_type as $crate::Plugin>::tools(&plugin);
            let mut tools = Vec::with_capacity(tool_providers.len());
            let mut tool_defs = Vec::with_capacity(tool_providers.len());
            for provider in tool_providers {
                let def = provider.definition();
                tool_defs.push($crate::PluginToolDef {
                    name: def.name.clone(),
                    description: def.description.clone(),
                    parameters_json: def.parameters_json.clone(),
                });
                tools.push(_OcToolEntry {
                    provider,
                    name: def.name,
                });
            }

            let tools_json = CString::new(
                serde_json::to_string(&tool_defs).unwrap_or_else(|_| "[]".to_string())
            ).unwrap_or_default();

            let state = Box::new(_OcState {
                plugin,
                channels,
                tools,
                context: ctx,
                channels_json,
                tools_json,
            });
            let state_ptr = Box::into_raw(state);

            unsafe { _OC_STATE = state_ptr; }

            state_ptr as *mut c_void
        }

        // ------------------------------------------------------------------
        // 5. oc_plugin_stop
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_plugin_stop(handle: *mut c_void) {
            if handle.is_null() {
                return;
            }
            unsafe {
                let state = Box::from_raw(handle as *mut _OcState);
                <$plugin_type as $crate::Plugin>::stop(&state.plugin);
                _OC_STATE = ptr::null_mut();
            }
        }

        // ------------------------------------------------------------------
        // 6. oc_plugin_channels  (JSON)
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_plugin_channels(handle: *mut c_void) -> *const c_char {
            let state = unsafe {
                if handle.is_null() {
                    return ptr::null();
                }
                &*(handle as *const _OcState)
            };
            state.channels_json.as_ptr()
        }

        // ------------------------------------------------------------------
        // 7. oc_plugin_tools  (JSON)
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_plugin_tools(handle: *mut c_void) -> *const c_char {
            let state = unsafe {
                if handle.is_null() {
                    return ptr::null();
                }
                &*(handle as *const _OcState)
            };
            state.tools_json.as_ptr()
        }

        // ------------------------------------------------------------------
        // 8. oc_channel_start
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_channel_start(channel_handle: *mut c_void) -> i32 {
            let state = unsafe {
                if _OC_STATE.is_null() {
                    return -1;
                }
                &mut *_OC_STATE
            };

            let idx = channel_handle as usize;
            if idx == 0 || idx > state.channels.len() {
                return -1;
            }

            let sender = state.context.sender();
            match state.channels[idx - 1].adapter.start(sender) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        }

        // ------------------------------------------------------------------
        // 9. oc_channel_send  (JSON input)
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_channel_send(
            channel_handle: *mut c_void,
            message_json: *const c_char,
        ) -> i32 {
            let state = unsafe {
                if _OC_STATE.is_null() {
                    return -1;
                }
                &*_OC_STATE
            };

            let idx = channel_handle as usize;
            if idx == 0 || idx > state.channels.len() {
                return -1;
            }

            let msg_str = unsafe {
                CStr::from_ptr(message_json).to_string_lossy()
            };

            let msg: serde_json::Value = match serde_json::from_str(&msg_str) {
                Ok(v) => v,
                Err(_) => return -1,
            };

            let tenant_id = msg["tenant_id"].as_str().unwrap_or("");
            let user_id = msg["user_id"].as_str().unwrap_or("");
            let text = msg["text"].as_str().unwrap_or("");

            match state.channels[idx - 1].adapter.send(tenant_id, user_id, text) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        }

        // ------------------------------------------------------------------
        // 10. oc_plugin_tool_execute
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_plugin_tool_execute(
            handle: *mut c_void,
            tool_name: *const c_char,
            args_json: *const c_char,
            context_json: *const c_char,
            result_buf: *mut c_char,
            result_buf_len: u32,
        ) -> i32 {
            let state = unsafe {
                if handle.is_null() {
                    return -1;
                }
                &*(handle as *const _OcState)
            };

            let tool_name_str = unsafe {
                CStr::from_ptr(tool_name)
                    .to_string_lossy()
                    .into_owned()
            };

            let args: serde_json::Value = unsafe {
                let s = CStr::from_ptr(args_json).to_string_lossy();
                serde_json::from_str(&s).unwrap_or(serde_json::Value::Null)
            };

            let context: $crate::PluginToolContext = unsafe {
                let s = CStr::from_ptr(context_json).to_string_lossy();
                serde_json::from_str(&s).unwrap_or_default()
            };

            let entry = state.tools.iter().find(|e| e.name == tool_name_str);

            let Some(entry) = entry else {
                let err = format!("Unknown tool: {}", tool_name_str);
                let bytes = err.as_bytes();
                let len = bytes.len().min(result_buf_len as usize);
                if len > 0 {
                    unsafe {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), result_buf as *mut u8, len);
                    }
                }
                return -1;
            };

            match entry.provider.execute(&args, &context) {
                Ok(result) => {
                    let bytes = result.as_bytes();
                    let len = bytes.len().min(result_buf_len as usize);
                    if len > 0 {
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                bytes.as_ptr(),
                                result_buf as *mut u8,
                                len,
                            );
                        }
                    }
                    len as i32
                }
                Err(e) => {
                    let err_str = e.to_string();
                    let bytes = err_str.as_bytes();
                    let len = bytes.len().min(result_buf_len as usize);
                    if len > 0 {
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                bytes.as_ptr(),
                                result_buf as *mut u8,
                                len,
                            );
                        }
                    }
                    -1
                }
            }
        }
    };
}
