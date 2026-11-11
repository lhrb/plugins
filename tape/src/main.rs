mod preview;

use gpui::{prelude::*, *};
use preview::{Controls, Preview};
use std::{
    f32::consts::PI,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Instant,
};

const INK: u32 = 0x191b1a;
const CREAM: u32 = 0xe7dfcc;
const MUTED: u32 = 0x8b8e82;
const AMBER: u32 = 0xe5ab62;

#[derive(Clone, Copy)]
enum Control {
    Drive,
    Tone,
    Wow,
    Output,
    Mix,
}
impl Control {
    fn name(self) -> &'static str {
        match self {
            Self::Drive => "RECORD LEVEL",
            Self::Tone => "TONE",
            Self::Wow => "WOW / FLUTTER",
            Self::Output => "OUTPUT",
            Self::Mix => "BLEND",
        }
    }
    fn range(self) -> (f32, f32, f32) {
        match self {
            Self::Drive => (0., 24., 6.),
            Self::Tone => (-1., 1., 0.),
            Self::Wow => (0., 1., 0.15),
            Self::Output => (-24., 6., -6.),
            Self::Mix => (0., 1., 1.),
        }
    }
    fn atom(self, c: &Controls) -> &AtomicU32 {
        match self {
            Self::Drive => &c.drive_db,
            Self::Tone => &c.tone,
            Self::Wow => &c.wow,
            Self::Output => &c.output_db,
            Self::Mix => &c.mix,
        }
    }
    fn get(self, c: &Controls) -> f32 {
        f32::from_bits(self.atom(c).load(Ordering::Relaxed))
    }
    fn set(self, c: &Controls, value: f32) {
        let (lo, hi, _) = self.range();
        self.atom(c)
            .store(value.clamp(lo, hi).to_bits(), Ordering::Relaxed);
    }
    fn display(self, c: &Controls) -> String {
        let v = self.get(c);
        match self {
            Self::Drive | Self::Output => format!("{v:+.1} dB"),
            Self::Tone if v.abs() < 0.01 => "NEUTRAL".into(),
            Self::Tone => format!("{v:+.2}"),
            _ => format!("{:.0} %", v * 100.),
        }
    }
}

struct TapeDeck {
    controls: Arc<Controls>,
    _audio: Option<Preview>,
    audio_error: Option<String>,
    drag: Option<(Control, f32, f32)>,
    started: Instant,
}

impl TapeDeck {
    fn new() -> Self {
        match Preview::new() {
            Ok(audio) => Self {
                controls: audio.controls.clone(),
                _audio: Some(audio),
                audio_error: None,
                drag: None,
                started: Instant::now(),
            },
            Err(error) => {
                eprintln!("OXIDE audition unavailable: {error}");
                Self {
                    controls: Arc::new(Controls::default()),
                    _audio: None,
                    audio_error: Some(error),
                    drag: None,
                    started: Instant::now(),
                }
            }
        }
    }

    fn knob(&self, control: Control, cx: &mut Context<Self>) -> impl IntoElement {
        let (lo, hi, _) = control.range();
        let normalized = (control.get(&self.controls) - lo) / (hi - lo);
        div()
            .id(control.name())
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .w(px(150.))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        control.set(&this.controls, control.range().2);
                    } else {
                        this.drag = Some((
                            control,
                            event.position.y.into(),
                            control.get(&this.controls),
                        ));
                    }
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                let delta: f32 = event.delta.pixel_delta(px(20.)).y.into();
                control.set(
                    &this.controls,
                    control.get(&this.controls) + delta * (hi - lo) / 1400.,
                );
                cx.stop_propagation();
                cx.notify();
            }))
            .child(label(control.name(), MUTED, 10.))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let mut p = Painter::new(window, bounds, 120., 110.);
                        p.circle(60., 58., 43., 0x101211);
                        for i in 0..=30 {
                            let a = (135. + i as f32 * 9.).to_radians();
                            let active = i as f32 / 30. <= normalized;
                            p.line(
                                60. + 48. * a.cos(),
                                55. + 48. * a.sin(),
                                60. + 51. * a.cos(),
                                55. + 51. * a.sin(),
                                1.3,
                                if active { AMBER } else { 0x44473f },
                            );
                        }
                        p.circle(60., 55., 39., 0x090b0a);
                        p.circle(60., 53., 36., 0x55584f);
                        p.circle(60., 54., 34., 0x292c28);
                        p.circle(60., 54., 29., 0x33362f);
                        p.circle(60., 54., 27., 0x2b2e28);
                        for i in 0..60 {
                            let a = i as f32 * PI / 30.;
                            p.line(
                                60. + 30. * a.cos(),
                                54. + 30. * a.sin(),
                                60. + 33. * a.cos(),
                                54. + 33. * a.sin(),
                                0.6,
                                0x51544a,
                            );
                        }
                        let a = (135. + normalized * 270.).to_radians();
                        p.line(
                            60. + 18. * a.cos(),
                            54. + 18. * a.sin(),
                            60. + 28. * a.cos(),
                            54. + 28. * a.sin(),
                            2.8,
                            CREAM,
                        );
                        p.circle(60., 54., 2., 0x3e4238);
                    },
                )
                .w(px(120.))
                .h(px(110.)),
            )
            .child(label(control.display(&self.controls), CREAM, 13.))
    }

    fn switch(&self, text: &'static str, active: bool) -> Stateful<Div> {
        div()
            .id(text)
            .px_4()
            .py_2()
            .rounded_sm()
            .border_1()
            .border_color(rgb(if active { 0x8b7046 } else { 0x42463b }))
            .bg(rgb(if active { 0x3d3829 } else { 0x242722 }))
            .text_color(rgb(if active { AMBER } else { MUTED }))
            .text_size(px(11.))
            .cursor_pointer()
            .child(text)
    }
}

impl Render for TapeDeck {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let playing = self.controls.playing.load(Ordering::Relaxed);
        let bypass = self.controls.bypass.load(Ordering::Relaxed);
        let fast = self.controls.speed_30_ips.load(Ordering::Relaxed);
        let meter = f32::from_bits(self.controls.meter.load(Ordering::Relaxed));
        if playing {
            window.request_animation_frame();
        }
        let angle = if playing {
            self.started.elapsed().as_secs_f32() * if fast { 1.1 } else { 0.55 }
        } else {
            0.
        };
        let has_audio = self.audio_error.is_none();
        div().size_full().flex().items_center().justify_center().bg(rgb(0x281b14)).p(px(15.)).font_family("sans-serif").text_color(rgb(CREAM))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if let Some((control, y, initial)) = this.drag {
                    let current: f32 = event.position.y.into();
                    let (lo,hi,_) = control.range();
                    let sensitivity = if event.modifiers.shift { 0.1 } else { 1. };
                    control.set(&this.controls, initial + (y-current)*(hi-lo)/180.*sensitivity);
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this,_,_,_| this.drag = None))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this,_,_,_| this.drag = None))
            .child(div().w(px(1010.)).h(px(740.)).flex_shrink_0().flex().flex_col().bg(rgb(INK)).border_1().border_color(rgb(0x626355)).rounded_md().shadow_xl()
                .child(div().flex().items_center().justify_between().px_8().h(px(88.)).border_b_1().border_color(rgb(0x363a32))
                    .child(div().flex().items_center().gap_4()
                        .child(div().flex().items_end().gap_1().child(label("OXIDE",CREAM,36.)).child(label("●",AMBER,14.)))
                        .child(div().w(px(1.)).h(px(32.)).bg(rgb(0x474b40)))
                        .child(div().flex().flex_col().gap_1().child(label("MAGNETIC TAPE SATURATION",CREAM,11.)).child(label("STEREO  /  TWO-TRACK RECORDER",MUTED,9.))))
                    .child(div().flex().flex_col().items_end().gap_1().child(label("TAPE WORKS",AMBER,12.)).child(label("MODEL 01  ·  GPUI PREVIEW",MUTED,9.))))
                .child(div().relative().mx_6().mt_5().rounded_sm().border_1().border_color(rgb(0x42463c)).bg(rgb(0x111512))
                    .child(div().flex().justify_between().px_4().pt_3().child(label("PRECISION MAGNETIC TRANSPORT",MUTED,9.)).child(label("¼ INCH  ·  LOW NOISE  ·  HIGH OUTPUT",MUTED,9.)))
                    .child(canvas(|_,_,_| (), move |bounds,_,window,_| {
                        let mut p = Painter::new(window,bounds,940.,275.);
                        // Subtle machining lines and recessed transport plate.
                        for y in (5..270).step_by(4) { p.line(0.,y as f32,940.,y as f32,0.35,0x1b201b); }
                        p.line(64.,202.,414.,202.,3.,0x5b3925);
                        p.line(64.,202.,116.,241.,2.,0x714b32);
                        p.line(116.,241.,362.,241.,2.,0x714b32);
                        p.line(362.,241.,414.,202.,2.,0x714b32);
                        draw_reel(&mut p,145.,125.,103.,angle,false);
                        draw_reel(&mut p,375.,125.,103.,angle,true);
                        for x in [116.,362.] { p.circle(x,240.,12.,0x73786b); p.circle(x,240.,8.,0x282e27); p.circle(x,240.,3.,0x9fa28e); }
                        p.rect(216.,218.,86.,34.,0x666b5c);
                        p.rect(222.,220.,74.,19.,0x30372d);
                        p.line(232.,230.,286.,230.,2.,0x9b9d85);
                        p.circle(259.,246.,2.,0x111810);
                        // Twin illuminated VU meters with calibrated arcs.
                        for (i,level) in [meter,meter*0.94].into_iter().enumerate() {
                            let x = 528.+i as f32*192.;
                            p.rect(x-5.,33.,186.,173.,0x080c09);
                            p.rect(x,38.,176.,163.,0x9c875d);
                            p.rect(x+3.,41.,170.,157.,0xdbc58e);
                            for y in 0..40 { let shade = (0xc7_u32 + (y/3) as u32).min(0xdf); p.rect(x+4.,42.+y as f32,168.,1.,(shade<<16)|((shade-18)<<8)|(shade-62)); }
                            for tick in 0..=24 {
                                let a = (211.+tick as f32*4.9).to_radians();
                                let r = if tick%4==0 { 87. } else { 91. };
                                p.line(x+88.+r*a.cos(),183.+r*a.sin(),x+88.+98.*a.cos(),183.+98.*a.sin(),if tick%4==0 {1.7} else {0.8},if tick>=18 {0x9b3c28} else {0x4b4936});
                            }
                            let db = 20.*level.max(0.00001).log10();
                            let a = (214.+((db+36.)/39.).clamp(0.,1.)*110.).to_radians();
                            p.line(x+88.,183.,x+88.+93.*a.cos(),183.+93.*a.sin(),1.5,0x943e28);
                            p.circle(x+88.,183.,5.,0x403d2e);
                            p.rect(x+4.,187.,168.,10.,0xb2a073);
                        }
                        p.circle(551.,237.,4.,if playing {0xe5ab62} else {0x51452f});
                        p.line(574.,237.,882.,237.,0.7,0x3c4438);
                        // Corner fasteners.
                        for (x,y) in [(14.,12.),(926.,12.),(14.,260.),(926.,260.)] { p.circle(x,y,4.,0x424b3e); p.line(x-2.,y+1.,x+2.,y-1.,1.,0x141a13); }
                    }).w_full().h(px(275.)))
                    .child(div().absolute().top(px(155.)).left(px(613.)).flex().gap(px(162.)).child(label("VU",0x514a33,19.)).child(label("VU",0x514a33,19.)))
                    .child(div().flex().justify_between().px_5().pb_3().child(label("FULL-TRACK STEREO HEAD ASSEMBLY",MUTED,9.)).child(label("L    −20   −10   −5    0   +3       R    −20   −10   −5    0   +3",MUTED,9.))))
                .child(div().flex().justify_around().px_5().py_5()
                    .child(self.knob(Control::Drive,cx)).child(self.knob(Control::Tone,cx))
                    .child(self.knob(Control::Wow,cx)).child(self.knob(Control::Output,cx)).child(self.knob(Control::Mix,cx)))
                .child(div().mx_8().h(px(1.)).bg(rgb(0x373c32)))
                .child(div().flex().items_center().justify_between().px_8().py_4()
                    .child(div().flex().items_center().gap_2().child(label("TAPE SPEED",MUTED,10.))
                        .child(self.switch("15 IPS",!fast).on_click(cx.listener(|this,_,_,cx| { this.controls.speed_30_ips.store(false,Ordering::Relaxed); cx.notify(); })))
                        .child(self.switch("30 IPS",fast).on_click(cx.listener(|this,_,_,cx| { this.controls.speed_30_ips.store(true,Ordering::Relaxed); cx.notify(); }))))
                    .child(div().flex().gap_2()
                        .child(self.switch(if playing { "■ STOP DEMO" } else { "▶ PLAY DEMO" },playing).on_click(cx.listener(move |this,_,_,cx| {
                            if this.audio_error.is_none() { this.controls.playing.store(!playing,Ordering::Relaxed); cx.notify(); }
                        })))
                        .child(self.switch("BYPASS",bypass).on_click(cx.listener(move |this,_,_,cx| { this.controls.bypass.store(!bypass,Ordering::Relaxed); cx.notify(); })))))
                .child(div().flex().items_center().justify_between().px_8().pb_4()
                    .child(label(if has_audio { "STANDALONE AUDITION  ·  NOT CONNECTED TO A DAW" } else { "AUDIO DEVICE UNAVAILABLE  ·  UI PREVIEW ONLY" },MUTED,9.))
                    .child(label("DRAG TO ADJUST  /  SHIFT: FINE  /  DOUBLE-CLICK: RESET",MUTED,9.))))
    }
}

fn label(text: impl Into<SharedString>, color: u32, size: f32) -> Div {
    div()
        .text_size(px(size))
        .text_color(rgb(color))
        .child(text.into())
}

struct Painter<'a> {
    window: &'a mut Window,
    origin: Point<Pixels>,
    scale: f32,
}
impl<'a> Painter<'a> {
    fn new(window: &'a mut Window, bounds: Bounds<Pixels>, width: f32, height: f32) -> Self {
        let w: f32 = bounds.size.width.into();
        let h: f32 = bounds.size.height.into();
        let scale = (w / width).min(h / height);
        Self {
            window,
            origin: bounds.origin
                + point(px((w - width * scale) / 2.), px((h - height * scale) / 2.)),
            scale,
        }
    }
    fn pt(&self, x: f32, y: f32) -> Point<Pixels> {
        self.origin + point(px(x * self.scale), px(y * self.scale))
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        self.window.paint_quad(fill(
            Bounds::new(self.pt(x, y), size(px(w * self.scale), px(h * self.scale))),
            rgb(color),
        ));
    }
    fn circle(&mut self, x: f32, y: f32, r: f32, color: u32) {
        let mut path = PathBuilder::fill();
        for i in 0..=80 {
            let a = i as f32 * PI / 40.;
            let pt = self.pt(x + r * a.cos(), y + r * a.sin());
            if i == 0 {
                path.move_to(pt);
            } else {
                path.line_to(pt);
            }
        }
        if let Ok(path) = path.build() {
            self.window.paint_path(path, rgb(color));
        }
    }
    fn line(&mut self, x: f32, y: f32, xx: f32, yy: f32, width: f32, color: u32) {
        let mut path = PathBuilder::stroke(px(width * self.scale));
        path.move_to(self.pt(x, y));
        path.line_to(self.pt(xx, yy));
        if let Ok(path) = path.build() {
            self.window.paint_path(path, rgb(color));
        }
    }
}

fn draw_reel(p: &mut Painter<'_>, x: f32, y: f32, r: f32, angle: f32, takeup: bool) {
    p.circle(x, y + 3., r + 3., 0x080d09);
    p.circle(x, y, r, 0x686e5f);
    p.circle(x, y, r - 2., 0x343d31);
    p.circle(x, y, r - 8., 0x96785b);
    let tape_radius = if takeup { r - 28. } else { r - 14. };
    p.circle(x, y, tape_radius, 0x4d3426);
    for i in (20..tape_radius as usize).step_by(3) {
        // Fine concentric winding lines, behind the aluminum spider.
        let rr = i as f32;
        for k in 0..72 {
            let a = k as f32 * PI / 36.;
            let b = (k + 1) as f32 * PI / 36.;
            p.line(
                x + rr * a.cos(),
                y + rr * a.sin(),
                x + rr * b.cos(),
                y + rr * b.sin(),
                0.4,
                0x654730,
            );
        }
    }
    p.circle(x, y, r - 31., 0x9d9e88);
    p.circle(x, y, r - 34., 0x858b77);
    for i in 0..3 {
        let a = angle + i as f32 * PI * 2. / 3.;
        let hx = x + 42. * a.cos();
        let hy = y + 42. * a.sin();
        p.circle(hx, hy, 24., 0x303b2e);
        p.circle(hx, hy + 1., 21., 0x141e15);
        p.circle(hx, hy + 1., 17., 0x463125);
    }
    p.circle(x, y, 27., 0x4f594a);
    p.circle(x, y, 24., 0xa7ab93);
    p.circle(x, y, 20., 0x727c68);
    p.circle(x, y, 10., 0x232d21);
    p.circle(x, y, 5., 0xadb39a);
    p.line(x - 3., y, x + 3., y, 1., 0x3a4434);
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1040.), px(770.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(1040.), px(770.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("OXIDE — Tape Works".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(|_| TapeDeck::new()),
        )
        .expect("Could not open the OXIDE preview window");
        cx.activate(true);
    });
}
