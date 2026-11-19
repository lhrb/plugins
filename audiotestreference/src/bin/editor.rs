//! The helper alone owns GPUI and its event loop. No device or DSP lives here.
#[path = "../embedding.rs"]
mod embedding;
use gpui::{prelude::*, *};
use std::{
    cell::Cell,
    io::{self, Read, Write},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Default)]
struct Status {
    fault: AtomicBool,
    closed: AtomicBool,
    gain_mode: AtomicBool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorMode {
    Identity,
    Gain,
}

const MIN_GAIN: f32 = -16.0;
const MAX_GAIN: f32 = 16.0;

struct EditorView {
    status: Arc<Status>,
    mode: EditorMode,
    gain: f32,
    gain_drag: Option<Bounds<Pixels>>,
    // Retain the X11 client while the child window is embedded.
    _embedding: x11rb::rust_connection::RustConnection,
    mode_tx: std::sync::mpsc::Sender<EditorMode>,
}

impl EditorView {
    // These handlers only update the preview. Connect user edits to the host here later.
    fn select_mode(&mut self, mode: EditorMode) {
        self.mode = mode;
        self.gain_drag = None;
    }

    fn edit_gain(&mut self, gain: f32) {
        self.gain = gain;
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
                if let Err(error) = this.mode_tx.send(mode) {
                    eprintln!("Could not queue mode request: {error}");
                }
            }))
    }

    fn gain_slider(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let fraction = (self.gain - MIN_GAIN) / (MAX_GAIN - MIN_GAIN);
        let bounds = Rc::new(Cell::new(None));
        let prepaint_bounds = bounds.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(format!("Gain: {}", gain_label(self.gain)))
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
                                        this.gain_drag = Some(bounds);
                                        this.edit_gain(gain_at(event.position.x, bounds));
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
                    .child("−16×")
                    .child("0×")
                    .child("+16×"),
            )
    }
}

fn gain_at(x: Pixels, bounds: Bounds<Pixels>) -> f32 {
    let offset = f32::from(x - bounds.origin.x);
    let width = f32::from(bounds.size.width);
    let fraction = (offset / width).clamp(0.0, 1.0);
    MIN_GAIN + fraction * (MAX_GAIN - MIN_GAIN)
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
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if !event.dragging() {
                    this.gain_drag = None;
                } else if let Some(bounds) = this.gain_drag {
                    this.edit_gain(gain_at(event.position.x, bounds));
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.gain_drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.gain_drag = None),
            )
            .child(div().text_xl().child("AudioTestReference"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0xa4aeba))
                    .child("Controls preview · not connected to the host"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.mode_button("Identity", EditorMode::Identity, cx))
                    .child(self.mode_button("Gain", EditorMode::Gain, cx)),
            )
            .when(self.mode == EditorMode::Gain, |view| {
                view.child(self.gain_slider(cx))
            })
            .child(if self.status.fault.load(Ordering::Relaxed) {
                "Fault: nonfinite input replaced with zero (latched until reset/activation)."
            } else {
                "Input safety: no nonfinite input observed since reset/activation."
            })
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--parent" {
        return Err("Use --parent <X11 window ID>".into());
    }
    let parent = args[1].parse::<u32>()?;
    let status = Arc::new(Status::default());
    let reader_status = status.clone();
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut message = [0u8; 2];
        while stdin.read_exact(&mut message).is_ok() {
            reader_status
                .fault
                .store(message[0] == b'1', Ordering::Relaxed);
            reader_status
                .gain_mode
                .store(message[1] == b'G', Ordering::Relaxed);
        }
        reader_status.closed.store(true, Ordering::Relaxed);
    });

    let (mode_tx, mode_rx) = std::sync::mpsc::channel::<EditorMode>();
    std::thread::spawn(move || {
        let mut stdout = io::stdout().lock();

        for mode in mode_rx {
            let byte = match mode {
                EditorMode::Identity => b'I',
                EditorMode::Gain => b'G',
            };

            if stdout.write_all(&[byte]).is_err() || stdout.flush().is_err() {
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
                        Timer::after(Duration::from_millis(250)).await;
                        if polled_status.closed.load(Ordering::Relaxed) {
                            let _ = cx.update(|cx| cx.quit());
                            break;
                        }
                        let mode = if polled_status.gain_mode.load(Ordering::Relaxed) {
                            EditorMode::Gain
                        } else {
                            EditorMode::Identity
                        };

                        if view
                            .update(cx, |this, cx| {
                                if this.mode != mode {
                                    this.select_mode(mode);
                                }
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    })
                    .detach();
                    EditorView {
                        status,
                        mode: EditorMode::Identity,
                        gain: 1.0,
                        gain_drag: None,
                        _embedding: connection,
                        mode_tx: mode_tx,
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
    use super::{gain_at, gain_label};
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
}
