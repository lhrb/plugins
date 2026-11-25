pub mod domain;
mod editor;

use nih_plug::prelude::*;
use std::{
    ops::RangeInclusive,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
};

use crate::domain::{
    low_pass_control, ConfigError, Processor, SampleResult, CUTOFF_RANGE, DEFAULT_CUTOFF,
    DEFAULT_Q, MAX_DELAY_SAMPLES, Q_RANGE,
};

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
enum HostMode {
    #[id = "identity"]
    Identity,
    #[id = "gain"]
    Gain,
    #[id = "delay"]
    Delay,
    #[id = "low_pass"]
    #[name = "Low-pass"]
    LowPass,
}

#[derive(Params)]
struct PluginParams {
    #[id = "mode"]
    mode: EnumParam<HostMode>,
    #[id = "gain"]
    gain: FloatParam,
    #[id = "delay_samples"]
    delay_samples: IntParam,
    #[id = "cutoff_fraction"]
    cutoff: FloatParam,
    #[id = "low_pass_q"]
    q: FloatParam,
}

impl Default for PluginParams {
    fn default() -> Self {
        Self {
            mode: EnumParam::new("Mode", HostMode::Identity),
            gain: FloatParam::new(
                "Gain",
                1.0,
                FloatRange::Linear {
                    min: -16.0,
                    max: 16.0,
                },
            ),
            delay_samples: IntParam::new(
                "Delay Time",
                0,
                IntRange::Linear {
                    min: 0,
                    max: MAX_DELAY_SAMPLES as i32,
                },
            )
            .with_unit(" samples"),
            cutoff: low_pass_param("Cutoff (fraction)", DEFAULT_CUTOFF, CUTOFF_RANGE),
            q: low_pass_param("Q", DEFAULT_Q, Q_RANGE),
        }
    }
}

fn low_pass_param(name: &str, default: f64, range: RangeInclusive<f64>) -> FloatParam {
    FloatParam::new(
        name,
        default as f32,
        FloatRange::Linear {
            min: *range.start() as f32,
            max: *range.end() as f32,
        },
    )
    .with_string_to_value(Arc::new(move |text| {
        let value = text.parse::<f64>().ok()?;
        range.contains(&value).then_some(value as f32)
    }))
}

enum ParameterEdit {
    Mode(HostMode),
    Gain(f32),
    DelaySamples(i32),
    Cutoff(f32),
    Q(f32),
}

pub struct SetParameterTask {
    edit: ParameterEdit,
    context: std::sync::Weak<dyn GuiContext>,
}

#[derive(Default)]
struct LowPassReadback {
    sample_rate: AtomicU32,
    // Two plain f32 values packed together; zero means no applied Low-pass setting.
    applied: AtomicU64,
}

#[derive(Default)]
pub struct AudioTestReference {
    params: Arc<PluginParams>,
    fault: Arc<AtomicBool>,
    low_pass_readback: Arc<LowPassReadback>,
    processor: Processor,
}

impl Plugin for AudioTestReference {
    const NAME: &'static str = "AudioTestReference (Shell Experiment)";
    const VENDOR: &'static str = "AudioTestReference";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;
    type SysExMessage = ();
    type BackgroundTask = SetParameterTask;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(editor::EditorBridge::new(
            Arc::clone(&self.params),
            Arc::clone(&self.fault),
            Arc::clone(&self.low_pass_readback),
            executor,
        )))
    }

    fn initialize(
        &mut self,
        layout: &AudioIOLayout,
        config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        if layout.main_input_channels != NonZeroU32::new(2)
            || layout.main_output_channels != NonZeroU32::new(2)
            || !domain::supports_sample_rate(config.sample_rate)
        {
            return false;
        }
        context.set_latency_samples(0);
        self.reset();
        self.low_pass_readback.applied.store(0, Ordering::Relaxed);
        self.low_pass_readback
            .sample_rate
            .store(config.sample_rate.to_bits(), Ordering::Release);
        true
    }

    fn deactivate(&mut self) {
        self.low_pass_readback
            .sample_rate
            .store(0, Ordering::Release);
    }

    fn reset(&mut self) {
        self.processor.reset();
        self.fault.store(false, Ordering::Relaxed);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _: &mut AuxiliaryBuffers,
        _: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let n = self.params.delay_samples.value();
        if (0..=MAX_DELAY_SAMPLES as i32).contains(&n) {
            let n = n as usize;
            let mut applied = 0;
            let requested_mode = match self.params.mode.value() {
                HostMode::Identity => Ok(domain::Mode::Identity),
                HostMode::Gain => {
                    let gain = f64::from(self.params.gain.value());
                    domain::Gain::new(gain).map(|gain| domain::Mode::Gain { gain })
                }
                HostMode::Delay => Ok(domain::Mode::Delay { n }),
                HostMode::LowPass => {
                    let cutoff_plain = self.params.cutoff.value();
                    let q_plain = self.params.q.value();
                    applied =
                        u64::from(cutoff_plain.to_bits()) | (u64::from(q_plain.to_bits()) << 32);
                    low_pass_control(cutoff_plain, CUTOFF_RANGE)
                        .zip(low_pass_control(q_plain, Q_RANGE))
                        .map(|(cutoff, q)| domain::Mode::LowPass { cutoff, q })
                        .ok_or(ConfigError::InvalidCoefficient)
                }
            };

            if let Ok(mode) = requested_mode {
                let delay_selected = matches!(mode, domain::Mode::Delay { .. });
                if self.processor.set_mode(mode).is_ok() {
                    // Track length even when Delay is inactive.
                    if !delay_selected {
                        let _ = self.processor.set_delay_samples(n);
                    }
                    self.low_pass_readback
                        .applied
                        .store(applied, Ordering::Relaxed);
                }
            }
        }

        let [left, right] = buffer.as_slice() else {
            return ProcessStatus::Error("expect stereo buffer");
        };

        let mut fault = false;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let SampleResult { output, faulted } = self.processor.process_frame([*l, *r]);
            *l = output[0];
            *r = output[1];
            fault |= faulted;
        }
        if fault {
            self.fault.store(true, Ordering::Relaxed);
        }
        ProcessStatus::Normal
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let params = Arc::clone(&self.params);

        Box::new(move |task: SetParameterTask| {
            let Some(context) = task.context.upgrade() else {
                return; // The editor has closed.
            };

            let setter = ParamSetter::new(context.as_ref());
            match task.edit {
                ParameterEdit::Mode(mode) => {
                    setter.begin_set_parameter(&params.mode);
                    setter.set_parameter(&params.mode, mode);
                    setter.end_set_parameter(&params.mode);
                }
                ParameterEdit::Gain(gain) => {
                    if domain::Gain::new(f64::from(gain)).is_err() {
                        return;
                    }
                    setter.begin_set_parameter(&params.gain);
                    setter.set_parameter(&params.gain, gain);
                    setter.end_set_parameter(&params.gain);
                }
                ParameterEdit::DelaySamples(n) => {
                    if !(0..=MAX_DELAY_SAMPLES as i32).contains(&n) {
                        return;
                    }
                    setter.begin_set_parameter(&params.delay_samples);
                    setter.set_parameter(&params.delay_samples, n);
                    setter.end_set_parameter(&params.delay_samples);
                }
                ParameterEdit::Cutoff(value) => {
                    if low_pass_control(value, CUTOFF_RANGE).is_none() {
                        return;
                    }
                    setter.begin_set_parameter(&params.cutoff);
                    setter.set_parameter(&params.cutoff, value);
                    setter.end_set_parameter(&params.cutoff);
                }
                ParameterEdit::Q(value) => {
                    if low_pass_control(value, Q_RANGE).is_none() {
                        return;
                    }
                    setter.begin_set_parameter(&params.q);
                    setter.set_parameter(&params.q, value);
                    setter.end_set_parameter(&params.q);
                }
            }
        })
    }
}

impl ClapPlugin for AudioTestReference {
    const CLAP_ID: &'static str = "org.audio-test-reference.shell-experiment";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Stereo pass-through / embedded GPUI shell experiment");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Stereo];
}

nih_export_clap!(AudioTestReference);
