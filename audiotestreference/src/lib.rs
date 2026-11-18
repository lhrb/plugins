pub mod domain;
mod editor;

use nih_plug::prelude::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::domain::{Processor, SampleResult};

#[derive(Default, Params)]
struct ShellParams {}

#[derive(Default)]
pub struct AudioTestReference {
    params: Arc<ShellParams>,
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
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(editor::ShellEditor::new(self.fault.clone())))
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
