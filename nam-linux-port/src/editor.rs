//! Host-side GPUI adapter. Only the helper owns a GPUI event loop.
//! Host notifications are marshalled back onto the DAW's GUI thread.
use crate::{
    features::amplifier::{
        assets::Shared,
        plugin::{NamLinux, NamParams},
    },
    protocol::{HostMessage, UiEvent, HEIGHT, WIDTH},
};
use nih_plug::prelude::*;
use std::{
    any::Any,
    collections::HashSet,
    ffi::CStr,
    io::{BufRead, BufReader, Write},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex, Weak,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

type Gestures = Arc<Mutex<HashSet<String>>>;

pub struct GuiTask {
    event: Option<UiEvent>,
    context: Weak<dyn GuiContext>,
    alive: Arc<AtomicBool>,
    gestures: Gestures,
}

pub fn task_executor(params: Arc<NamParams>, shared: Arc<Shared>) -> TaskExecutor<NamLinux> {
    Box::new(move |task| {
        if !task.alive.load(Ordering::Acquire) {
            return;
        }
        let Some(context) = task.context.upgrade() else {
            return;
        };
        let Some(event) = task.event else {
            end_gestures(&params, context.as_ref(), &task.gestures);
            task.alive.store(false, Ordering::Release);
            return;
        };
        let id = match &event {
            UiEvent::Begin { id } | UiEvent::Set { id, .. } | UiEvent::End { id } => Some(id),
            _ => None,
        };
        if let Some(id) = id {
            let Some((_, ptr, _)) = params
                .param_map()
                .into_iter()
                .find(|(name, _, _)| name == id)
            else {
                return;
            };
            let mut gestures = task.gestures.lock().unwrap();
            // SAFETY: ptr comes from params' own map; the Arc keeps it alive.
            // The host's execute_gui dispatcher runs this closure on its GUI thread.
            unsafe {
                match event {
                    UiEvent::Begin { id } => {
                        if gestures.insert(id) {
                            context.raw_begin_set_parameter(ptr);
                        }
                    }
                    UiEvent::Set { id, normalized } if normalized.is_finite() => {
                        if gestures.insert(id) {
                            context.raw_begin_set_parameter(ptr);
                        }
                        context.raw_set_parameter_normalized(ptr, normalized.clamp(0., 1.));
                    }
                    UiEvent::End { id } => {
                        if gestures.remove(&id) {
                            context.raw_end_set_parameter(ptr);
                        }
                    }
                    _ => {}
                }
            }
        } else {
            shared.handle_asset_event(&event);
        }
    })
}

fn end_gestures(params: &NamParams, context: &dyn GuiContext, gestures: &Gestures) {
    let mut active = gestures.lock().unwrap();
    for (id, ptr, _) in params.param_map() {
        if active.remove(&id) {
            // SAFETY: pointer belongs to the still-live params and this runs on the GUI thread.
            unsafe {
                context.raw_end_set_parameter(ptr);
            }
        }
    }
}

/// Locate the helper next to the loaded plugin, not next to the DAW executable.
/// An explicit override is useful for development and relocatable packaging.
pub fn helper_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NAM_LINUX_EDITOR") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    // SAFETY: dladdr writes a valid Dl_info when successful; dli_fname is then a C string.
    let library = unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(helper_path as *const () as *const libc::c_void, &mut info) == 0
            || info.dli_fname.is_null()
        {
            return None;
        }
        PathBuf::from(std::ffi::OsStr::from_bytes(
            CStr::from_ptr(info.dli_fname).to_bytes(),
        ))
    };
    let path = library.parent()?.join("nam-linux-editor");
    path.is_file().then_some(path)
}

pub struct NamEditor {
    params: Arc<NamParams>,
    shared: Arc<Shared>,
    executor: AsyncExecutor<NamLinux>,
    scale: AtomicU32,
}

impl NamEditor {
    pub fn new(
        params: Arc<NamParams>,
        shared: Arc<Shared>,
        executor: AsyncExecutor<NamLinux>,
    ) -> Self {
        Self {
            params,
            shared,
            executor,
            scale: AtomicU32::new(1.0_f32.to_bits()),
        }
    }
}

impl Editor for NamEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn Any + Send> {
        let ParentWindowHandle::X11Window(parent) = parent else {
            eprintln!("NAM Linux: editor requires an X11/XWayland host parent");
            return Box::new(());
        };
        let Some(helper) = helper_path() else {
            eprintln!(
                "NAM Linux: install nam-linux-editor beside the plugin or set NAM_LINUX_EDITOR"
            );
            return Box::new(());
        };
        let child = Command::new(helper)
            .args(["--parent", &parent.to_string()])
            .env(
                "GPUI_X11_SCALE_FACTOR",
                f32::from_bits(self.scale.load(Ordering::Relaxed)).to_string(),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                eprintln!("NAM Linux: cannot start editor: {error}");
                return Box::new(());
            }
        };
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let alive = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let gestures = Arc::new(Mutex::new(HashSet::new()));
        let executor = self.executor.clone();
        let reader_alive = alive.clone();
        let reader_stop = stop.clone();
        let reader_context = Arc::downgrade(&context);
        let reader_gestures = gestures.clone();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if reader_stop.load(Ordering::Acquire) || line.len() > 64 * 1024 {
                    break;
                }
                if let Ok(event) = serde_json::from_str::<UiEvent>(&line) {
                    executor.execute_gui(GuiTask {
                        event: Some(event),
                        context: reader_context.clone(),
                        alive: reader_alive.clone(),
                        gestures: reader_gestures.clone(),
                    });
                }
            }
            executor.execute_gui(GuiTask {
                event: None,
                context: reader_context,
                alive: reader_alive,
                gestures: reader_gestures,
            });
            reader_stop.store(true, Ordering::Release);
        });
        let params = self.params.clone();
        let shared = self.shared.clone();
        let writer_stop = stop.clone();
        let writer = thread::spawn(move || {
            while !writer_stop.load(Ordering::Acquire) {
                let snapshot = shared.snapshot(params.normalized_values());
                let message = HostMessage::Snapshot(snapshot);
                if serde_json::to_writer(&mut stdin, &message).is_err()
                    || stdin.write_all(b"\n").is_err()
                    || stdin.flush().is_err()
                {
                    break;
                }
                thread::sleep(Duration::from_millis(33));
            }
        });
        Box::new(EditorHandle {
            child,
            alive,
            stop,
            reader: Some(reader),
            writer: Some(writer),
            params: self.params.clone(),
            context,
            gestures,
        })
    }

    fn size(&self) -> (u32, u32) {
        (WIDTH, HEIGHT)
    }
    fn set_scale_factor(&self, factor: f32) -> bool {
        if !factor.is_finite() || !(0.5..=4.0).contains(&factor) {
            return false;
        }
        self.scale.store(factor.to_bits(), Ordering::Relaxed);
        true
    }
    // Snapshots poll atomically published parameters; these callbacks may originate on audio threads.
    fn param_value_changed(&self, _: &str, _: f32) {}
    fn param_modulation_changed(&self, _: &str, _: f32) {}
    fn param_values_changed(&self) {}
}

struct EditorHandle {
    child: Child,
    alive: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    params: Arc<NamParams>,
    context: Arc<dyn GuiContext>,
    gestures: Gestures,
}

impl Drop for EditorHandle {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.stop.store(true, Ordering::Release);
        end_gestures(&self.params, self.context.as_ref(), &self.gestures);
        // Termination closes both pipes before joining, including a writer blocked on a hung UI.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}
