//! Host-side helper ownership and parameter routing. Never called by DSP.
use nih_plug::prelude::*;
use std::{
    any::Any,
    ffi::CStr,
    io::{Read, Write},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::{
    AudioTestReference, HostMode, LowPassReadback, ParameterEdit, PluginParams, SetParameterTask,
};

fn helper_path() -> Option<PathBuf> {
    // SAFETY: dladdr fills info on success; dli_fname is a NUL-terminated library path.
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
    let path = library.parent()?.join("audio-test-reference-editor");
    path.is_file().then_some(path)
}

pub struct EditorBridge {
    params: Arc<PluginParams>,
    fault: Arc<AtomicBool>,
    low_pass: Arc<LowPassReadback>,
    scale: AtomicU32,
    executor: AsyncExecutor<AudioTestReference>,
}

impl EditorBridge {
    pub fn new(
        params: Arc<PluginParams>,
        fault: Arc<AtomicBool>,
        low_pass: Arc<LowPassReadback>,
        executor: AsyncExecutor<AudioTestReference>,
    ) -> Self {
        Self {
            params,
            fault,
            low_pass,
            scale: AtomicU32::new(1.0f32.to_bits()),
            executor,
        }
    }
}

impl Editor for EditorBridge {
    fn spawn(&self, parent: ParentWindowHandle, cx: Arc<dyn GuiContext>) -> Box<dyn Any + Send> {
        let ParentWindowHandle::X11Window(parent) = parent else {
            eprintln!("AudioTestReference: editor requires an X11/XWayland parent");
            return Box::new(());
        };
        let Some(helper) = helper_path() else {
            eprintln!(
                "AudioTestReference: install audio-test-reference-editor beside the CLAP module"
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
                eprintln!("AudioTestReference: cannot launch editor: {error}");
                return Box::new(());
            }
        };
        let mut to_helper = child.stdin.take().expect("piped helper stdin");
        let stop = Arc::new(AtomicBool::new(false));
        let writer_stop = stop.clone();
        let fault = Arc::clone(&self.fault);
        let params = Arc::clone(&self.params);
        let low_pass = Arc::clone(&self.low_pass);
        let writer = thread::spawn(move || {
            while !writer_stop.load(Ordering::Relaxed) {
                let fault_byte = if fault.load(Ordering::Relaxed) {
                    b'1'
                } else {
                    b'0'
                };

                let mode_byte = match params.mode.value() {
                    HostMode::Identity => b'I',
                    HostMode::Gain => b'G',
                    HostMode::Delay => b'D',
                    HostMode::LowPass => b'L',
                };

                // Status frame: fault, mode, LE gain/delay, requested cutoff/Q,
                // active rate, and the cutoff/Q pair last applied by DSP.
                let mut message = [0u8; 30];
                message[..2].copy_from_slice(&[fault_byte, mode_byte]);
                message[2..6].copy_from_slice(&params.gain.unmodulated_plain_value().to_le_bytes());
                message[6..10]
                    .copy_from_slice(&params.delay_samples.unmodulated_plain_value().to_le_bytes());
                message[10..14]
                    .copy_from_slice(&params.cutoff.unmodulated_plain_value().to_le_bytes());
                message[14..18].copy_from_slice(&params.q.unmodulated_plain_value().to_le_bytes());
                let rate = low_pass.sample_rate.load(Ordering::Acquire);
                let applied = low_pass.applied.load(Ordering::Relaxed);
                message[18..22].copy_from_slice(&rate.to_le_bytes());
                message[22..].copy_from_slice(&applied.to_le_bytes());
                if to_helper.write_all(&message).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(16));
            }
        });

        let executor = self.executor.clone();
        let weak_context = Arc::downgrade(&cx);
        let mut from_helper = child.stdout.take().expect("piped helper stdout");
        let reader = thread::spawn(move || {
            // Edit frame: tag byte and four payload bytes (unused for mode edits).
            let mut message = [0u8; 5];
            while from_helper.read_exact(&mut message).is_ok() {
                let [tag, payload @ ..] = message;
                let edit = match tag {
                    b'I' => ParameterEdit::Mode(HostMode::Identity),
                    b'G' => ParameterEdit::Mode(HostMode::Gain),
                    b'D' => ParameterEdit::Mode(HostMode::Delay),
                    b'L' => ParameterEdit::Mode(HostMode::LowPass),
                    b'V' => ParameterEdit::Gain(f32::from_le_bytes(payload)),
                    b'N' => ParameterEdit::DelaySamples(i32::from_le_bytes(payload)),
                    b'U' => ParameterEdit::Cutoff(f32::from_le_bytes(payload)),
                    b'Q' => ParameterEdit::Q(f32::from_le_bytes(payload)),
                    _ => {
                        eprintln!("AudioTestReference: unknown editor edit tag {tag}");
                        continue;
                    }
                };

                executor.execute_gui(SetParameterTask {
                    edit,
                    context: weak_context.clone(),
                });
            }
        });
        Box::new(EditorHandle {
            child,
            stop,
            writer: Some(writer),
            _context: cx,
            reader: Some(reader),
        })
    }
    fn size(&self) -> (u32, u32) {
        (660, 430)
    }
    fn set_scale_factor(&self, factor: f32) -> bool {
        if !factor.is_finite() || !(0.5..=4.0).contains(&factor) {
            return false;
        }
        self.scale.store(factor.to_bits(), Ordering::Relaxed);
        true
    }
    fn param_value_changed(&self, _: &str, _: f32) {}
    fn param_modulation_changed(&self, _: &str, _: f32) {}
    fn param_values_changed(&self) {}
}
struct EditorHandle {
    child: Child,
    stop: Arc<AtomicBool>,
    writer: Option<JoinHandle<()>>,
    _context: Arc<dyn GuiContext>,
    reader: Option<JoinHandle<()>>,
}
impl Drop for EditorHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Kill first to release even a pipe writer blocked by a hung renderer.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
