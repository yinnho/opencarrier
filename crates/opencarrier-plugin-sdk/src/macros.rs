//! The `declare_plugin!` macro — auto-generates all C ABI exports.
//!
//! Plugin developers call `declare_plugin!(MyPluginType)` at the bottom of
//! their `lib.rs` to generate the 12 `oc_*` exported functions.

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
            channel_type_c: CString,
            name_c: CString,
        }

        struct _OcToolEntry {
            provider: Box<dyn $crate::ToolProvider>,
            name_c: CString,
            description_c: CString,
            parameters_json_c: CString,
        }

        struct _OcState {
            #[allow(dead_code)]
            plugin: $plugin_type,
            channels: Vec<_OcChannelEntry>,
            tools: Vec<_OcToolEntry>,
            context: $crate::PluginContext,
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
            opencarrier_types::plugin::PLUGIN_ABI_VERSION
        }

        // ------------------------------------------------------------------
        // 4. oc_plugin_init
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_plugin_init(
            config_json: *const c_char,
            message_cb: opencarrier_types::plugin::FfiMessageCallback,
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

            let config: opencarrier_types::plugin::PluginConfig =
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
            for adapter in channel_adapters {
                let channel_type_c = CString::new(adapter.channel_type()).unwrap_or_default();
                let name_c = CString::new(adapter.name()).unwrap_or_default();
                channels.push(_OcChannelEntry {
                    adapter,
                    channel_type_c,
                    name_c,
                });
            }

            // Collect tools
            let tool_providers = <$plugin_type as $crate::Plugin>::tools(&plugin);
            let mut tools = Vec::with_capacity(tool_providers.len());
            for provider in tool_providers {
                let def = provider.definition();
                let name_c = CString::new(def.name).unwrap_or_default();
                let description_c = CString::new(def.description).unwrap_or_default();
                let parameters_json_c = CString::new(def.parameters_json).unwrap_or_default();
                tools.push(_OcToolEntry {
                    provider,
                    name_c,
                    description_c,
                    parameters_json_c,
                });
            }

            let state = Box::new(_OcState {
                plugin,
                channels,
                tools,
                context: ctx,
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
                // state dropped here
                _OC_STATE = ptr::null_mut();
            }
        }

        // ------------------------------------------------------------------
        // 6. oc_plugin_channels
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_plugin_channels(
            handle: *mut c_void,
            out_channels: *mut *mut opencarrier_types::plugin::FfiChannelInfo,
        ) -> u32 {
            let state = unsafe {
                if handle.is_null() {
                    return 0;
                }
                &*(handle as *const _OcState)
            };

            let count = state.channels.len();
            if count == 0 || out_channels.is_null() {
                return 0;
            }

            let mut infos: Vec<opencarrier_types::plugin::FfiChannelInfo> =
                Vec::with_capacity(count);
            for (i, entry) in state.channels.iter().enumerate() {
                let ch_handle = (i + 1) as *mut c_void;
                infos.push(opencarrier_types::plugin::FfiChannelInfo {
                    channel_type: entry.channel_type_c.as_ptr(),
                    name: entry.name_c.as_ptr(),
                    handle: ch_handle,
                });
            }

            unsafe {
                *out_channels = infos.as_mut_ptr() as *mut _;
                std::mem::forget(infos);
            }

            count as u32
        }

        // ------------------------------------------------------------------
        // 7. oc_plugin_free_channels
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_plugin_free_channels(
            ptr: *mut opencarrier_types::plugin::FfiChannelInfo,
            count: u32,
        ) {
            if ptr.is_null() || count == 0 {
                return;
            }
            unsafe {
                let _ = Vec::from_raw_parts(ptr, count as usize, count as usize);
            }
        }

        // ------------------------------------------------------------------
        // 8. oc_plugin_tools
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_plugin_tools(
            handle: *mut c_void,
            out_tools: *mut *mut opencarrier_types::plugin::FfiToolDef,
        ) -> u32 {
            let state = unsafe {
                if handle.is_null() {
                    return 0;
                }
                &*(handle as *const _OcState)
            };

            let count = state.tools.len();
            if count == 0 || out_tools.is_null() {
                return 0;
            }

            let mut defs: Vec<opencarrier_types::plugin::FfiToolDef> = Vec::with_capacity(count);
            for entry in &state.tools {
                defs.push(opencarrier_types::plugin::FfiToolDef {
                    name: entry.name_c.as_ptr(),
                    description: entry.description_c.as_ptr(),
                    parameters_json: entry.parameters_json_c.as_ptr(),
                });
            }

            unsafe {
                *out_tools = defs.as_mut_ptr() as *mut _;
                std::mem::forget(defs);
            }

            count as u32
        }

        // ------------------------------------------------------------------
        // 9. oc_plugin_free_tools
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_plugin_free_tools(
            ptr: *mut opencarrier_types::plugin::FfiToolDef,
            count: u32,
        ) {
            if ptr.is_null() || count == 0 {
                return;
            }
            unsafe {
                let _ = Vec::from_raw_parts(ptr, count as usize, count as usize);
            }
        }

        // ------------------------------------------------------------------
        // 10. oc_channel_start
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
        // 11. oc_channel_send
        // ------------------------------------------------------------------
        #[no_mangle]
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn oc_channel_send(
            channel_handle: *mut c_void,
            tenant_id: *const c_char,
            user_id: *const c_char,
            text: *const c_char,
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

            let tenant = unsafe { CStr::from_ptr(tenant_id).to_string_lossy() };
            let user = unsafe { CStr::from_ptr(user_id).to_string_lossy() };
            let text_str = unsafe { CStr::from_ptr(text).to_string_lossy() };

            match state.channels[idx - 1].adapter.send(&tenant, &user, &text_str) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        }

        // ------------------------------------------------------------------
        // 12. oc_plugin_tool_execute
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

            let context: opencarrier_types::plugin::PluginToolContext = unsafe {
                let s = CStr::from_ptr(context_json).to_string_lossy();
                serde_json::from_str(&s).unwrap_or_default()
            };

            let entry = state.tools.iter().find(|e| {
                e.name_c.to_str().unwrap_or("") == tool_name_str
            });

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
