//! The helper alone owns GPUI and its event loop. No device or DSP lives here.
#[path = "../embedding.rs"]
mod embedding;
use audio_test_reference::domain::{
    low_pass_control, Coefficients, CUTOFF_RANGE, DEFAULT_CUTOFF, DEFAULT_Q, MAX_DELAY_SAMPLES,
    Q_RANGE,
};
use gpui::{prelude::*, *};
use std::{
    cell::Cell,
    io::{self, Read, Write},
    ops::RangeInclusive,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Default)]
struct Status {
    fault: AtomicBool,
    closed: AtomicBool,
    mode: AtomicU8,
    gain: AtomicU32,
    delay_samples: AtomicI32,
    filter: Mutex<FilterStatus>,
}

#[derive(Clone, Copy)]
struct FilterStatus {
    cutoff: f32,
    q: f32,
    rate: f32,
    applied_cutoff: f32,
    applied_q: f32,
}

impl Default for FilterStatus {
    fn default() -> Self {
        Self {
            cutoff: DEFAULT_CUTOFF as f32,
            q: DEFAULT_Q as f32,
            rate: 0.0,
            applied_cutoff: 0.0,
            applied_q: 0.0,
        }
    }
}

impl FilterStatus {
    fn readback(self) -> impl IntoElement {
        let cutoff = low_pass_control(self.applied_cutoff, CUTOFF_RANGE);
        let q = low_pass_control(self.applied_q, Q_RANGE);
        let status = div().text_sm();
        if self.rate == 0.0 {
            return status
                .child("No active sample rate. Effective values appear during processing.");
        }
        if let Some((cutoff, q)) = cutoff.zip(q) {
            if let Ok(Coefficients { b0, b1, b2, a1, a2 }) = Coefficients::from_cutoff_q(cutoff, q)
            {
                let hz = cutoff * f64::from(self.rate);
                return status
                    .child(div().child(format!("Effective: {cutoff:?} · {hz:?} Hz · Q {q:?}")))
                    .child(div().child(format!("b0={b0:?} · b1={b1:?} · b2={b2:?}")))
                    .child(div().child(format!("a1={a1:?} · a2={a2:?} · a0=1")));
            }
        }
        status.child("Effective values appear after Low-pass is applied by the audio engine.")
    }
}

impl Status {
    fn mode(&self) -> EditorMode {
        match self.mode.load(Ordering::Relaxed) {
            b'G' => EditorMode::Gain,
            b'D' => EditorMode::Delay,
            b'L' => EditorMode::LowPass,
            _ => EditorMode::Identity,
        }
    }

    fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum EditorMode {
    Identity = b'I',
    Gain = b'G',
    Delay = b'D',
    LowPass = b'L',
}

enum EditorEdit {
    Mode(EditorMode),
    Gain(f32),
    DelaySamples(i32),
    Cutoff(f32),
    Q(f32),
}

#[derive(Clone, Copy)]
enum Slider {
    Gain,
    Delay,
    Cutoff,
    Q,
}

const MIN_GAIN: f32 = -16.0;
const MAX_GAIN: f32 = 16.0;

struct EditorView {
    status: Arc<Status>,
    mode: EditorMode,
    gain: f32,
    delay_samples: i32,
    filter: FilterStatus,
    slider_drag: Option<(Slider, Bounds<Pixels>)>,
    // Retain the X11 client while the child window is embedded.
    _embedding: x11rb::rust_connection::RustConnection,
    edit_tx: std::sync::mpsc::Sender<EditorEdit>,
}

impl EditorView {
    fn select_mode(&mut self, mode: EditorMode) {
        self.mode = mode;
        self.slider_drag = None;
    }

    fn edit_slider(&mut self, slider: Slider, x: Pixels, bounds: Bounds<Pixels>) {
        match slider {
            Slider::Gain => {
                self.gain = gain_at(x, bounds);
                self.send_edit(EditorEdit::Gain(self.gain));
            }
            Slider::Delay => {
                self.delay_samples = delay_samples_at(x, bounds);
                self.send_edit(EditorEdit::DelaySamples(self.delay_samples));
            }
            Slider::Cutoff => {
                if let Some(value) = low_pass_at(x, bounds, CUTOFF_RANGE) {
                    self.filter.cutoff = value;
                    self.send_edit(EditorEdit::Cutoff(value));
                }
            }
            Slider::Q => {
                if let Some(value) = low_pass_at(x, bounds, Q_RANGE) {
                    self.filter.q = value;
                    self.send_edit(EditorEdit::Q(value));
                }
            }
        }
    }

    fn send_edit(&self, edit: EditorEdit) {
        if let Err(error) = self.edit_tx.send(edit) {
            eprintln!("Could not queue editor edit: {error}");
        }
    }

    fn mode_button(
        &self,
        label: &'static str,
        mode: EditorMode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.mode == mode;
        div()
            .id(label)
            .px_4()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(if selected { 0x80bfff } else { 0x535c68 }))
            .bg(rgb(if selected { 0x284769 } else { 0x2b3036 }))
            .cursor_pointer()
            .child(label)
            .on_click(cx.listener(move |this, _, _, _| {
                this.send_edit(EditorEdit::Mode(mode));
            }))
    }

    fn slider(
        &self,
        slider: Slider,
        label: String,
        fraction: f32,
        [min, midpoint, max]: [&'static str; 3],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let bounds = Rc::new(Cell::new(None));
        let prepaint_bounds = bounds.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(label)
            .child(
                div()
                    .px_2()
                    .on_children_prepainted(move |children, _, _| {
                        prepaint_bounds.set(children.first().copied());
                    })
                    .child(
                        div()
                            .relative()
                            .h(px(24.))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    if let Some(bounds) = bounds.get() {
                                        this.slider_drag = Some((slider, bounds));
                                        this.edit_slider(slider, event.position.x, bounds);
                                        cx.notify();
                                    }
                                }),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(10.))
                                    .w_full()
                                    .h(px(4.))
                                    .rounded_full()
                                    .bg(rgb(0x535c68)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(7.))
                                    .left(relative(0.5))
                                    .w(px(1.))
                                    .h(px(10.))
                                    .bg(rgb(0xa4aeba)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(5.))
                                    .left(relative(fraction))
                                    .ml(px(-7.))
                                    .size(px(14.))
                                    .rounded_full()
                                    .bg(rgb(0x80bfff)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_sm()
                    .text_color(rgb(0xa4aeba))
                    .child(min)
                    .child(midpoint)
                    .child(max),
            )
    }
}

fn slider_fraction(x: Pixels, bounds: Bounds<Pixels>) -> f32 {
    let offset = f32::from(x - bounds.origin.x);
    let width = f32::from(bounds.size.width);
    (offset / width).clamp(0.0, 1.0)
}

fn gain_at(x: Pixels, bounds: Bounds<Pixels>) -> f32 {
    MIN_GAIN + slider_fraction(x, bounds) * (MAX_GAIN - MIN_GAIN)
}

fn delay_samples_at(x: Pixels, bounds: Bounds<Pixels>) -> i32 {
    (slider_fraction(x, bounds) * MAX_DELAY_SAMPLES as f32).round() as i32
}

fn low_pass_at(x: Pixels, bounds: Bounds<Pixels>, range: RangeInclusive<f64>) -> Option<f32> {
    let fraction = f64::from(slider_fraction(x, bounds));
    let value = *range.start() + fraction * (*range.end() - *range.start());
    range.contains(&value).then_some(value as f32)
}

fn low_pass_position(value: f32, range: RangeInclusive<f64>) -> f32 {
    let value = low_pass_control(value, range.clone()).unwrap_or(*range.start());
    ((value - *range.start()) / (*range.end() - *range.start())) as f32
}

fn gain_label(gain: f32) -> String {
    if gain == 0.0 {
        return "0× · mute (−∞ dB)".to_owned();
    }
    let db = 20.0 * f64::from(gain).abs().log10();
    format!("{gain}× · {db:.2} dB")
}

impl Render for EditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x202328))
            .text_color(rgb(0xe6e8eb))
            .p_8()
            .flex()
            .flex_col()
            .gap_4()
            .when(self.mode == EditorMode::LowPass, |view| view.gap_3())
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if !event.dragging() {
                    this.slider_drag = None;
                } else if let Some((slider, bounds)) = this.slider_drag {
                    this.edit_slider(slider, event.position.x, bounds);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.slider_drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.slider_drag = None),
            )
            .child(div().text_xl().child("AudioTestReference"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.mode_button("Identity", EditorMode::Identity, cx))
                    .child(self.mode_button("Gain", EditorMode::Gain, cx))
                    .child(self.mode_button("Delay", EditorMode::Delay, cx))
                    .child(self.mode_button("Low-pass", EditorMode::LowPass, cx)),
            )
            .when(self.mode == EditorMode::Gain, |view| {
                view.child(self.slider(
                    Slider::Gain,
                    format!("Gain: {}", gain_label(self.gain)),
                    (self.gain - MIN_GAIN) / (MAX_GAIN - MIN_GAIN),
                    ["−16×", "0×", "+16×"],
                    cx,
                ))
            })
            .when(self.mode == EditorMode::Delay, |view| {
                view.child(self.slider(
                    Slider::Delay,
                    format!("Delay Time: {} samples", self.delay_samples),
                    self.delay_samples as f32 / MAX_DELAY_SAMPLES as f32,
                    ["0", "96,000", "192,000"],
                    cx,
                ))
            })
            .when(self.mode == EditorMode::LowPass, |view| {
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(self.slider(
                            Slider::Cutoff,
                            format!("Cutoff (fraction): {}", self.filter.cutoff),
                            low_pass_position(self.filter.cutoff, CUTOFF_RANGE),
                            ["0.0001", "", "0.49"],
                            cx,
                        ))
                        .child(self.slider(
                            Slider::Q,
                            format!("Q: {}", self.filter.q),
                            low_pass_position(self.filter.q, Q_RANGE),
                            ["0.1", "", "20"],
                            cx,
                        ))
                        .child(self.filter.readback()),
                )
            })
            .child(if self.status.fault.load(Ordering::Relaxed) {
                "Audio fault: nonfinite input or invalid output (latched until reset/activation)."
            } else {
                "Audio safety: no faults observed since reset/activation."
            })
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--parent" {
        return Err("Use --parent <X11 window ID>".into());
    }
    let parent = args[1].parse::<u32>()?;
    let status = Arc::new(Status {
        gain: AtomicU32::new(1.0f32.to_bits()),
        ..Status::default()
    });
    let reader_status = status.clone();
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut message = [0u8; 30];
        while stdin.read_exact(&mut message).is_ok() {
            let [fault, mode, a, b, c, d, e, f, g, h, ..] = message;
            let gain = f32::from_le_bytes([a, b, c, d]);
            if (MIN_GAIN..=MAX_GAIN).contains(&gain) {
                reader_status.gain.store(gain.to_bits(), Ordering::Relaxed);
            }
            let delay_samples = i32::from_le_bytes([e, f, g, h]);
            if (0..=MAX_DELAY_SAMPLES as i32).contains(&delay_samples) {
                reader_status
                    .delay_samples
                    .store(delay_samples, Ordering::Relaxed);
            }
            let float_at =
                |offset| f32::from_le_bytes(message[offset..offset + 4].try_into().unwrap());
            let mut filter = reader_status.filter.lock().unwrap();
            let cutoff = float_at(10);
            let q = float_at(14);
            if low_pass_control(cutoff, CUTOFF_RANGE).is_some() {
                filter.cutoff = cutoff;
            }
            if low_pass_control(q, Q_RANGE).is_some() {
                filter.q = q;
            }
            filter.rate = float_at(18);
            filter.applied_cutoff = float_at(22);
            filter.applied_q = float_at(26);
            reader_status.fault.store(fault == b'1', Ordering::Relaxed);
            reader_status.mode.store(mode, Ordering::Relaxed);
        }
        reader_status.closed.store(true, Ordering::Relaxed);
    });

    let (edit_tx, edit_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = io::stdout().lock();

        for edit in edit_rx {
            let (tag, [a, b, c, d]) = match edit {
                EditorEdit::Mode(mode) => (mode as u8, [0; 4]),
                EditorEdit::Gain(gain) => (b'V', gain.to_le_bytes()),
                EditorEdit::DelaySamples(n) => (b'N', n.to_le_bytes()),
                EditorEdit::Cutoff(value) => (b'U', value.to_le_bytes()),
                EditorEdit::Q(value) => (b'Q', value.to_le_bytes()),
            };

            if stdout.write_all(&[tag, a, b, c, d]).is_err() || stdout.flush().is_err() {
                break;
            }
        }
    });

    Application::new().run(move |cx: &mut App| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(660.), px(430.)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: WindowKind::PopUp,
                is_resizable: false,
                is_movable: false,
                is_minimizable: false,
                app_id: Some("audio-test-reference-editor".into()),
                ..Default::default()
            },
            move |window, cx| {
                let connection =
                    embedding::embed(parent, window.scale_factor()).unwrap_or_else(|error| {
                        eprintln!("AudioTestReference embedding: {error}");
                        std::process::exit(1);
                    });
                cx.new(|cx: &mut Context<EditorView>| {
                    let polled_status = status.clone();
                    cx.spawn(async move |view, cx| loop {
                        Timer::after(Duration::from_millis(16)).await;
                        if polled_status.closed.load(Ordering::Relaxed) {
                            let _ = cx.update(|cx| cx.quit());
                            break;
                        }
                        let mode = polled_status.mode();
                        let gain = polled_status.gain();
                        let delay_samples = polled_status.delay_samples.load(Ordering::Relaxed);
                        let mut filter = *polled_status.filter.lock().unwrap();

                        if view
                            .update(cx, |this, cx| {
                                if this.mode != mode {
                                    this.select_mode(mode);
                                }
                                if this.slider_drag.is_none() || mode != EditorMode::Gain {
                                    this.gain = gain;
                                }
                                if this.slider_drag.is_none() || mode != EditorMode::Delay {
                                    this.delay_samples = delay_samples;
                                }
                                match this.slider_drag {
                                    Some((Slider::Cutoff, _)) => filter.cutoff = this.filter.cutoff,
                                    Some((Slider::Q, _)) => filter.q = this.filter.q,
                                    _ => {}
                                }
                                this.filter = filter;
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    })
                    .detach();
                    EditorView {
                        mode: status.mode(),
                        gain: status.gain(),
                        delay_samples: status.delay_samples.load(Ordering::Relaxed),
                        filter: FilterStatus::default(),
                        status,
                        slider_drag: None,
                        _embedding: connection,
                        edit_tx,
                    }
                })
            },
        );
        if let Err(error) = result {
            eprintln!("AudioTestReference window: {error}");
            std::process::exit(1);
        }
    });
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("AudioTestReference editor: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{delay_samples_at, gain_at, gain_label};
    use gpui::{point, px, size, Bounds};

    #[test]
    fn gain_label_preserves_sign_and_handles_mute() {
        assert_eq!(gain_label(0.5), "0.5× · -6.02 dB");
        assert_eq!(gain_label(-0.5), "-0.5× · -6.02 dB");
        assert_eq!(gain_label(0.0), "0× · mute (−∞ dB)");
    }

    #[test]
    fn gain_slider_maps_track_midpoint_to_mute_and_clamps_at_ends() {
        let bounds = Bounds::new(point(px(10.), px(0.)), size(px(100.), px(24.)));
        assert_eq!(gain_at(px(0.), bounds), -16.0);
        assert_eq!(gain_at(px(60.), bounds), 0.0);
        assert_eq!(gain_at(px(120.), bounds), 16.0);
    }

    #[test]
    fn delay_slider_maps_position_to_whole_samples_and_clamps_at_ends() {
        let bounds = Bounds::new(point(px(10.), px(0.)), size(px(100.), px(24.)));
        assert_eq!(delay_samples_at(px(0.), bounds), 0);
        assert_eq!(delay_samples_at(px(60.), bounds), 96_000);
        assert_eq!(delay_samples_at(px(60.0004), bounds), 96_001);
        assert_eq!(delay_samples_at(px(120.), bounds), 192_000);
    }
}
