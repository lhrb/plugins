//! Session reducer. Background events cannot rewrite snapshots or newer runs.
use crate::measurement::{Conditions, MeasureError, Measurement};
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Settings {
    pub identity: String,
    pub parameters: Vec<(u32, f64)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub conditions: Conditions,
    pub slots: [Settings; 2],
}
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Complete(Measurement),
    PluginFailure(String),
    Timeout,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Running,
    Success,
    Cancelled,
    InvalidConfiguration,
    PartialFailure,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunId(u64);
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub run: Option<RunId>,
    pub snapshot: Option<Snapshot>,
    pub status: Status,
    pub outcomes: [Option<Outcome>; 2],
    pub stale: [bool; 2],
    pub progress: [usize; 2],
}
pub struct RunState {
    conditions: Conditions,
    slots: [Settings; 2],
    serial: u64,
    view: View,
}
impl Default for RunState {
    fn default() -> Self {
        Self {
            conditions: Conditions::default(),
            slots: Default::default(),
            serial: 0,
            view: View {
                run: None,
                snapshot: None,
                status: Status::Idle,
                outcomes: [None, None],
                stale: [false; 2],
                progress: [0; 2],
            },
        }
    }
}
impl RunState {
    pub fn set_slot(&mut self, slot: usize, settings: Settings) {
        if self.slots[slot] != settings {
            self.slots[slot] = settings;
            if self.view.snapshot.is_some() {
                self.view.stale[slot] = true;
            }
        }
    }
    pub fn set_conditions(&mut self, conditions: Conditions) {
        if self.conditions != conditions {
            self.conditions = conditions;
            if self.view.snapshot.is_some() {
                self.view.stale = [true; 2];
            }
        }
    }
    pub fn start(&mut self) -> Result<RunId, MeasureError> {
        if let Err(e) = self.conditions.validate() {
            self.view.status = Status::InvalidConfiguration;
            return Err(e);
        }
        self.serial += 1;
        let run = RunId(self.serial);
        self.view = View {
            run: Some(run),
            snapshot: Some(Snapshot {
                conditions: self.conditions.clone(),
                slots: self.slots.clone(),
            }),
            status: Status::Running,
            outcomes: [None, None],
            stale: [false; 2],
            progress: [0; 2],
        };
        Ok(run)
    }
    pub fn cancel(&mut self) {
        if self.view.status == Status::Running {
            self.view.status = Status::Cancelled;
        }
    }
    fn accepts(&self, run: RunId, slot: usize) -> bool {
        slot < 2
            && self.view.run == Some(run)
            && self.view.status == Status::Running
            && self.view.outcomes[slot].is_none()
    }
    pub fn progress(&mut self, run: RunId, slot: usize, completed: usize) {
        if self.accepts(run, slot) {
            let count = self
                .view
                .snapshot
                .as_ref()
                .unwrap()
                .conditions
                .frequencies_hz
                .len();
            self.view.progress[slot] = self.view.progress[slot].max(completed.min(count));
        }
    }
    pub fn finish(&mut self, run: RunId, slot: usize, outcome: Outcome) {
        if !self.accepts(run, slot) {
            return;
        }
        self.view.outcomes[slot] = Some(outcome);
        if self.view.outcomes.iter().all(Option::is_some) {
            self.view.status = if self
                .view
                .outcomes
                .iter()
                .all(|o| matches!(o, Some(Outcome::Complete(_))))
            {
                Status::Success
            } else {
                Status::PartialFailure
            };
        }
    }
    pub fn view(&self) -> View {
        self.view.clone()
    }
}
