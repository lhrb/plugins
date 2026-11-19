pub mod domain;
mod editor;

use nih_plug::prelude::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::domain::{Processor, SampleResult};

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
enum HostMode {
    #[id = "identity"]
    Identity,
    #[id = "gain"]
    Gain,
}

#[derive(Params)]
struct PluginParams {
    #[id = "mode"]
    mode: EnumParam<HostMode>,
    #[id = "gain"]
    gain: FloatParam,
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
        }
    }
}

enum ParameterEdit {
    Mode(HostMode),
    Gain(f32),
}

pub struct SetParameterTask {
    edit: ParameterEdit,
    context: std::sync::Weak<dyn GuiContext>,
}

#[derive(Default)]
pub struct AudioTestReference {
    params: Arc<PluginParams>,
    fault: Arc<AtomicBool>,
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
        self.fault.store(false, Ordering::Relaxed);
        true
    }

    fn reset(&mut self) {
        self.fault.store(false, Ordering::Relaxed);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _: &mut AuxiliaryBuffers,
        _: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut fault = false;
        let requested_mode = match self.params.mode.value() {
            HostMode::Identity => Ok(domain::Mode::Identity),
            HostMode::Gain => {
                let gain = f64::from(self.params.gain.value());
                domain::Gain::new(gain).map(|gain| domain::Mode::Gain { gain })
            }
        };

        if let Ok(mode) = requested_mode {
            self.processor.set_mode(mode);
        }

        for channel in buffer.as_slice() {
            for sample in channel.iter_mut() {
                let SampleResult { output, faulted } = self.processor.process_sample(*sample);
                *sample = output;
                fault |= faulted;
            }
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
