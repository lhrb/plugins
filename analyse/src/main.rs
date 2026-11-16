//! Trusted local binaries only. Each worker owns one CLAP instance for its lifetime.
use clap_probe::*;
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
    io::{self, BufRead, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
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

struct Plugin {
    // Fields remain alive throughout destroy/deinit, including failed initialization.
    _library: libloading::Library,
    entry: *const clap_plugin_entry,
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    state: Box<HostState>,
    active: bool,
    processing: bool,
    time: i64,
    params: Vec<clap_param_info>,
}
impl Plugin {
    unsafe fn extension<T>(&self, id: &CStr) -> *const T {
        ((*self.plugin).get_extension.unwrap())(self.plugin, id.as_ptr()).cast()
    }
    unsafe fn load(path: &str, id: &str) -> Result<Self> {
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
    unsafe fn service(&self) -> Result<()> {
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
    unsafe fn snapshot(&self) -> Result<String> {
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
    unsafe fn block(
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
    unsafe fn set(&mut self, id: u32, value: f64) -> Result<()> {
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
    unsafe fn render(&mut self, reset: bool, path: &Path) -> Result<()> {
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

unsafe fn worker(args: &[String]) -> Result<()> {
    let mut plugin = Plugin::load(&args[0], &args[1])?;
    println!("OK ready");
    io::stdout().flush()?;
    let (tx, commands) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        // Keep main-thread callbacks serviced even while this loaded slot is idle.
        plugin.service()?;
        let line = match commands.recv_timeout(Duration::from_millis(10)) {
            Ok(line) => line?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let words: Vec<_> = line.split_whitespace().collect();
        match words.as_slice() {
            ["snapshot"] => println!("OK snapshot {}", plugin.snapshot()?),
            ["set", id, value] => {
                plugin.set(id.parse()?, value.parse()?)?;
                println!("OK set {}", plugin.snapshot()?);
            }
            ["render", reset, path] => {
                plugin.render(*reset == "reset", Path::new(path))?;
                println!("OK render {} time={}", plugin.snapshot()?, plugin.time);
            }
            ["crash"] => std::process::abort(),
            ["hang"] => loop {
                thread::sleep(Duration::from_secs(60));
            },
            ["quit"] => {
                // Acknowledge only after teardown, so the parent's timeout covers it too.
                drop(plugin);
                println!("OK quit");
                io::stdout().flush()?;
                return Ok(());
            }
            _ => return Err("invalid worker command".into()),
        }
        plugin.service()?;
        io::stdout().flush()?;
    }
    Ok(())
}

struct Worker {
    name: String,
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    timeout: Duration,
}
impl Worker {
    fn spawn(name: &str, path: &str, id: &str, timeout: Duration) -> Result<Self> {
        let mut child = Command::new(std::env::current_exe()?)
            .args(["worker", path, id])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in io::BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let mut worker = Self {
            name: name.into(),
            child,
            stdin,
            lines,
            timeout,
        };
        worker.response()?;
        Ok(worker)
    }
    fn response(&mut self) -> Result<String> {
        let deadline = std::time::Instant::now() + self.timeout;
        loop {
            match self
                .lines
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            {
                Ok(line) => {
                    println!("{}: {line}", self.name);
                    if line.starts_with("OK ") {
                        return Ok(line);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.child.kill()?;
                    let status = self.child.wait()?;
                    return Err(format!(
                        "{} configured timeout {:?}; killed {status}",
                        self.name, self.timeout
                    )
                    .into());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let status = match self.child.try_wait()? {
                        Some(status) => status,
                        None => {
                            self.child.kill()?;
                            self.child.wait()?
                        }
                    };
                    return Err(
                        format!("{} worker exited/closed stdout {status}", self.name).into(),
                    );
                }
            }
        }
    }
    fn command(&mut self, command: &str) -> Result<String> {
        writeln!(self.stdin, "{command}")?;
        self.stdin.flush()?;
        self.response()
    }
    fn render(&mut self, dir: &Path, label: &str, reset: bool) -> Result<Vec<f32>> {
        let path = dir.join(format!("{}-{label}.f32le", self.name));
        self.command(&format!(
            "render {} {}",
            if reset { "reset" } else { "continue" },
            path.display()
        ))?;
        read_samples(&path)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn read_samples(path: &Path) -> Result<Vec<f32>> {
    let bytes = fs::read(path)?;
    require(
        bytes.len() == RENDER_BLOCKS * BLOCK * 2 * 4,
        "unexpected render length",
    )?;
    let samples: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    require(samples.iter().all(|v| v.is_finite()), "nonfinite render")?;
    Ok(samples)
}
fn compare(label: &str, a: &[f32], b: &[f32]) {
    let (peak, rms) = difference(a, b);
    println!(
        "COMPARE {label}: peak={peak:.12e} rms={rms:.12e} tolerance={TOLERANCE} within={}",
        peak <= TOLERANCE
    );
}
fn run(args: &[String]) -> Result<()> {
    require(
        args.len() == 10,
        "run ABS_PATH_A ID_A PARAM_A VALUE_A ABS_PATH_B ID_B PARAM_B VALUE_B OUTPUT_DIR TIMEOUT_MS",
    )?;
    let dir = fs::canonicalize(&args[8])?;
    require(
        !dir.to_string_lossy().contains(char::is_whitespace),
        "output path cannot contain whitespace",
    )?;
    let mut input_bytes = Vec::new();
    for b in 0..PRE_BLOCKS + RENDER_BLOCKS {
        let block = input(b);
        for i in 0..BLOCK {
            for channel in &block {
                input_bytes.extend_from_slice(&channel[i].to_le_bytes());
            }
        }
    }
    fs::write(dir.join("input.f32le"), input_bytes)?;
    let timeout = Duration::from_millis(args[9].parse()?);
    require(!timeout.is_zero(), "timeout must be positive")?;
    let mut a = Worker::spawn("oxide", &args[0], &args[1], timeout)?;
    let mut b = Worker::spawn("chow", &args[4], &args[5], timeout)?;
    // Both selected plugins remain loaded throughout all renders and interventions.
    let a1 = a.render(&dir, "repeat1", true)?;
    let a2 = a.render(&dir, "repeat2", true)?;
    compare("oxide-reset-repeat", &a1, &a2);
    let b1 = b.render(&dir, "repeat1", true)?;
    let b2 = b.render(&dir, "repeat2", true)?;
    compare("chow-reset-repeat", &b1, &b2);
    let mut ac = Worker::spawn("oxide-control", &args[0], &args[1], timeout)?;
    let mut bc = Worker::spawn("chow-control", &args[4], &args[5], timeout)?;
    // Mirror all prior CHOW renders: reset alone did not reproduce its initial state.
    let bc1 = bc.render(&dir, "repeat1", true)?;
    let bc2 = bc.render(&dir, "repeat2", true)?;
    compare("chow-fresh-instance-first-trial", &b1, &bc1);
    compare("chow-matched-history-second-trial", &b2, &bc2);
    let before_b = b.command("snapshot")?;
    let bs = b.render(&dir, "history-start", true)?;
    let bcs = bc.render(&dir, "history-start", true)?;
    compare("chow-control-baseline", &bs, &bcs);
    a.command(&format!("set {} {}", args[2], args[3]))?;
    let changed_a = a.render(&dir, "edited", true)?;
    compare("oxide-parameter-effect", &a2, &changed_a);
    require(
        before_b == b.command("snapshot")?,
        "oxide edit changed chow settings",
    )?;
    let bh = b.render(&dir, "history-continue", false)?;
    let bch = bc.render(&dir, "history-continue", false)?;
    compare("chow-history-after-oxide-edit-reset", &bh, &bch);
    compare(
        "chow-continuation-versus-start-history-sensitivity",
        &bs,
        &bh,
    );
    // Apply the same new setting to A's control, then establish equal starting histories.
    ac.command(&format!("set {} {}", args[2], args[3]))?;
    let before_a = a.command("snapshot")?;
    let ast = a.render(&dir, "history-start", true)?;
    let acst = ac.render(&dir, "history-start", true)?;
    compare("oxide-control-baseline", &ast, &acst);
    b.command(&format!("set {} {}", args[6], args[7]))?;
    let changed_b = b.render(&dir, "edited", true)?;
    compare("chow-parameter-effect", &b2, &changed_b);
    require(
        before_a == a.command("snapshot")?,
        "chow edit changed oxide settings",
    )?;
    let ah = a.render(&dir, "history-continue", false)?;
    let ach = ac.render(&dir, "history-continue", false)?;
    compare("oxide-history-after-chow-edit-reset", &ah, &ach);
    compare(
        "oxide-continuation-versus-start-history-sensitivity",
        &ast,
        &ah,
    );
    // Retain an actual completed peer render across both controlled worker faults.
    let retained = dir.join("chow-edited.f32le");
    let retained_bytes = fs::read(&retained)?;
    let crash = a.command("crash").expect_err("crash should fail");
    println!("RECOVERY crash reported: {crash}");
    require(
        fs::read(&retained)? == retained_bytes,
        "peer result changed after crash",
    )?;
    b.command("snapshot")?;
    let mut replacement = Worker::spawn("oxide-replacement", &args[0], &args[1], timeout)?;
    let timed = replacement
        .command("hang")
        .expect_err("hang should time out");
    println!("RECOVERY timeout reported: {timed}");
    require(
        fs::read(&retained)? == retained_bytes,
        "peer result changed after timeout",
    )?;
    b.command("snapshot")?;
    println!(
        "RECOVERY retained {} bytes; surviving peer responsive",
        retained_bytes.len()
    );
    for worker in [&mut b, &mut ac, &mut bc] {
        worker.command("quit")?;
        require(worker.child.wait()?.success(), "worker shutdown failed")?;
    }
    println!("CHECKPOINT slice 1 finished; numerical variation is evidence, not automatically a host defect");
    Ok(())
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("worker") if args.len() == 3 => unsafe { worker(&args[1..]) },
        Some("inspect") if args.len() == 4 => (|| { let mut w = Worker::spawn("inspect", &args[1], &args[2], Duration::from_millis(args[3].parse()?))?; w.command("snapshot")?; w.command("quit")?; require(w.child.wait()?.success(), "shutdown failed") })(),
        Some("run") => run(&args[1..]),
        _ => Err("usage: clap-probe inspect ABS_PATH ID TIMEOUT_MS | run PATH_A ID_A PARAM_A VALUE_A PATH_B ID_B PARAM_B VALUE_B EXISTING_OUTPUT_DIR TIMEOUT_MS".into()),
    };
    if let Err(error) = result {
        eprintln!("UNSUPPORTED/FAILED: {error}");
        std::process::exit(1);
    }
}
