use clap_probe::{
    comparison::*,
    hosting::{Parameter, Worker},
    measurement::*,
};
use gpui::{prelude::*, *};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

const TEXT: u32 = 0xe1e6ef;
const MUTED: u32 = 0x99a8bc;
const COLORS: [u32; 4] = [0x67c6ff, 0x74edc0, 0xffb65c, 0xeb83c9];
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Identity(usize),
    Conditions(usize),
    Parameter(usize, usize),
}
enum Command {
    Load(PathBuf, String),
    Set(u64, u32, f64),
    Sweep(RunId, Conditions),
    Quit,
}
enum Event {
    Loaded(usize, Vec<Parameter>, Option<u32>),
    Edited(usize, u64, Vec<Parameter>, Option<u32>),
    Error(usize, String),
    Progress(RunId, usize, usize),
    Finished(RunId, usize, Outcome, Option<u32>),
}
struct Slot {
    path: Option<PathBuf>,
    identity: String,
    parameters: Vec<Parameter>,
    latency: Option<u32>,
    loaded: bool,
    loading: bool,
    pending_edits: usize,
    running: Option<RunId>,
    revision: u64,
    message: String,
    tx: mpsc::Sender<Command>,
    cancel: Arc<AtomicBool>,
}
fn slot(index: usize, events: mpsc::Sender<Event>) -> Slot {
    let (tx, commands) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancelled = cancel.clone();
    std::thread::spawn(move || {
        let mut worker: Option<Worker> = None;
        while let Ok(command) = commands.recv() {
            match command {
                Command::Load(path, id) => {
                    if let Some(mut old) = worker.take() {
                        let _ = old.shutdown();
                    }
                    match Worker::spawn(
                        &std::env::current_exe().unwrap(),
                        &path,
                        &id,
                        Duration::from_secs(5),
                    ) {
                        Ok(mut loaded) => {
                            loaded.cancel = cancelled.clone();
                            let _ = events.send(Event::Loaded(
                                index,
                                loaded.parameters.clone(),
                                loaded.latency,
                            ));
                            worker = Some(loaded);
                        }
                        Err(error) => {
                            let _ = events.send(Event::Error(index, error.to_string()));
                        }
                    }
                }
                Command::Set(revision, id, value) => {
                    if let Some(worker) = worker.as_mut() {
                        match worker.set(id, value) {
                            Ok(()) => {
                                let _ = events.send(Event::Edited(
                                    index,
                                    revision,
                                    worker.parameters.clone(),
                                    worker.latency,
                                ));
                            }
                            Err(error) => {
                                let _ = events.send(Event::Error(index, error.to_string()));
                            }
                        }
                    }
                }
                Command::Sweep(run, conditions) => {
                    let outcome = if let Some(worker) = worker.as_mut() {
                        let before = worker
                            .parameters
                            .iter()
                            .map(|p| (p.id, p.value))
                            .collect::<Vec<_>>();
                        let measured = measure_with_progress(&conditions, worker, |count| {
                            let _ = events.send(Event::Progress(run, index, count));
                        });
                        match measured {
                            Ok(measured) => match worker.info() {
                                Ok(())
                                    if before
                                        == worker
                                            .parameters
                                            .iter()
                                            .map(|p| (p.id, p.value))
                                            .collect::<Vec<_>>() =>
                                {
                                    Outcome::Complete(measured)
                                }
                                Ok(()) => Outcome::PluginFailure(
                                    "Plugin changed exposed settings during sweep".into(),
                                ),
                                Err(error) => Outcome::PluginFailure(error.to_string()),
                            },
                            Err(MeasureError::ProcessorFailure(error))
                                if error.contains("Timeout") =>
                            {
                                Outcome::Timeout
                            }
                            Err(error) => Outcome::PluginFailure(format!("{error:?}")),
                        }
                    } else {
                        Outcome::PluginFailure("No loaded worker".into())
                    };
                    let latency = worker.as_ref().and_then(|w| w.latency);
                    let _ = events.send(Event::Finished(run, index, outcome, latency));
                }
                Command::Quit => {
                    if let Some(mut worker) = worker {
                        let _ = worker.shutdown();
                    }
                    break;
                }
            }
        }
    });
    Slot {
        path: None,
        identity: String::new(),
        parameters: vec![],
        latency: None,
        loaded: false,
        loading: false,
        pending_edits: 0,
        running: None,
        revision: 0,
        message: "Choose a trusted binary and enter its exact CLAP ID".into(),
        tx,
        cancel,
    }
}
struct Inspector {
    slots: [Slot; 2],
    state: RunState,
    conditions: Conditions,
    fields: [String; 5],
    events: mpsc::Receiver<Event>,
    focus: FocusHandle,
    editing: Option<Field>,
    buffer: String,
    message: String,
    run_latency: [Option<u32>; 2],
}
impl Inspector {
    fn new(cx: &mut Context<Self>) -> Self {
        let (tx, events) = mpsc::channel();
        cx.spawn(async move |view, cx| loop {
            Timer::after(Duration::from_millis(30)).await;
            if view
                .update(cx, |this: &mut Self, cx| {
                    this.poll();
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
        Self { slots: [slot(0, tx.clone()), slot(1, tx)], state: RunState::default(),
            conditions: Conditions::default(), fields: ["375, 997, 6000".into(), "-36".into(), "4096".into(), "8192".into(), "16384".into()],
            events, focus: cx.focus_handle(), editing: None, buffer: String::new(),
            message: "Trusted local CLAP only · no audio device · select each binary and exact ID, then Load".into(), run_latency: [None; 2] }
    }
    fn settings(&self, slot: usize) -> Settings {
        let s = &self.slots[slot];
        Settings {
            identity: format!(
                "{} | {}",
                s.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                s.identity
            ),
            parameters: s.parameters.iter().map(|p| (p.id, p.value)).collect(),
        }
    }
    fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Loaded(i, parameters, latency) => {
                    self.slots[i].parameters = parameters;
                    self.slots[i].latency = latency;
                    self.slots[i].loading = false;
                    self.slots[i].loaded = true;
                    self.slots[i].message = "Loaded in isolated worker".into();
                    self.state.set_slot(i, self.settings(i));
                }
                Event::Edited(i, revision, parameters, latency) => {
                    self.slots[i].pending_edits = self.slots[i].pending_edits.saturating_sub(1);
                    self.slots[i].latency = latency;
                    if revision == self.slots[i].revision {
                        self.slots[i].parameters = parameters;
                        self.state.set_slot(i, self.settings(i));
                        self.slots[i].message = "Parameter applied and read back".into();
                    }
                }
                Event::Error(i, error) => {
                    self.slots[i].loading = false;
                    self.slots[i].pending_edits = 0;
                    self.slots[i].loaded = false;
                    self.slots[i].message = format!("{error} · reload required");
                }
                Event::Progress(run, i, count) => self.state.progress(run, i, count),
                Event::Finished(run, i, outcome, latency) => {
                    if self.slots[i].running != Some(run) {
                        continue;
                    }
                    self.slots[i].running = None;
                    if self.state.view().run == Some(run) {
                        self.run_latency[i] = latency;
                    }
                    if !matches!(outcome, Outcome::Complete(_))
                        && !self.slots[i].cancel.load(Ordering::Acquire)
                    {
                        self.slots[i].loaded = false;
                        self.slots[i].message = "Worker failed · reload required".into();
                    }
                    self.state.finish(run, i, outcome);
                }
            }
        }
    }
    fn can_replace(&self, slot: usize) -> bool {
        let s = &self.slots[slot];
        self.state.view().status != Status::Running
            && !s.loading
            && s.running.is_none()
            && s.pending_edits == 0
    }
    fn choose(&mut self, slot: usize, cx: &mut Context<Self>) {
        if !self.can_replace(slot) {
            self.message = "Wait for slot commands to finish before replacing it".into();
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select trusted Linux CLAP binary".into()),
        });
        cx.spawn(async move |view, cx| {
            let result = paths.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(paths))) if this.can_replace(slot) => {
                        if let Some(path) = paths.first() {
                            this.slots[slot].path = Some(path.clone());
                            this.slots[slot].loaded = false;
                            this.state.set_slot(slot, this.settings(slot));
                        }
                    }
                    Ok(Ok(_)) => {}
                    other => this.message = format!("File chooser: {other:?}"),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn load(&mut self, slot: usize) {
        if !self.can_replace(slot) {
            self.message = "Cancel and wait for slot commands before replacing it".into();
            return;
        }
        self.editing = None;
        let s = &mut self.slots[slot];
        if let Some(path) = &s.path {
            if s.identity.trim().is_empty() {
                s.message = "Enter the exact plugin ID first".into();
                return;
            }
            s.loaded = false;
            s.loading = true;
            s.parameters.clear();
            s.message = "Loading… (5 second command deadline)".into();
            let _ = s.tx.send(Command::Load(path.clone(), s.identity.clone()));
        } else {
            s.message = "Select a binary first".into();
        }
        self.state.set_slot(slot, self.settings(slot));
    }
    fn start(&mut self) {
        if self
            .slots
            .iter()
            .any(|s| s.loading || s.pending_edits > 0 || s.running.is_some())
        {
            self.message =
                "Wait for loading, parameter readback, or cancelled work to finish".into();
            return;
        }
        if !self.slots.iter().all(|s| s.loaded) {
            self.message = "Load both slots before running".into();
            return;
        }
        if self.state.view().status == Status::Running {
            return;
        }
        self.state.set_conditions(self.conditions.clone());
        match self.state.start() {
            Ok(run) => {
                self.run_latency = [None; 2];
                for s in &mut self.slots {
                    s.running = Some(run);
                    s.cancel.store(false, Ordering::Release);
                    let _ = s.tx.send(Command::Sweep(run, self.conditions.clone()));
                }
                self.message = "Processing independently · raw gain/phase, no delay compensation".into();
            }
            Err(_) => self.message = "Invalid conditions: 48 kHz/256 only; positive sub-Nyquist frequencies, ≥8 observed cycles; durations positive multiples of 256; level −100…0 dBFS peak".into(),
        }
    }
    fn cancel(&mut self) {
        self.state.cancel();
        for s in &self.slots {
            s.cancel.store(true, Ordering::Release);
        }
        self.message =
            "Cancelled · completed results retained; incomplete work cannot publish".into();
    }
    fn begin_edit(&mut self, field: Field, value: String, window: &mut Window) {
        self.editing = Some(field);
        self.buffer = value;
        self.focus.focus(window);
    }
    fn commit(&mut self) {
        let Some(field) = self.editing.take() else {
            return;
        };
        let text = self.buffer.trim().to_owned();
        match field {
            Field::Identity(i) => {
                if !self.can_replace(i) {
                    self.message =
                        "Cancel and wait for slot commands before changing identity".into();
                    return;
                }
                self.slots[i].identity = text;
                self.slots[i].loaded = false;
                self.state.set_slot(i, self.settings(i));
            }
            Field::Conditions(i) => {
                self.fields[i] = text;
                let parsed = (|| -> Option<Conditions> {
                    Some(Conditions {
                        frequencies_hz: self.fields[0]
                            .split(',')
                            .map(|s| s.trim().parse())
                            .collect::<Result<Vec<_>, _>>()
                            .ok()?,
                        level_dbfs_peak: self.fields[1].parse().ok()?,
                        warmup_frames: self.fields[2].parse().ok()?,
                        settling_frames: self.fields[3].parse().ok()?,
                        observation_frames: self.fields[4].parse().ok()?,
                        ..self.conditions.clone()
                    })
                })();
                if let Some(c) = parsed {
                    self.conditions = c;
                } else {
                    self.conditions.frequencies_hz.clear();
                }
                self.state.set_conditions(self.conditions.clone());
            }
            Field::Parameter(i, p) => {
                if !self.slots[i].loaded {
                    self.message = "Load this slot before editing parameters".into();
                    return;
                }
                let parameter = &mut self.slots[i].parameters[p];
                match text.parse::<f64>() {
                    Ok(value) if value.is_finite() && value >= parameter.min && value <= parameter.max
                        && (!parameter.stepped || value.fract() == 0.) && !parameter.readonly => {
                        parameter.value = value; parameter.formatted = "pending readback".into();
                        let id = parameter.id;
                        self.slots[i].revision += 1;
                        self.slots[i].pending_edits += 1;
                        let _ = self.slots[i].tx.send(Command::Set(self.slots[i].revision, id, value));
                        self.state.set_slot(i, self.settings(i));
                    }
                    _ => self.message = "Invalid parameter: use its plain CLAP range (integer for stepped controls)".into(),
                }
            }
        }
    }
    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_none() {
            return;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "enter" => self.commit(),
            "escape" => self.editing = None,
            "backspace" => {
                self.buffer.pop();
            }
            "a" if event.keystroke.modifiers.control => self.buffer.clear(),
            "v" if event.keystroke.modifiers.control => {
                if let Some(item) = cx.read_from_clipboard() {
                    if let Some(text) = item.text() {
                        self.buffer.push_str(&text);
                    }
                }
            }
            "space" => self.buffer.push(' '),
            _ if key.chars().count() == 1 && !event.keystroke.modifiers.control => self
                .buffer
                .push_str(event.keystroke.key_char.as_deref().unwrap_or(key)),
            _ => {}
        }
        cx.notify();
    }
    fn input(&self, field: Field, value: String, cx: &mut Context<Self>) -> Stateful<Div> {
        let active = self.editing == Some(field);
        let display = if active {
            format!("{}▏", self.buffer)
        } else {
            value.clone()
        };
        div()
            .id(SharedString::from(format!(
                "field-{}",
                match field {
                    Field::Identity(i) => format!("id-{i}"),
                    Field::Conditions(i) => format!("c-{i}"),
                    Field::Parameter(i, p) => format!("p-{i}-{p}"),
                }
            )))
            .px_2()
            .py_1()
            .min_h(px(25.))
            .bg(rgb(if active { 0x263952 } else { 0x101722 }))
            .border_1()
            .border_color(rgb(if active { 0x67c6ff } else { 0x334157 }))
            .rounded_sm()
            .cursor_text()
            .child(display)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.begin_edit(field, value.clone(), window);
                cx.notify();
            }))
    }
    fn slot_view(&self, i: usize, cx: &mut Context<Self>) -> Div {
        let s = &self.slots[i];
        div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgb(0x1a2432))
            .rounded_md()
            .child(label(
                format!(
                    "SLOT {} · {}",
                    i + 1,
                    if s.loaded { "LOADED" } else { "NOT READY" }
                ),
                COLORS[i * 2],
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        button(format!("choose-{i}"), "Select binary…")
                            .on_click(cx.listener(move |this, _, _, cx| this.choose(i, cx))),
                    )
                    .child(label(
                        s.path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "No binary selected".into()),
                        MUTED,
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(label("Exact CLAP ID", MUTED))
                    .child(
                        self.input(Field::Identity(i), s.identity.clone(), cx)
                            .flex_1(),
                    )
                    .child(button(format!("load-{i}"), "Load").on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.load(i);
                            cx.notify();
                        },
                    ))),
            )
            .child(label(
                format!(
                    "{} · reported latency: {} samples",
                    s.message,
                    s.latency
                        .map(|x| x.to_string())
                        .unwrap_or_else(|| "not reported".into())
                ),
                MUTED,
            ))
            .child(
                div()
                    .id(SharedString::from(format!("params-{i}")))
                    .h(px(190.))
                    .overflow_y_scroll()
                    .children(s.parameters.iter().enumerate().map(|(p, parameter)| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .py_1()
                            .child(div().flex_1().child(format!(
                                "{}{}",
                                parameter.name,
                                if parameter.readonly {
                                    " (read only)"
                                } else {
                                    ""
                                }
                            )))
                            .child(label(format!("{}…{}", parameter.min, parameter.max), MUTED))
                            .child(
                                self.input(Field::Parameter(i, p), parameter.value.to_string(), cx)
                                    .w(px(110.)),
                            )
                            .child(div().w(px(115.)).child(parameter.formatted.clone()))
                    })),
            )
    }
}
impl Render for Inspector {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.state.view();
        let snapshot = view.snapshot.clone();
        let outcomes = view.outcomes.clone();
        div().id("inspector").track_focus(&self.focus).on_key_down(cx.listener(Self::key))
            .size_full().overflow_y_scroll().flex().flex_col().gap_3().p_4().bg(rgb(0x101722)).text_color(rgb(TEXT)).text_size(px(12.)).font_family("sans-serif")
            .child(div().flex().justify_between().child(label("PLUGIN INSPECTOR · stepped-sine comparison", TEXT))
                .child(label("Offline stereo · 48,000 Hz · 256 frames · no normalization", MUTED)))
            .child(div().flex().gap_3().child(self.slot_view(0,cx)).child(self.slot_view(1,cx)))
            .child(div().flex().flex_wrap().gap_3().items_center().children([
                ("Sampled Hz (comma-separated)",0), ("Input dBFS peak",1), ("Silent warm-up frames",2), ("Settling frames / tone",3), ("Observation frames / tone",4)
            ].into_iter().map(|(name,i)| div().flex().flex_col().gap_1().child(label(name,MUTED)).child(self.input(Field::Conditions(i),self.fields[i].clone(),cx)))))
            .child(div().flex().gap_3().items_center()
                .child(button("routing",format!("Excite {} only → measure L and R", if self.conditions.input_channel == 0 { "L" } else { "R" })).on_click(cx.listener(|this,_,_,cx| {
                    this.conditions.input_channel = 1-this.conditions.input_channel; this.state.set_conditions(this.conditions.clone()); cx.notify();
                })))
                .child(button("run","Run comparison").on_click(cx.listener(|this,_,_,cx| { this.commit(); this.start(); cx.notify(); })))
                .child(button("cancel","Cancel").on_click(cx.listener(|this,_,_,cx| { this.cancel(); cx.notify(); })))
                .child(label(format!("{:?} · progress A {} / B {} · stale A={} B={}",view.status,view.progress[0],view.progress[1],view.stale[0],view.stale[1]),TEXT)))
            .child(label(self.message.clone(),MUTED))
            .child(label("Click a value to edit · Ctrl+A clears · Ctrl+V pastes · Enter applies · Escape discards. Parameter edits during a run queue for the next run and mark its result stale.", MUTED))
            .child(label("Procedure: reset if supported → fixed silent warm-up → each tone settling then observation. Keep history across frequencies. Hidden state can survive reset; real-plugin repeats need not match.", MUTED))
            .child(div().flex().gap_4().children((0..4).map(|i| label(format!("● Slot {} output {}",i/2+1,if i%2==0 {"L"} else {"R"}),COLORS[i]))))
            .child(plot(&outcomes, true))
            .child(plot(&outcomes, false))
            .child(label("Discrete dots are actual samples on shared logarithmic frequency axes. Unavailable points are omitted, not zero. Phase is wrapped, uncompensated; reported latency is not removed.", MUTED))
            .children(snapshot.map(|s| {
                let details = format!("RUN SNAPSHOT · {:?}\nA: {}\nA plain settings: {:?}\nB: {}\nB plain settings: {:?}\nReported latency A {:?}, B {:?} samples (None = unavailable). Pure-delay fit is diagnostic, not general group delay.",s.conditions,s.slots[0].identity,s.slots[0].parameters,s.slots[1].identity,s.slots[1].parameters,self.run_latency[0],self.run_latency[1]);
                div().id("snapshot").max_h(px(140.)).overflow_y_scroll().child(details)
            }))
            .child(div().id("points").max_h(px(180.)).overflow_y_scroll().children(outcomes.iter().enumerate().map(|(i,outcome)| {
                let detail = match outcome {
                    Some(Outcome::Complete(m)) => format!("Slot {} · COMPLETE · reset supported={} · pure-delay fit {:?} samples\n{}\n{}", i+1,m.reset_supported,m.delay_samples,m.history_warning,m.points.iter().map(|p| format!("{} Hz: L {:?}; R {:?}",p.frequency_hz,p.channels[0],p.channels[1])).collect::<Vec<_>>().join("\n")),
                    other => format!("Slot {}: {other:?}",i+1),
                };
                div().py_2().child(detail)
            })))
    }
}
fn label(text: impl Into<SharedString>, color: u32) -> Div {
    div().text_color(rgb(color)).child(text.into())
}
fn button(id: impl Into<SharedString>, text: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id.into())
        .px_3()
        .py_2()
        .rounded_sm()
        .bg(rgb(0x30425c))
        .cursor_pointer()
        .child(text.into())
}
fn plot(outcomes: &[Option<Outcome>; 2], gain: bool) -> Div {
    let mut traces: Vec<Vec<(f64, f64)>> = vec![vec![]; 4];
    for (slot, outcome) in outcomes.iter().enumerate() {
        if let Some(Outcome::Complete(m)) = outcome {
            for p in &m.points {
                for ch in 0..2 {
                    if let Response::Available { gain_db, phase_deg } = p.channels[ch] {
                        traces[slot * 2 + ch]
                            .push((p.frequency_hz, if gain { gain_db } else { phase_deg }));
                    }
                }
            }
        }
    }
    let values: Vec<_> = traces.iter().flatten().copied().collect();
    let lo = values.iter().map(|v| v.0).fold(20., f64::min).max(0.001);
    let hi = values.iter().map(|v| v.0).fold(20000., f64::max);
    let ymin = if gain {
        values.iter().map(|v| v.1).fold(-24., f64::min).floor() - 3.
    } else {
        -180.
    };
    let ymax = if gain {
        values.iter().map(|v| v.1).fold(6., f64::max).ceil() + 3.
    } else {
        180.
    };
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(label(
            format!(
                "{} · shared Y {:.1}…{:.1} {}",
                if gain { "GAIN" } else { "PHASE (raw)" },
                ymin,
                ymax,
                if gain { "dB" } else { "degrees" }
            ),
            TEXT,
        ))
        .child(
            div()
                .flex()
                .gap_2()
                .child(
                    div()
                        .w(px(55.))
                        .h(px(155.))
                        .flex()
                        .flex_col()
                        .justify_between()
                        .child(format!("{ymax:.0}"))
                        .child(format!("{:.0}", (ymax + ymin) / 2.))
                        .child(format!("{ymin:.0}")),
                )
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            let w: f32 = bounds.size.width.into();
                            let h: f32 = bounds.size.height.into();
                            let at = |x: f64, y: f64| {
                                bounds.origin
                                    + point(
                                        px((((x.ln() - lo.ln()) / (hi.ln() - lo.ln())) as f32)
                                            * (w - 8.)
                                            + 4.),
                                        px(((ymax - y) / (ymax - ymin)) as f32 * (h - 8.) + 4.),
                                    )
                            };
                            for t in 0..=4 {
                                let x = lo * (hi / lo).powf(t as f64 / 4.);
                                line(window, at(x, ymin), at(x, ymax), 0x2a384c, 1.);
                                let y = ymin + (ymax - ymin) * t as f64 / 4.;
                                line(window, at(lo, y), at(hi, y), 0x2a384c, 1.);
                            }
                            for (i, trace) in traces.iter().enumerate() {
                                for &(x, y) in trace {
                                    let p = at(x, y);
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            p - point(px(3.), px(3.)),
                                            size(px(6.), px(6.)),
                                        ),
                                        rgb(COLORS[i]),
                                    ));
                                }
                            }
                        },
                    )
                    .w_full()
                    .h(px(155.))
                    .bg(rgb(0x0b111b)),
                ),
        )
        .child(
            div()
                .ml(px(63.))
                .flex()
                .justify_between()
                .children((0..=4).map(|t| {
                    label(
                        format!("{:.0} Hz", lo * (hi / lo).powf(t as f64 / 4.)),
                        MUTED,
                    )
                })),
        )
}
fn line(
    window: &mut Window,
    a: gpui::Point<Pixels>,
    b: gpui::Point<Pixels>,
    color: u32,
    width: f32,
) {
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(a);
    path.line_to(b);
    if let Ok(path) = path.build() {
        window.paint_path(path, rgb(color));
    }
}
impl Drop for Inspector {
    fn drop(&mut self) {
        for slot in &self.slots {
            slot.cancel.store(true, Ordering::Release);
            let _ = slot.tx.send(Command::Quit);
        }
    }
}
pub fn run() {
    Application::new().run(|cx: &mut App| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1320.), px(1000.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(950.), px(700.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Plugin Inspector".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(Inspector::new),
        )
        .expect("Cannot open inspector window");
        cx.activate(true);
    });
}
