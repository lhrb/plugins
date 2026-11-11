//! OXIDE's portable DSP and optional CLAP/VST3 adapter.

pub mod features {
    pub mod saturation;
}

#[cfg(feature = "plugin")]
use features::saturation::plugin::Oxide;

#[cfg(feature = "plugin")]
nih_plug::nih_export_clap!(Oxide);
#[cfg(feature = "plugin")]
nih_plug::nih_export_vst3!(Oxide);
