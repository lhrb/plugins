//! Standalone GPUI view. The host owns audio and authoritative parameter state.
use gpui::{prelude::*, *};
use nam_linux_port::protocol::{HostMessage, Snapshot, UiEvent};
use std::{
    io::{self, BufRead, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use x11rb::{connection::Connection, protocol::xproto::ConnectionExt};

const BLUE: u32 = 0x6098f4;
const TEXT: u32 = 0xe6e8eb;
const MUTED: u32 = 0x949aa5;
const IDS: [&str; 9] = [
    "input",
    "threshold",
    "bass",
    "middle",
    "treble",
    "output",
    "gate_enabled",
    "eq_enabled",
    "ir_enabled",
];
const LABELS: [&str; 6] = ["Input", "Threshold", "Bass", "Middle", "Treble", "Output"];
const DEFAULTS: [f32; 9] = [0.5, 0.2, 0.5, 0.5, 0.5, 0.5, 1., 1., 1.];

#[derive(Default)]
struct Inbox {
    latest: Mutex<Option<Snapshot>>,
    closed: AtomicBool,
}

struct Editor {
    values: [f32; 9],
    input_peak: f32,
    output_peak: f32,
    model: Option<String>,
    ir: Option<String>,
    status: String,
    drag: Option<(usize, f32, f32)>,
    settings: bool,
    preview: bool,
    // Keep the reparenting client alive for the lifetime of the embedded view.
    _embedding: Option<x11rb::rust_connection::RustConnection>,
}

impl Editor {
    fn new(
        preview: bool,
        inbox: Arc<Inbox>,
        embedding: Option<x11rb::rust_connection::RustConnection>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |view, cx| {
            loop {
                Timer::after(Duration::from_millis(33)).await;
                if inbox.closed.load(Ordering::Acquire) {
                    let _ = cx.update(|cx| cx.quit());
                    break;
                }
                let snapshot = inbox.latest.lock().unwrap().take();
                if let Some(snapshot) = snapshot {
                    if view
                        .update(cx, |this, cx| {
                            // Avoid fighting an active pointer gesture with an older host echo.
                            for (i, value) in snapshot.values.into_iter().enumerate() {
                                if this.drag.map(|d| d.0) != Some(i) && value.is_finite() {
                                    this.values[i] = value.clamp(0., 1.);
                                }
                            }
                            this.input_peak = snapshot.input_peak;
                            this.output_peak = snapshot.output_peak;
                            this.model = snapshot.model;
                            this.ir = snapshot.ir;
                            this.status = snapshot.status;
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        })
        .detach();
        Self {
            values: DEFAULTS,
            input_peak: 0.,
            output_peak: 0.,
            model: None,
            ir: None,
            status: if preview {
                "Preview · no audio connection".into()
            } else {
                "Connecting to audio host…".into()
            },
            drag: None,
            settings: false,
            preview,
            _embedding: embedding,
        }
    }

    fn send(&mut self, event: UiEvent) {
        if self.preview {
            return;
        }
        let result = (|| -> io::Result<()> {
            let mut out = io::stdout().lock();
            serde_json::to_writer(&mut out, &event)?;
            out.write_all(b"\n")?;
            out.flush()
        })();
        if let Err(error) = result {
            self.status = format!("Host connection: {error}");
        }
    }

    fn begin(&mut self, i: usize) {
        self.send(UiEvent::Begin { id: IDS[i].into() });
    }
    fn end(&mut self, i: usize) {
        self.send(UiEvent::End { id: IDS[i].into() });
    }
    fn set(&mut self, i: usize, value: f32) {
        self.values[i] = value.clamp(0., 1.);
        self.send(UiEvent::Set {
            id: IDS[i].into(),
            normalized: self.values[i],
        });
    }
    fn discrete(&mut self, i: usize, value: f32) {
        self.begin(i);
        self.set(i, value);
        self.end(i);
    }
    fn finish_drag(&mut self) {
        if let Some((i, _, _)) = self.drag.take() {
            self.end(i);
        }
    }

    fn choose(&mut self, ir: bool, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                if ir {
                    "Select impulse response (.wav)"
                } else {
                    "Select neural amp model (.nam)"
                }
                .into(),
            ),
        });
        cx.spawn(async move |view, cx| {
            let result = paths.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                            if !extension.eq_ignore_ascii_case(if ir { "wav" } else { "nam" }) {
                                this.status =
                                    format!("Choose a {} file", if ir { ".wav" } else { ".nam" });
                            } else if let Some(path) = path.to_str() {
                                if this.preview {
                                    if ir {
                                        this.ir = Some(path.into());
                                    } else {
                                        this.model = Some(path.into());
                                    }
                                } else {
                                    this.send(if ir {
                                        UiEvent::LoadIr { path: path.into() }
                                    } else {
                                        UiEvent::LoadModel { path: path.into() }
                                    });
                                }
                            } else {
                                this.status = "The file path is not valid UTF-8".into();
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.status = format!("File chooser: {error}"),
                    Err(error) => this.status = format!("File chooser: {error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn knob(&self, i: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let value = self.values[i];
        let display = match i {
            0 | 5 => format!("{:.1} dB", value * 48. - 24.),
            1 => format!("{:.1} dB", value * 100. - 100.),
            _ => format!("{:.1}", value * 10.),
        };
        div()
            .id(IDS[i])
            .w(px(87.))
            .flex()
            .flex_col()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.finish_drag();
                    if event.click_count >= 2 {
                        this.discrete(i, DEFAULTS[i]);
                    } else {
                        this.begin(i);
                        this.drag = Some((i, event.position.y.into(), this.values[i]));
                    }
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                let delta: f32 = event.delta.pixel_delta(px(20.)).y.into();
                let sensitivity = if event.modifiers.shift { 0.1 } else { 1. };
                this.discrete(i, this.values[i] + delta / 1200. * sensitivity);
                cx.stop_propagation();
                cx.notify();
            }))
            .child(text(LABELS[i], TEXT, 13.))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| draw_knob(bounds, window, value),
                )
                .w(px(82.))
                .h(px(92.)),
            )
            .child(text(display, TEXT, 13.))
    }

    fn toggle(&self, i: usize, label: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.values[i] >= 0.5;
        div()
            .id(IDS[i])
            .flex()
            .flex_col()
            .items_center()
            .gap(px(7.))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.discrete(i, if active { 0. } else { 1. });
                cx.notify();
            }))
            .child(
                div()
                    .w(px(46.))
                    .h(px(19.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(0x07080a))
                    .bg(rgb(if active { BLUE } else { 0x353940 }))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .ml(px(if active { 27. } else { 1. }))
                            .size(px(16.))
                            .rounded_full()
                            .bg(rgb(0x24282e))
                            .border_1()
                            .border_color(rgb(0x4a515d)),
                    ),
            )
            .child(text(label, TEXT, 12.))
    }

    fn selector(&self, ir: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let path = if ir { &self.ir } else { &self.model };
        let name = path
            .as_deref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                if ir {
                    "Select IR…".into()
                } else {
                    "Select model…".into()
                }
            });
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(38.))
            .child(text(if ir { "≋" } else { "▤" }, MUTED, 22.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .w(px(458.))
                    .h(px(28.))
                    .px(px(4.))
                    .bg(rgb(0x090a0c))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(0x35383d))
                    .child(
                        button(if ir { "ir-folder" } else { "model-folder" }, "▱")
                            .on_click(cx.listener(move |this, _, _, cx| this.choose(ir, cx))),
                    )
                    .child(
                        button(if ir { "ir-prev" } else { "model-prev" }, "‹").on_click(
                            cx.listener(move |this, _, _, cx| {
                                if this.preview {
                                    this.status =
                                        "Adjacent-file navigation requires the audio host".into();
                                } else {
                                    this.send(if ir {
                                        UiEvent::PreviousIr
                                    } else {
                                        UiEvent::PreviousModel
                                    });
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button(if ir { "ir-next" } else { "model-next" }, "›").on_click(
                            cx.listener(move |this, _, _, cx| {
                                if this.preview {
                                    this.status =
                                        "Adjacent-file navigation requires the audio host".into();
                                } else {
                                    this.send(if ir {
                                        UiEvent::NextIr
                                    } else {
                                        UiEvent::NextModel
                                    });
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        div()
                            .id(if ir { "ir-name" } else { "model-name" })
                            .flex_1()
                            .overflow_hidden()
                            .text_center()
                            .text_size(px(12.))
                            .cursor_pointer()
                            .child(name)
                            .on_click(cx.listener(move |this, _, _, cx| this.choose(ir, cx))),
                    )
                    .child(
                        button(if ir { "ir-clear" } else { "model-clear" }, "×").on_click(
                            cx.listener(move |this, _, _, cx| {
                                if ir {
                                    this.ir = None;
                                } else {
                                    this.model = None;
                                }
                                this.send(if ir {
                                    UiEvent::ClearIr
                                } else {
                                    UiEvent::ClearModel
                                });
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(if ir {
                self.toggle(8, "IR", cx).into_any_element()
            } else {
                text("NAM", MUTED, 10.).into_any_element()
            })
    }
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = self.input_peak;
        let output = self.output_peak;
        div().size_full().bg(rgb(0x17191c)).p(px(14.)).font_family("sans-serif").text_color(rgb(TEXT))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if let Some((i, y, initial)) = this.drag {
                    let current: f32 = event.position.y.into();
                    this.set(i, initial + (y - current) / if event.modifiers.shift { 1800. } else { 180. }); cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.finish_drag()))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| this.finish_drag()))
            .child(div().size_full().relative().flex().flex_col().rounded(px(36.)).border_2().border_color(rgb(0x33363b)).bg(rgb(0x1d1f23)).shadow_lg().overflow_hidden()
                .child(div().h(px(77.)).flex_shrink_0().flex().items_center().justify_center().relative()
                    .child(div().flex().items_baseline().gap(px(12.)).child(text("NAM", TEXT, 29.)).child(text("/ LINUX", MUTED, 19.)))
                    .child(div().absolute().right(px(22.)).top(px(17.)).child(button("settings", "⚙").on_click(cx.listener(|this, _, _, cx| { this.settings = !this.settings; cx.notify(); })))))
                .child(div().relative().h(px(217.)).flex_shrink_0()
                    .child(div().absolute().left(px(4.)).top(px(0.)).child(meter(input)))
                    .child(div().absolute().right(px(4.)).top(px(0.)).child(meter(output)))
                    .child(div().flex().justify_center().mt(px(11.)).children((0..6).map(|i| self.knob(i, cx))))
                    .child(div().absolute().top(px(163.)).left(px(132.)).child(self.toggle(6, "Noise Gate", cx)))
                    .child(div().absolute().top(px(163.)).left(px(337.)).child(self.toggle(7, "EQ", cx))))
                .child(div().flex_1().border_t_2().border_color(rgb(0x08090b)).bg(rgb(0x202226)).px(px(43.)).pt(px(10.))
                    .child(self.selector(false, cx)).child(self.selector(true, cx))
                    .child(div().mt(px(1.)).overflow_hidden().text_ellipsis().child(text(self.status.clone(), MUTED, 9.))))
                .when(self.settings, |el| el.child(div().absolute().top(px(56.)).right(px(20.)).w(px(292.)).p(px(15.)).rounded(px(8.)).bg(rgb(0x292d34)).border_1().border_color(rgb(0x596273)).shadow_lg()
                    .child(text("NEURAL AMP MODELER / LINUX", TEXT, 12.))
                    .child(text("Independent Linux port; not an official NAM release. The DAW controls the sample rate. Models and IRs must support the active host rate.", MUTED, 11.))
                    .child(text("Drag vertically · Shift for fine control · Scroll to adjust · Double-click to reset", BLUE, 11.)))))
    }
}

fn text(value: impl Into<SharedString>, color: u32, font_size: f32) -> Div {
    div()
        .text_size(px(font_size))
        .text_color(rgb(color))
        .child(value.into())
}
fn button(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(23.))
        .h(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .cursor_pointer()
        .text_size(px(20.))
        .text_color(rgb(TEXT))
        .hover(|s| s.bg(rgb(0x353b45)))
        .child(label)
}

fn meter(peak: f32) -> impl IntoElement {
    let db = if peak.is_finite() && peak > 0. {
        20. * peak.log10()
    } else {
        -60.
    };
    let level = ((db + 60.) / 60.).clamp(0., 1.);
    div()
        .w(px(19.))
        .h(px(207.))
        .p(px(3.))
        .rounded(px(3.))
        .border_1()
        .border_color(rgb(0x373a40))
        .bg(rgb(0x070809))
        .flex()
        .flex_col()
        .justify_end()
        .gap(px(1.))
        .children((0..60).rev().map(move |i| {
            div()
                .h(px(2.))
                .w_full()
                .bg(rgb(if (i as f32) < level * 60. {
                    if i >= 57 {
                        0xe87575
                    } else {
                        BLUE
                    }
                } else {
                    0x171e2a
                }))
        }))
}

fn draw_knob(bounds: Bounds<Pixels>, window: &mut Window, value: f32) {
    let origin = bounds.origin;
    let pt = |x: f32, y: f32| origin + point(px(x), px(y));
    let circle = |window: &mut Window, x: f32, y: f32, radius: f32, color: u32| {
        let mut path = PathBuilder::fill();
        for i in 0..=80 {
            let angle = i as f32 * std::f32::consts::TAU / 80.;
            let p = pt(x + radius * angle.cos(), y + radius * angle.sin());
            if i == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, rgb(color));
        }
    };
    circle(window, 42., 48., 34., 0x141517);
    for (radius, color) in [
        (33., 0x08090b),
        (30., 0x44484d),
        (28., 0x14161a),
        (25., 0x272b30),
        (22., 0x2b2f35),
    ] {
        circle(window, 41., 44., radius, color);
    }
    for (fraction, color, width) in [(1., 0x414957, 1.), (value, BLUE, 2.)] {
        let mut arc = PathBuilder::stroke(px(width));
        for i in 0..=80 {
            let angle = (135. + fraction * 270. * i as f32 / 80.).to_radians();
            let p = pt(41. + 35. * angle.cos(), 44. + 35. * angle.sin());
            if i == 0 {
                arc.move_to(p);
            } else {
                arc.line_to(p);
            }
        }
        if let Ok(path) = arc.build() {
            window.paint_path(path, rgb(color));
        }
    }
    let angle = (135. + value * 270.).to_radians();
    circle(
        window,
        41. + 20. * angle.cos(),
        44. + 20. * angle.sin(),
        2.4,
        BLUE,
    );
}

fn embed(
    parent: u32,
    scale: f32,
) -> Result<x11rb::rust_connection::RustConnection, Box<dyn std::error::Error>> {
    let (connection, screen) = x11rb::connect(None)?;
    // GPUI 0.2.2's X11 HasWindowHandle implementation panics. Locate our sole
    // window by its process property instead, including any WM frame ancestors.
    let pid_atom = connection.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
    let mut pending = vec![connection.setup().roots[screen].root];
    let mut child = None;
    while let Some(window) = pending.pop() {
        if let Ok(property) = connection
            .get_property(
                false,
                window,
                pid_atom,
                x11rb::protocol::xproto::AtomEnum::CARDINAL,
                0,
                1,
            )?
            .reply()
        {
            if property.value32().and_then(|mut values| values.next()) == Some(std::process::id()) {
                child = Some(window);
                break;
            }
        }
        if let Ok(tree) = connection.query_tree(window)?.reply() {
            pending.extend(tree.children);
        }
    }
    let child = child.ok_or("Cannot find the GPUI X11 window for this process")?;
    // GPUI maps before exposing the view. Let the desktop WM withdraw it
    // before changing it into an unmanaged child of the plugin host.
    connection.unmap_window(child)?.check()?;
    connection.flush()?;
    std::thread::sleep(Duration::from_millis(100));
    connection
        .change_window_attributes(
            child,
            &x11rb::protocol::xproto::ChangeWindowAttributesAux::new().override_redirect(1),
        )?
        .check()?;
    connection
        .configure_window(
            child,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .width((660. * scale).round() as u32)
                .height((430. * scale).round() as u32),
        )?
        .check()?;
    // A WM may still process the original top-level MapRequest and reparent
    // back to root. Retry until its pending management requests have settled.
    for _ in 0..50 {
        connection.reparent_window(child, parent, 0, 0)?.check()?;
        connection.map_window(child)?.check()?;
        connection.flush()?;
        std::thread::sleep(Duration::from_millis(20));
        if connection.query_tree(child)?.reply()?.parent == parent {
            return Ok(connection);
        }
    }
    Err("The window manager prevented embedding the editor".into())
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut parent = None;
    let mut preview = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--preview" => preview = true,
            "--parent" => {
                parent = Some(
                    args.next()
                        .ok_or("--parent requires an X11 window ID")?
                        .parse::<u32>()?,
                )
            }
            _ => return Err(format!("Unknown argument: {arg}").into()),
        }
    }
    if !preview && parent.is_none() {
        return Err("Use --parent <X11 window ID> or --preview".into());
    }
    if preview && parent.is_some() {
        return Err("--preview and --parent are mutually exclusive".into());
    }
    let inbox = Arc::new(Inbox::default());
    if !preview {
        let inbox = inbox.clone();
        std::thread::spawn(move || {
            for line in io::stdin().lock().lines() {
                match line {
                    Ok(line) => match serde_json::from_str::<HostMessage>(&line) {
                        Ok(HostMessage::Snapshot(snapshot)) => {
                            *inbox.latest.lock().unwrap() = Some(snapshot)
                        }
                        Err(error) => eprintln!("NAM editor protocol: {error}"),
                    },
                    Err(error) => {
                        eprintln!("NAM editor stdin: {error}");
                        break;
                    }
                }
            }
            inbox.closed.store(true, Ordering::Release);
        });
    }
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
                kind: if preview {
                    WindowKind::Normal
                } else {
                    WindowKind::PopUp
                },
                is_resizable: false,
                is_movable: preview,
                is_minimizable: false,
                app_id: Some("nam-linux-editor".into()),
                ..Default::default()
            },
            move |window, cx| {
                let embedding = parent.map(|parent| {
                    embed(parent, window.scale_factor()).unwrap_or_else(|error| {
                        eprintln!("NAM editor embedding: {error}");
                        std::process::exit(1);
                    })
                });
                cx.new(|cx| Editor::new(preview, inbox, embedding, cx))
            },
        );
        if let Err(error) = result {
            eprintln!("NAM editor window: {error}");
            cx.quit();
        }
        if preview {
            cx.activate(true);
        }
    });
    Ok(())
}
