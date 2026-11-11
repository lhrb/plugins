//! NIH-plug adapter. Hosts expose the controls through their generic editor.

use super::domain::{Parameters, TapeProcessor};
use nih_plug::prelude::*;
use std::sync::Arc;

pub struct Oxide {
    params: Arc<OxideParams>,
    processor: TapeProcessor,
}

#[derive(Params)]
pub struct OxideParams {
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "wow"]
    pub wow: FloatParam,
    #[id = "speed"]
    pub speed: BoolParam,
}

impl Default for OxideParams {
    fn default() -> Self {
        Self {
            drive: FloatParam::new(
                "Drive",
                6.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 24.0,
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),
            output: FloatParam::new(
                "Output",
                -6.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 6.0,
                },
            )
            .with_unit(" dB")
            .with_step_size(0.1),
            mix: FloatParam::new("Mix", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            tone: FloatParam::new(
                "Tone",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            ),
            wow: FloatParam::new(
                "Wow / Flutter",
                0.15,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            speed: BoolParam::new("Speed", false).with_value_to_string(Arc::new(|fast| {
                if fast { "30 ips" } else { "15 ips" }.into()
            })),
        }
    }
}

impl OxideParams {
    fn values(&self) -> Parameters {
        Parameters {
            drive_db: self.drive.value(),
            output_db: self.output.value(),
            mix: self.mix.value(),
            tone: self.tone.value(),
            wow: self.wow.value(),
            speed_30_ips: self.speed.value(),
        }
    }
}

impl Default for Oxide {
    fn default() -> Self {
        Self {
            params: Arc::new(OxideParams::default()),
            processor: TapeProcessor::default(),
        }
    }
}

impl Plugin for Oxide {
    const NAME: &'static str = "OXIDE";
    const VENDOR: &'static str = "Tape Works";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let rate = buffer_config.sample_rate;
        if !rate.is_finite() || !(8_000.0..=384_000.0).contains(&rate) {
            return false;
        }
        self.processor.set_parameters(self.params.values());
        self.processor.set_sample_rate(rate);
        true
    }

    fn reset(&mut self) {
        self.processor.set_parameters(self.params.values());
        self.processor.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // NIH-plug splits blocks at automation events. Domain smoothing is the
        // single smoothing layer, including smooth transitions between speeds.
        self.processor.set_parameters(self.params.values());
        let channels = buffer.as_slice();
        let (left, right) = channels.split_at_mut(1);
        for (left, right) in left[0].iter_mut().zip(right[0].iter_mut()) {
            let frame = self.processor.process_frame([*left, *right]);
            *left = frame[0];
            *right = frame[1];
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for Oxide {
    const CLAP_ID: &'static str = "com.tapeworks.oxide";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Stereo tape saturation and transport coloration");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Distortion,
    ];
}

impl Vst3Plugin for Oxide {
    const VST3_CLASS_ID: [u8; 16] = *b"TapeWorksOXIDE01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
}
