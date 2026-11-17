//! CLAP lifecycle adapter shared by the probe and inspector.
use crate::*;
use clap_sys::{
    audio_buffer::*,
    entry::*,
    events::*,
    ext::{audio_ports::*, latency::*, params::*, render::*, thread_check::*},
    factory::plugin_factory::*,
    host::*,
    plugin::*,
    process::*,
    version::*,
};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    fs,
    path::Path,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    thread,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        "<null>".into()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}
struct HostState {
    main: thread::ThreadId,
    audio: AtomicBool,
    callback: AtomicBool,
    restart: AtomicBool,
}
unsafe fn state<'a>(h: *const clap_host) -> &'a HostState {
    &*((*h).host_data.cast())
}
unsafe extern "C" fn is_main(h: *const clap_host) -> bool {
    thread::current().id() == state(h).main
}
unsafe extern "C" fn is_audio(h: *const clap_host) -> bool {
    is_main(h) && state(h).audio.load(Ordering::Relaxed)
}
unsafe extern "C" fn callback(h: *const clap_host) {
    state(h).callback.store(true, Ordering::Relaxed);
}
unsafe extern "C" fn restart(h: *const clap_host) {
    state(h).restart.store(true, Ordering::Relaxed);
}
unsafe extern "C" fn request_process(_: *const clap_host) {} // Always scheduled by this offline host.
static THREAD_CHECK: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(is_main),
    is_audio_thread: Some(is_audio),
};
unsafe extern "C" fn host_extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    if CStr::from_ptr(id) == CLAP_EXT_THREAD_CHECK {
        (&THREAD_CHECK as *const clap_host_thread_check).cast()
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn event_count(e: *const clap_input_events) -> u32 {
    (!(*e).ctx.is_null()) as u32
}
unsafe extern "C" fn event_get(
    e: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    if index == 0 {
        (*e).ctx.cast()
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn discard_event(
    _: *const clap_output_events,
    _: *const clap_event_header,
) -> bool {
    false
}

pub struct Plugin {
    // Fields remain alive throughout destroy/deinit, including failed initialization.
    _library: libloading::Library,
    entry: *const clap_plugin_entry,
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    state: Box<HostState>,
    active: bool,
    processing: bool,
    pub time: i64,
    params: Vec<clap_param_info>,
}
impl Plugin {
    unsafe fn extension<T>(&self, id: &CStr) -> *const T {
        ((*self.plugin).get_extension.unwrap())(self.plugin, id.as_ptr()).cast()
    }
    pub unsafe fn load(path: &str, id: &str) -> Result<Self> {
        require(
            Path::new(path).is_absolute(),
            "binary path must be absolute",
        )?;
        let library = libloading::Library::new(path)?;
        let entry = *library.get::<*const clap_plugin_entry>(b"clap_entry\0")?;
        require(
            !entry.is_null() && clap_version_is_compatible((*entry).clap_version),
            "incompatible CLAP entry",
        )?;
        let path = CString::new(path)?;
        require(
            ((*entry).init.ok_or("missing entry init")?)(path.as_ptr()),
            "entry init failed",
        )?;
        let mut state = Box::new(HostState {
            main: thread::current().id(),
            audio: AtomicBool::new(false),
            callback: AtomicBool::new(false),
            restart: AtomicBool::new(false),
        });
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: (&mut *state as *mut HostState).cast(),
            name: c"Slice 1 probe".as_ptr(),
            vendor: c"local experiment".as_ptr(),
            url: c"".as_ptr(),
            version: c"0.1.0".as_ptr(),
            get_extension: Some(host_extension),
            request_restart: Some(restart),
            request_process: Some(request_process),
            request_callback: Some(callback),
        });
        let mut this = Self {
            _library: library,
            entry,
            plugin: ptr::null(),
            _host: host,
            state,
            active: false,
            processing: false,
            time: 0,
            params: vec![],
        };
        let factory =
            ((*entry).get_factory.ok_or("missing get_factory")?)(CLAP_PLUGIN_FACTORY_ID.as_ptr())
                .cast::<clap_plugin_factory>();
        require(!factory.is_null(), "missing plugin factory")?;
        // Enumerate only the explicitly selected container, and require an exact identity.
        let mut found = false;
        for i in 0..((*factory).get_plugin_count.unwrap())(factory) {
            let d = ((*factory).get_plugin_descriptor.unwrap())(factory, i);
            require(!d.is_null(), "null descriptor")?;
            println!(
                "DESCRIPTOR id={} name={} version={} clap={:?}",
                text((*d).id),
                text((*d).name),
                text((*d).version),
                (*d).clap_version
            );
            if text((*d).id) == id {
                require(
                    clap_version_is_compatible((*d).clap_version),
                    "incompatible plugin version",
                )?;
                found = true;
            }
        }
        require(found, "explicit plugin ID not in container")?;
        let id = CString::new(id)?;
        this.plugin = ((*factory).create_plugin.unwrap())(factory, &*this._host, id.as_ptr());
        require(!this.plugin.is_null(), "create_plugin failed")?;
        require(
            ((*this.plugin).init.ok_or("missing plugin init")?)(this.plugin),
            "plugin init failed",
        )?;
        let ports = this.extension::<clap_plugin_audio_ports>(CLAP_EXT_AUDIO_PORTS);
        require(!ports.is_null(), "audio-ports extension required")?;
        for input in [true, false] {
            require(
                ((*ports).count.unwrap())(this.plugin, input) == 1,
                "requires exactly one input/output bus",
            )?;
            let mut info = std::mem::zeroed();
            require(
                ((*ports).get.unwrap())(this.plugin, 0, input, &mut info),
                "port get failed",
            )?;
            require(
                info.channel_count == 2
                    && !info.port_type.is_null()
                    && CStr::from_ptr(info.port_type) == CLAP_PORT_STEREO,
                "requires stereo ports",
            )?;
            println!(
                "PORT input={input} channels={} type={} flags={}",
                info.channel_count,
                text(info.port_type),
                info.flags
            );
        }
        let params = this.extension::<clap_plugin_params>(CLAP_EXT_PARAMS);
        require(!params.is_null(), "params extension required")?;
        for i in 0..((*params).count.unwrap())(this.plugin) {
            let mut info = std::mem::zeroed();
            require(
                ((*params).get_info.unwrap())(this.plugin, i, &mut info),
                "param get_info failed",
            )?;
            println!(
                "PARAM id={} name={} min={} max={} default={} flags={}",
                info.id,
                text(info.name.as_ptr()),
                info.min_value,
                info.max_value,
                info.default_value,
                info.flags
            );
            this.params.push(info);
        }
        let render = this.extension::<clap_plugin_render>(CLAP_EXT_RENDER);
        if render.is_null() {
            println!("RENDER extension absent: offline scheduling, no plugin-specific mode");
        } else {
            require(
                !((*render).has_hard_realtime_requirement.unwrap())(this.plugin),
                "hard realtime plugin unsupported",
            )?;
            require(
                ((*render).set.unwrap())(this.plugin, CLAP_RENDER_OFFLINE),
                "offline mode rejected",
            )?;
            println!("RENDER offline accepted");
        }
        require(
            ((*this.plugin).activate.unwrap())(this.plugin, RATE, BLOCK as u32, BLOCK as u32),
            "activate rejected 48k/256",
        )?;
        this.active = true;
        let latency = this.extension::<clap_plugin_latency>(CLAP_EXT_LATENCY);
        if latency.is_null() {
            println!("LATENCY extension absent");
        } else {
            println!("LATENCY samples={}", ((*latency).get.unwrap())(this.plugin));
        }
        this.state.audio.store(true, Ordering::Relaxed);
        let started = ((*this.plugin).start_processing.unwrap())(this.plugin);
        this.state.audio.store(false, Ordering::Relaxed);
        require(started, "start_processing failed")?;
        this.processing = true;
        this.service()?;
        Ok(this)
    }
    pub unsafe fn service(&self) -> Result<()> {
        // Bound callback cycles. The parent's wall-clock timeout also bounds a stuck call.
        for _ in 0..16 {
            if !self.state.callback.swap(false, Ordering::Relaxed) {
                break;
            }
            ((*self.plugin).on_main_thread.unwrap())(self.plugin);
        }
        require(
            !self.state.restart.load(Ordering::Relaxed),
            "unsupported: plugin requested restart",
        )
    }
    pub unsafe fn reset(&mut self) -> bool {
        if let Some(reset) = (*self.plugin).reset {
            self.state.audio.store(true, Ordering::Relaxed);
            reset(self.plugin);
            self.state.audio.store(false, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
    pub unsafe fn latency(&self) -> Option<u32> {
        let ext = self.extension::<clap_plugin_latency>(CLAP_EXT_LATENCY);
        if ext.is_null() {
            None
        } else {
            Some(((*ext).get.unwrap())(self.plugin))
        }
    }
    pub unsafe fn parameters(&self) -> Result<Vec<crate::hosting::Parameter>> {
        let ext = self.extension::<clap_plugin_params>(CLAP_EXT_PARAMS);
        self.params
            .iter()
            .map(|info| {
                let mut value = 0.;
                require(
                    ((*ext).get_value.unwrap())(self.plugin, info.id, &mut value),
                    "parameter readback failed",
                )?;
                let mut formatted = [0; 256];
                let formatted = if ((*ext).value_to_text.unwrap())(
                    self.plugin,
                    info.id,
                    value,
                    formatted.as_mut_ptr(),
                    256,
                ) {
                    text(formatted.as_ptr())
                } else {
                    value.to_string()
                };
                Ok(crate::hosting::Parameter {
                    id: info.id,
                    name: text(info.name.as_ptr()),
                    min: info.min_value,
                    max: info.max_value,
                    value,
                    formatted,
                    stepped: info.flags & CLAP_PARAM_IS_STEPPED != 0,
                    readonly: info.flags & CLAP_PARAM_IS_READONLY != 0,
                })
            })
            .collect()
    }
    pub unsafe fn snapshot(&self) -> Result<String> {
        let ext = self.extension::<clap_plugin_params>(CLAP_EXT_PARAMS);
        let mut values = vec![];
        for info in &self.params {
            let mut value = 0.0;
            require(
                ((*ext).get_value.unwrap())(self.plugin, info.id, &mut value),
                "get_value failed",
            )?;
            values.push(format!("{}={value:.12}", info.id));
        }
        Ok(values.join(","))
    }
    pub unsafe fn block(
        &mut self,
        mut input: [[f32; BLOCK]; 2],
        event: Option<&clap_event_param_value>,
    ) -> Result<[[f32; BLOCK]; 2]> {
        let mut output = [[f32::NAN; BLOCK]; 2];
        let mut ins = [input[0].as_mut_ptr(), input[1].as_mut_ptr()];
        let mut outs = [output[0].as_mut_ptr(), output[1].as_mut_ptr()];
        let in_buffer = clap_audio_buffer {
            data32: ins.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let mut out_buffer = clap_audio_buffer {
            data32: outs.as_mut_ptr(),
            ..in_buffer
        };
        let events = clap_input_events {
            ctx: event.map_or(ptr::null_mut(), |e| {
                (e as *const clap_event_param_value).cast_mut().cast()
            }),
            size: Some(event_count),
            get: Some(event_get),
        };
        let out_events = clap_output_events {
            ctx: ptr::null_mut(),
            try_push: Some(discard_event),
        };
        let process = clap_process {
            steady_time: self.time,
            frames_count: BLOCK as u32,
            transport: ptr::null(),
            audio_inputs: &in_buffer,
            audio_outputs: &mut out_buffer,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &events,
            out_events: &out_events,
        };
        self.state.audio.store(true, Ordering::Relaxed);
        let status = ((*self.plugin).process.unwrap())(self.plugin, &process);
        self.state.audio.store(false, Ordering::Relaxed);
        self.time += BLOCK as i64;
        self.service()?;
        require(
            (CLAP_PROCESS_CONTINUE..=CLAP_PROCESS_SLEEP).contains(&status),
            "process failed/unknown status",
        )?;
        require(
            output.iter().flatten().all(|v| v.is_finite()),
            "nonfinite/unwritten output",
        )?;
        Ok(output)
    }
    pub unsafe fn set(&mut self, id: u32, value: f64) -> Result<()> {
        let info = self
            .params
            .iter()
            .find(|p| p.id == id)
            .ok_or("unknown param ID")?;
        require(
            value.is_finite()
                && value >= info.min_value
                && value <= info.max_value
                && info.flags & CLAP_PARAM_IS_READONLY == 0,
            "invalid parameter change",
        )?;
        require(
            info.flags & CLAP_PARAM_IS_STEPPED == 0 || value.fract() == 0.0,
            "stepped parameter needs integer",
        )?;
        let event = clap_event_param_value {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_param_value>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_PARAM_VALUE,
                flags: 0,
            },
            param_id: id,
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value,
        };
        self.block([[0.0; BLOCK]; 2], Some(&event))?;
        let params = self.extension::<clap_plugin_params>(CLAP_EXT_PARAMS);
        let mut actual = 0.0;
        require(
            ((*params).get_value.unwrap())(self.plugin, id, &mut actual),
            "parameter readback failed",
        )?;
        require(
            (actual - value).abs() <= TOLERANCE,
            "parameter readback differs from request",
        )?;
        let mut formatted = [0; 256];
        require(
            ((*params).value_to_text.unwrap())(
                self.plugin,
                id,
                actual,
                formatted.as_mut_ptr(),
                formatted.len() as u32,
            ),
            "parameter formatting failed",
        )?;
        println!(
            "CHANGE id={id} requested={value} actual={actual} text={}",
            text(formatted.as_ptr())
        );
        Ok(())
    }
    pub unsafe fn render(&mut self, reset: bool, path: &Path) -> Result<()> {
        let settings = self.snapshot()?;
        if reset {
            self.state.audio.store(true, Ordering::Relaxed);
            ((*self.plugin).reset.unwrap())(self.plugin);
            self.state.audio.store(false, Ordering::Relaxed);
            // Reset DSP only; keep the host's elapsed sample timeline monotonic.
            for b in 0..PRE_BLOCKS {
                self.block(input(b), None)?;
            }
        }
        let mut bytes = Vec::with_capacity(RENDER_BLOCKS * BLOCK * 2 * 4);
        for b in PRE_BLOCKS..PRE_BLOCKS + RENDER_BLOCKS {
            let output = self.block(input(b), None)?;
            for i in 0..BLOCK {
                for channel in &output {
                    bytes.extend_from_slice(&channel[i].to_le_bytes());
                }
            }
        }
        require(
            self.snapshot()? == settings,
            "reset/render changed parameter settings",
        )?;
        let latency = self.extension::<clap_plugin_latency>(CLAP_EXT_LATENCY);
        if !latency.is_null() {
            println!(
                "LATENCY after-render samples={}",
                ((*latency).get.unwrap())(self.plugin)
            );
        }
        fs::write(path, bytes)?;
        Ok(())
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        unsafe {
            if !self.plugin.is_null() {
                if self.processing {
                    self.state.audio.store(true, Ordering::Relaxed);
                    ((*self.plugin).stop_processing.unwrap())(self.plugin);
                    self.state.audio.store(false, Ordering::Relaxed);
                }
                if self.active {
                    ((*self.plugin).deactivate.unwrap())(self.plugin);
                }
                if let Some(destroy) = (*self.plugin).destroy {
                    destroy(self.plugin);
                }
            }
            if let Some(deinit) = (*self.entry).deinit {
                deinit();
            }
        }
    }
}
