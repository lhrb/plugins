use super::{
    assets::{load_asset, Assets, Change, Shared, Worker},
    domain::{db_gain, Controls, Domain},
};
use crate::features::model::NamModel;
use nih_plug::prelude::*;
use std::sync::{atomic::Ordering, Arc, Mutex};

#[derive(Params)]
pub struct NamParams {
    #[id = "input"]
    pub input: FloatParam,
    #[id = "threshold"]
    pub threshold: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "middle"]
    pub middle: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "gate_enabled"]
    pub gate_enabled: BoolParam,
    #[id = "eq_enabled"]
    pub eq_enabled: BoolParam,
    #[id = "ir_enabled"]
    pub ir_enabled: BoolParam,
    #[persist = "assets"]
    pub assets: Arc<Mutex<Assets>>,
}
impl Default for NamParams {
    fn default() -> Self {
        let db = |name, default, min, max| {
            FloatParam::new(name, default, FloatRange::Linear { min, max }).with_unit(" dB")
        };
        let tone = |name| {
            FloatParam::new(
                name,
                5.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 10.0,
                },
            )
        };
        Self {
            input: db("Input", 0.0, -24.0, 24.0),
            threshold: db("Gate threshold", -80.0, -100.0, 0.0),
            bass: tone("Bass"),
            middle: tone("Middle"),
            treble: tone("Treble"),
            output: db("Output", 0.0, -24.0, 24.0),
            gate_enabled: BoolParam::new("Gate enabled", true),
            eq_enabled: BoolParam::new("EQ enabled", true),
            ir_enabled: BoolParam::new("IR enabled", true),
            assets: Arc::new(Mutex::new(Assets::default())),
        }
    }
}
impl NamParams {
    pub fn normalized_values(&self) -> [f32; 9] {
        [
            self.input.unmodulated_normalized_value(),
            self.threshold.unmodulated_normalized_value(),
            self.bass.unmodulated_normalized_value(),
            self.middle.unmodulated_normalized_value(),
            self.treble.unmodulated_normalized_value(),
            self.output.unmodulated_normalized_value(),
            self.gate_enabled.unmodulated_normalized_value(),
            self.eq_enabled.unmodulated_normalized_value(),
            self.ir_enabled.unmodulated_normalized_value(),
        ]
    }
    fn controls(&self) -> Controls {
        Controls {
            input: self.input.value(),
            threshold: self.threshold.value(),
            bass: self.bass.value(),
            middle: self.middle.value(),
            treble: self.treble.value(),
            output: self.output.value(),
            gate_enabled: self.gate_enabled.value(),
            eq_enabled: self.eq_enabled.value(),
            ir_enabled: self.ir_enabled.value(),
        }
    }
}

pub struct NamLinux {
    pub params: Arc<NamParams>,
    pub shared: Arc<Shared>,
    // Join the worker before native DSP and the remaining shared queues are destroyed.
    _worker: Worker,
    model: Option<NamModel>,
    ir: Option<NamModel>,
    domain: Domain,
    input: Vec<f32>,
    left: Vec<f32>,
    right: Vec<f32>,
    ir_input: Vec<f32>,
    ir_left: Vec<f32>,
    ir_right: Vec<f32>,
    skip_initial_reset: bool,
}
impl Default for NamLinux {
    fn default() -> Self {
        let params = Arc::new(NamParams::default());
        let shared = Shared::new(params.assets.clone());
        Self {
            _worker: Worker::start(shared.clone()),
            params,
            shared,
            model: None,
            ir: None,
            domain: Domain::new(48000.0),
            input: Vec::new(),
            left: Vec::new(),
            right: Vec::new(),
            ir_input: Vec::new(),
            ir_left: Vec::new(),
            ir_right: Vec::new(),
            skip_initial_reset: false,
        }
    }
}
impl NamLinux {
    fn adopt(&mut self) {
        // Only this thread pushes retired objects; the worker only removes them.
        // Reserve retirement capacity BEFORE removing prepared work, so a full queue
        // never makes this thread destroy a model, allocation, or failed queue item.
        if self.shared.retired.is_full() {
            return;
        }
        if let Some(mut prepared) = self.shared.ready.pop() {
            if prepared.generation == self.shared.generation.load(Ordering::Acquire) {
                if let Change::Replace(ref mut model) = prepared.model {
                    std::mem::swap(&mut self.model, model);
                }
                if let Change::Replace(ref mut ir) = prepared.ir {
                    std::mem::swap(&mut self.ir, ir);
                }
                self.shared.processing_fault.store(false, Ordering::Relaxed);
            }
            // Capacity is guaranteed by the single producer contract above.
            if let Err(_impossible) = self.shared.retired.push(prepared) {
                unreachable!("retirement queue lost reserved capacity");
            }
        }
    }
}
impl Plugin for NamLinux {
    const NAME: &'static str = "NAM Linux";
    const VENDOR: &'static str = "NAM Linux contributors";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type SysExMessage = ();
    type BackgroundTask = crate::editor::GuiTask;
    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }
    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        crate::editor::helper_path()?;
        Some(Box::new(crate::editor::NamEditor::new(
            self.params.clone(),
            self.shared.clone(),
            async_executor,
        )))
    }
    fn task_executor(&mut self) -> TaskExecutor<Self> {
        crate::editor::task_executor(self.params.clone(), self.shared.clone())
    }
    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let frames = config.max_buffer_size as usize;
        if !config.sample_rate.is_finite()
            || config.sample_rate <= 0.0
            || frames == 0
            || frames > i32::MAX as usize
        {
            return false;
        }
        self.input.resize(frames, 0.0);
        self.left.resize(frames, 0.0);
        self.right.resize(frames, 0.0);
        self.ir_input.resize(frames, 0.0);
        self.ir_left.resize(frames, 0.0);
        self.ir_right.resize(frames, 0.0);
        self.domain = Domain::new(config.sample_rate);
        // initialize is non-realtime: prewarm synchronously so even an immediate
        // offline render starts with the selected chain, independent of polling.
        let assets = self.params.assets.lock().unwrap().clone();
        let mut errors = Vec::new();
        let mut load = |path: &Option<String>, extension| {
            path.as_ref().and_then(|path| {
                load_asset(path, extension, config.sample_rate as f64, frames)
                    .map_err(|error| errors.push(error))
                    .ok()
            })
        };
        self.model = load(&assets.model, "nam");
        self.ir = load(&assets.ir, "wav");
        *self.shared.status.lock().unwrap() = if errors.is_empty() {
            "Ready".into()
        } else {
            errors.join("; ")
        };
        self.shared
            .configure(config.sample_rate as f64, frames, assets);
        self.shared.processing_fault.store(false, Ordering::Relaxed);
        self.skip_initial_reset = true;
        true
    }
    fn reset(&mut self) {
        self.domain.reset();
        if self.skip_initial_reset {
            self.skip_initial_reset = false;
            return;
        }
        // NAM reset prewarms and may allocate. Keep rendering the current instance
        // until a fresh worker-prepared generation arrives. Native history therefore
        // persists briefly across subsequent transport resets (also offline).
        self.shared.generation.fetch_add(1, Ordering::Release);
    }
    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.adopt();
        let frames = buffer.samples();
        if frames > self.input.len() {
            for channel in buffer.as_slice().iter_mut() {
                channel.fill(0.0);
            }
            self.shared.meters(0.0, 0.0);
            return ProcessStatus::Normal;
        }
        let controls = self.params.controls();
        let input_gain = db_gain(controls.input);
        let threshold = db_gain(controls.threshold);
        let output_gain = db_gain(controls.output);
        let gains = [controls.bass, controls.middle, controls.treble]
            .map(|value| db_gain((value - 5.0) * 2.4));
        let mut input_peak = 0.0_f32;
        for i in 0..frames {
            let x = buffer.as_slice()[0][i];
            input_peak = input_peak.max(x.abs());
            self.input[i] = self
                .domain
                .input(x, input_gain, threshold, controls.gate_enabled);
        }
        let model_ok = self
            .model
            .as_mut()
            .map(|model| {
                model
                    .process(
                        &self.input[..frames],
                        &mut self.left[..frames],
                        &mut self.right[..frames],
                    )
                    .is_ok()
            })
            .unwrap_or(false);
        if !model_ok {
            if self.model.is_some() {
                self.shared.processing_fault.store(true, Ordering::Relaxed);
                self.left[..frames].fill(0.0);
                self.right[..frames].fill(0.0);
            } else {
                self.left[..frames].copy_from_slice(&self.input[..frames]);
                self.right[..frames].copy_from_slice(&self.input[..frames]);
            }
        }
        for i in 0..frames {
            let eq = self
                .domain
                .eq([self.left[i], self.right[i]], gains, controls.eq_enabled);
            self.left[i] = eq[0];
            self.right[i] = eq[1];
            // Convolution accepts mono input and may produce a stereo cabinet.
            self.ir_input[i] = (eq[0] + eq[1]) * 0.5;
        }
        let ir_ok = self
            .ir
            .as_mut()
            .map(|ir| {
                ir.process(
                    &self.ir_input[..frames],
                    &mut self.ir_left[..frames],
                    &mut self.ir_right[..frames],
                )
                .is_ok()
            })
            .unwrap_or(false);
        let ir_fault = self.ir.is_some() && !ir_ok;
        if ir_fault {
            self.shared.processing_fault.store(true, Ordering::Relaxed);
        }
        let stereo = buffer.channels() > 1;
        let mut output_peak = 0.0_f32;
        for i in 0..frames {
            let dry = [self.left[i], self.right[i]];
            let wet = if ir_ok {
                [self.ir_left[i], self.ir_right[i]]
            } else {
                dry
            };
            let out = self
                .domain
                .output(dry, wet, output_gain, controls.ir_enabled && ir_ok);
            let out = if ir_fault && controls.ir_enabled {
                [0.0; 2]
            } else {
                out
            };
            if stereo {
                buffer.as_slice()[0][i] = out[0];
                buffer.as_slice()[1][i] = out[1];
                output_peak = output_peak.max(out[0].abs()).max(out[1].abs());
            } else {
                let mono = (out[0] + out[1]) * 0.5;
                buffer.as_slice()[0][i] = mono;
                output_peak = output_peak.max(mono.abs());
            }
        }
        self.shared.meters(input_peak, output_peak);
        // Neural models can emit noise at zero input, and IRs can have tails.
        // Do not let the host suspend them just because the input became silent.
        if self.model.is_some() || self.ir.is_some() {
            ProcessStatus::KeepAlive
        } else {
            ProcessStatus::Normal
        }
    }
}
impl ClapPlugin for NamLinux {
    const CLAP_ID: &'static str = "org.nam-linux.amplifier";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("NAM amplifier with gate, three-band EQ and cabinet impulse response");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Mono,
        ClapFeature::Stereo,
    ];
}
impl Vst3Plugin for NamLinux {
    const VST3_CLASS_ID: [u8; 16] = *b"NamLinuxAmp00001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Context;
    impl InitContext<NamLinux> for Context {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute(&self, _: crate::editor::GuiTask) {
            panic!("unexpected task")
        }
        fn set_latency_samples(&self, _: u32) {}
        fn set_current_voice_capacity(&self, _: u32) {}
    }
    impl ProcessContext<NamLinux> for Context {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute_background(&self, _: crate::editor::GuiTask) {
            panic!("unexpected task")
        }
        fn execute_gui(&self, _: crate::editor::GuiTask) {
            panic!("unexpected task")
        }
        fn transport(&self) -> &Transport {
            panic!("unused transport")
        }
        fn next_event(&mut self) -> Option<PluginNoteEvent<NamLinux>> {
            None
        }
        fn send_event(&mut self, _: PluginNoteEvent<NamLinux>) {}
        fn set_latency_samples(&self, _: u32) {}
        fn set_current_voice_capacity(&self, _: u32) {}
    }
    #[test]
    fn offline_first_block_uses_model_and_reset_does_not_mute() {
        let path =
            std::env::temp_dir().join(format!("nam-amplifier-offline-{}.nam", std::process::id()));
        std::fs::write(&path, br#"{"version":"0.5.4","architecture":"Linear","config":{"receptive_field":1,"bias":false,"in_channels":1,"out_channels":1},"weights":[2],"sample_rate":48000}"#).unwrap();
        let mut plugin = NamLinux::default();
        plugin.params.assets.lock().unwrap().model = Some(path.to_string_lossy().into_owned());
        for channels in [1, 2] {
            let layout = &NamLinux::AUDIO_IO_LAYOUTS[if channels == 2 { 0 } else { 1 }];
            assert!(plugin.initialize(
                layout,
                &BufferConfig {
                    sample_rate: 48000.0,
                    min_buffer_size: None,
                    max_buffer_size: 64,
                    process_mode: ProcessMode::Offline
                },
                &mut Context
            ));
            plugin.reset();
            for _ in 0..4 {
                let mut left = [0.25; 64];
                let mut right = [0.0; 64];
                let mut buffer = Buffer::default();
                // SAFETY: matching lengths, live disjoint arrays for the whole process call.
                unsafe {
                    buffer.set_slices(64, |slices| {
                        slices.push(&mut left);
                        if channels == 2 {
                            slices.push(&mut right);
                        }
                    });
                }
                plugin.process(
                    &mut buffer,
                    &mut AuxiliaryBuffers {
                        inputs: &mut [],
                        outputs: &mut [],
                    },
                    &mut Context,
                );
                assert!(buffer
                    .as_slice()
                    .iter()
                    .all(|channel| channel.iter().all(|x| (*x - 0.5).abs() < 1e-6)));
                plugin.reset();
            }
        }
        drop(plugin);
        std::fs::remove_file(path).unwrap();
    }
}
