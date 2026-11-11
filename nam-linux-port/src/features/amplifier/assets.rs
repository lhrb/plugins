use crate::{
    features::model::NamModel,
    protocol::{Snapshot, UiEvent},
};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Assets {
    pub model: Option<String>,
    pub ir: Option<String>,
}

pub enum Change {
    Keep,
    Replace(Option<NamModel>),
}
pub struct Prepared {
    pub model: Change,
    pub ir: Change,
    pub generation: u64,
}

pub struct Shared {
    pub assets: Arc<Mutex<Assets>>,
    pub(crate) ready: ArrayQueue<Prepared>,
    pub(crate) retired: ArrayQueue<Prepared>,
    config: Mutex<(f64, usize, Assets, u64, u64)>,
    pub(crate) processing_fault: AtomicBool,
    pub(crate) generation: AtomicU64,
    ui_revision: AtomicU64,
    stop: AtomicBool,
    input_peak: AtomicU32,
    output_peak: AtomicU32,
    pub(crate) status: Mutex<String>,
}
impl Shared {
    pub fn new(assets: Arc<Mutex<Assets>>) -> Arc<Self> {
        Arc::new(Self {
            assets,
            ready: ArrayQueue::new(2),
            retired: ArrayQueue::new(2),
            config: Mutex::new((0.0, 0, Assets::default(), 0, 0)),
            processing_fault: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            ui_revision: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            input_peak: AtomicU32::new(0),
            output_peak: AtomicU32::new(0),
            status: Mutex::new("Ready".into()),
        })
    }
    pub fn configure(&self, rate: f64, frames: usize, initialized: Assets) {
        let mut config = self.config.lock().unwrap();
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *config = (
            rate,
            frames,
            initialized,
            generation,
            self.ui_revision.load(Ordering::Acquire),
        );
    }
    pub fn snapshot(&self, values: [f32; 9]) -> Snapshot {
        let assets = self.assets.lock().unwrap().clone();
        Snapshot {
            values,
            input_peak: f32::from_bits(self.input_peak.load(Ordering::Relaxed)),
            output_peak: f32::from_bits(self.output_peak.load(Ordering::Relaxed)),
            model: assets.model,
            ir: assets.ir,
            status: if self.processing_fault.load(Ordering::Relaxed) {
                "DSP processing failed; reload the asset or reset playback".into()
            } else {
                self.status.lock().unwrap().clone()
            },
        }
    }
    pub(crate) fn meters(&self, input: f32, output: f32) {
        self.input_peak.store(input.to_bits(), Ordering::Relaxed);
        self.output_peak.store(output.to_bits(), Ordering::Relaxed);
    }
    pub fn handle_asset_event(&self, event: &UiEvent) {
        let mut assets = self.assets.lock().unwrap();
        match event {
            UiEvent::LoadModel { path } => assets.model = Some(path.clone()),
            UiEvent::LoadIr { path } => assets.ir = Some(path.clone()),
            UiEvent::ClearModel => assets.model = None,
            UiEvent::ClearIr => assets.ir = None,
            UiEvent::PreviousModel => assets.model = adjacent(&assets.model, "nam", false),
            UiEvent::NextModel => assets.model = adjacent(&assets.model, "nam", true),
            UiEvent::PreviousIr => assets.ir = adjacent(&assets.ir, "wav", false),
            UiEvent::NextIr => assets.ir = adjacent(&assets.ir, "wav", true),
            _ => return,
        }
        self.ui_revision.fetch_add(1, Ordering::Release);
    }
}
pub(crate) fn load_asset(
    path: &str,
    extension: &str,
    rate: f64,
    frames: usize,
) -> Result<NamModel, String> {
    if Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
    {
        NamModel::load(Path::new(path), rate, frames)
    } else {
        Err(format!("Expected a .{extension} file"))
    }
}

fn adjacent(current: &Option<String>, extension: &str, next: bool) -> Option<String> {
    let path = Path::new(current.as_ref()?);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(path.parent()?)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(extension))
        })
        .collect();
    paths.sort();
    let index = paths.iter().position(|p| p == path)?;
    let index = if next {
        (index + 1) % paths.len()
    } else {
        (index + paths.len() - 1) % paths.len()
    };
    Some(paths[index].to_string_lossy().into_owned())
}

/// The thread holds only a Weak pointer between polls; plugin drop always joins it.
pub struct Worker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn start(shared: Arc<Shared>) -> Self {
        let weak = Arc::downgrade(&shared);
        let thread = thread::spawn(move || {
            let mut previous: Option<(Assets, u64)> = None;
            let mut revision = 0;
            loop {
                let Some(shared) = weak.upgrade() else { break };
                if shared.stop.load(Ordering::Acquire) {
                    break;
                }
                while shared.retired.pop().is_some() {}
                let desired = shared.assets.lock().unwrap().clone();
                let generation = shared.generation.load(Ordering::Acquire);
                let current_revision = shared.ui_revision.load(Ordering::Acquire);
                let (rate, frames, initialized, initial_generation, initial_revision) =
                    shared.config.lock().unwrap().clone();
                // initialize() already loaded this exact baseline synchronously.
                if initial_generation == generation
                    && previous.as_ref().map(|p| p.1) != Some(generation)
                {
                    previous = Some((initialized, generation));
                    revision = initial_revision;
                }
                if rate > 0.0
                    && (previous.as_ref() != Some(&(desired.clone(), generation))
                        || revision != current_revision)
                {
                    let ui = previous.is_some()
                        && revision != current_revision
                        && previous.as_ref().unwrap().1 == generation;
                    let mut errors = Vec::new();
                    let mut load = |path: &Option<String>, extension: &str| {
                        let Some(path) = path else {
                            return Change::Replace(None);
                        };
                        let result = load_asset(path, extension, rate, frames);
                        match result {
                            Ok(model) => Change::Replace(Some(model)),
                            Err(error) => {
                                errors.push(error);
                                if ui {
                                    Change::Keep
                                } else {
                                    Change::Replace(None)
                                }
                            }
                        }
                    };
                    let prepared = Prepared {
                        model: load(&desired.model, "nam"),
                        ir: load(&desired.ir, "wav"),
                        generation,
                    };
                    let mut persisted = desired.clone();
                    if ui {
                        let old = &previous.as_ref().unwrap().0;
                        if matches!(prepared.model, Change::Keep) {
                            persisted.model = old.model.clone();
                        }
                        if matches!(prepared.ir, Change::Keep) {
                            persisted.ir = old.ir.clone();
                        }
                        let mut assets = shared.assets.lock().unwrap();
                        if *assets == desired {
                            *assets = persisted.clone();
                        }
                    }
                    // Waiting and dropping superseded work happen only on this thread.
                    let mut pending = prepared;
                    loop {
                        match shared.ready.push(pending) {
                            Ok(()) => break,
                            Err(value) => pending = value,
                        }
                        if shared.stop.load(Ordering::Acquire) {
                            return;
                        }
                        while shared.retired.pop().is_some() {}
                        thread::sleep(Duration::from_millis(5));
                    }
                    *shared.status.lock().unwrap() = if errors.is_empty() {
                        "Ready".into()
                    } else {
                        errors.join("; ")
                    };
                    previous = Some((persisted, generation));
                }
                revision = current_revision;
                drop(shared);
                thread::sleep(Duration::from_millis(25));
            }
        });
        Self {
            shared,
            thread: Some(thread),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait(shared: &Shared) -> Prepared {
        for _ in 0..100 {
            if let Some(update) = shared.ready.pop() {
                return update;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("worker did not publish");
    }
    #[test]
    fn explicit_same_path_selection_reloads() {
        let path = std::env::temp_dir().join(format!(
            "nam-worker-retry-{}-{}.nam",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, br#"{"version":"0.5.4","architecture":"Linear","config":{"receptive_field":1,"bias":false},"weights":[1],"sample_rate":48000}"#).unwrap();
        let assets = Assets {
            model: Some(path.to_str().unwrap().into()),
            ir: None,
        };
        let shared = Shared::new(Arc::new(Mutex::new(assets.clone())));
        let _worker = Worker::start(shared.clone());
        shared.configure(48000.0, 64, assets);
        thread::sleep(Duration::from_millis(60));
        assert!(shared.ready.is_empty());
        shared.handle_asset_event(&UiEvent::LoadModel {
            path: path.to_str().unwrap().into(),
        });
        assert!(matches!(wait(&shared).model, Change::Replace(Some(_))));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn worker_stops_when_prepared_queue_is_full() {
        let shared = Shared::new(Arc::new(Mutex::new(Assets::default())));
        let worker = Worker::start(shared.clone());
        shared.configure(48000.0, 64, Assets::default());
        for target in 1..=2 {
            shared.generation.fetch_add(1, Ordering::Release);
            for _ in 0..100 {
                if shared.ready.len() == target {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(shared.ready.len(), target);
        }
        shared.generation.fetch_add(1, Ordering::Release);
        thread::sleep(Duration::from_millis(50));
        drop(worker); // Must interrupt publication backpressure without an audio consumer.
    }
    #[test]
    fn restore_failure_clears_but_ui_failure_keeps() {
        let shared = Shared::new(Arc::new(Mutex::new(Assets::default())));
        let _worker = Worker::start(shared.clone());
        shared.configure(48000.0, 64, Assets::default());
        shared.generation.fetch_add(1, Ordering::Release);
        drop(wait(&shared));
        shared.handle_asset_event(&UiEvent::LoadModel {
            path: "/missing.nam".into(),
        });
        assert!(matches!(wait(&shared).model, Change::Keep));
        assert_eq!(shared.assets.lock().unwrap().model, None);
        shared.assets.lock().unwrap().model = Some("/restored-missing.nam".into());
        assert!(matches!(wait(&shared).model, Change::Replace(None)));
        shared.generation.fetch_add(1, Ordering::Release);
        assert!(matches!(wait(&shared).model, Change::Replace(None)));
    }
}
