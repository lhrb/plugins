//! Version-local editor IPC. No UI or host types cross the process boundary.
use serde::{Deserialize, Serialize};

pub const WIDTH: u32 = 660;
pub const HEIGHT: u32 = 430;
pub const PARAM_IDS: [&str; 9] = [
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEvent {
    Begin { id: String },
    Set { id: String, normalized: f32 },
    End { id: String },
    LoadModel { path: String },
    LoadIr { path: String },
    ClearModel,
    ClearIr,
    PreviousModel,
    NextModel,
    PreviousIr,
    NextIr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub values: [f32; 9],
    pub input_peak: f32,
    pub output_peak: f32,
    pub model: Option<String>,
    pub ir: Option<String>,
    pub status: String,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            values: [0.5, 0.2, 0.5, 0.5, 0.5, 0.5, 1., 1., 1.],
            input_peak: 0.,
            output_peak: 0.,
            model: None,
            ir: None,
            status: "Select a .nam model and an optional cabinet IR".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum HostMessage {
    Snapshot(Snapshot),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ipc_round_trip_preserves_paths_and_controls() {
        let event = UiEvent::LoadModel {
            path: "/tmp/amp ü.nam".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(matches!(serde_json::from_str::<UiEvent>(&json).unwrap(),
            UiEvent::LoadModel { path } if path == "/tmp/amp ü.nam"));
        let message = HostMessage::Snapshot(Snapshot::default());
        let encoded = serde_json::to_string(&message).unwrap();
        let HostMessage::Snapshot(decoded) = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.values, Snapshot::default().values);
    }
}
