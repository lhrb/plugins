//! Linux host adapters around the unchanged official NAM inference library.
pub mod editor;
pub mod protocol;
pub mod features {
    pub mod amplifier;
    pub mod model;
}

nih_plug::nih_export_clap!(features::amplifier::plugin::NamLinux);
nih_plug::nih_export_vst3!(features::amplifier::plugin::NamLinux);
