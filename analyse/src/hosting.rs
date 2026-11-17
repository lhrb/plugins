//! Trusted CLAP process adapter. The parent never loads plugin code.
use crate::{clap::Plugin, measurement::Processor, BLOCK};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, BufRead, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Parameter {
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub value: f64,
    pub formatted: String,
    pub stepped: bool,
    pub readonly: bool,
}
#[derive(Debug, Serialize, Deserialize)]
enum Request {
    Info,
    Set(u32, f64),
    Reset,
    Block(Vec<[f64; 2]>),
    Quit,
    Crash,
    Hang,
}
#[derive(Debug, Serialize, Deserialize)]
enum Reply {
    Info(Vec<Parameter>, Option<u32>),
    Reset(bool),
    Block(Vec<[f64; 2]>),
    Bye,
    Error(String),
}

/// Called before starting GPUI, on the worker's original OS main thread.
/// This is an internal transport mode, not a user-facing CLI.
pub fn worker_main(path: &str, id: &str) -> Result<(), String> {
    // SAFETY: all plugin calls stay on this thread and Plugin owns ABI lifetimes.
    let mut plugin = unsafe { Plugin::load(path, id) }.map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        unsafe { plugin.service() }.map_err(|e| e.to_string())?;
        let line = match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(line) => line.map_err(|e| e.to_string())?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => return Ok(()),
        };
        let request: Request = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let reply = unsafe {
            match request {
                Request::Info => Reply::Info(
                    plugin.parameters().map_err(|e| e.to_string())?,
                    plugin.latency(),
                ),
                Request::Set(id, value) => {
                    plugin.set(id, value).map_err(|e| e.to_string())?;
                    Reply::Info(
                        plugin.parameters().map_err(|e| e.to_string())?,
                        plugin.latency(),
                    )
                }
                Request::Reset => Reply::Reset(plugin.reset()),
                Request::Block(input) => {
                    if input.len() != BLOCK {
                        return Err("invalid block length".into());
                    }
                    let planar =
                        std::array::from_fn(|ch| std::array::from_fn(|i| input[i][ch] as f32));
                    let output = plugin.block(planar, None).map_err(|e| e.to_string())?;
                    Reply::Block(
                        (0..BLOCK)
                            .map(|i| [output[0][i] as f64, output[1][i] as f64])
                            .collect(),
                    )
                }
                Request::Quit => {
                    drop(plugin);
                    send(&Reply::Bye)?;
                    return Ok(());
                }
                Request::Crash => std::process::abort(),
                Request::Hang => loop {
                    thread::sleep(Duration::from_secs(60));
                },
            }
        };
        send(&reply)?;
    }
}
fn send(reply: &Reply) -> Result<(), String> {
    // Prefix keeps plugin stdout diagnostics out of the protocol parser.
    let mut out = io::stdout().lock();
    writeln!(
        out,
        "@inspector {}",
        serde_json::to_string(reply).map_err(|e| e.to_string())?
    )
    .map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}
#[derive(Clone, Debug, PartialEq)]
pub enum WorkerError {
    Timeout,
    Failure(String),
}
impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
pub struct Worker {
    child: Child,
    stdin: ChildStdin,
    replies: mpsc::Receiver<String>,
    timeout: Duration,
    pub parameters: Vec<Parameter>,
    pub latency: Option<u32>,
    pub cancel: Arc<AtomicBool>,
    reset_error: Option<String>,
}
impl Worker {
    pub fn spawn(
        executable: &Path,
        path: &Path,
        id: &str,
        timeout: Duration,
    ) -> Result<Self, WorkerError> {
        let mut child = Command::new(executable)
            .arg("--worker")
            .arg(path)
            .arg(id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| WorkerError::Failure(e.to_string()))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, replies) = mpsc::channel();
        thread::spawn(move || {
            for line in io::BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if let Some(payload) = line.strip_prefix("@inspector ") {
                            if tx.send(payload.to_owned()).is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let mut worker = Self {
            child,
            stdin,
            replies,
            timeout,
            parameters: vec![],
            latency: None,
            cancel: Arc::new(AtomicBool::new(false)),
            reset_error: None,
        };
        worker.info()?;
        Ok(worker)
    }
    fn request(&mut self, request: Request) -> Result<Reply, WorkerError> {
        let result = self.exchange(request);
        if result.is_err() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        result
    }
    fn exchange(&mut self, request: Request) -> Result<Reply, WorkerError> {
        let fail = |e: io::Error| WorkerError::Failure(e.to_string());
        writeln!(self.stdin, "{}", serde_json::to_string(&request).unwrap()).map_err(fail)?;
        self.stdin.flush().map_err(fail)?;
        let deadline = Instant::now() + self.timeout;
        // One small audio block per request bounds pipe writes; a stuck plugin
        // cannot consume the next request and fill the input pipe indefinitely.
        loop {
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(10));
            match self.replies.recv_timeout(wait) {
                Ok(line) => {
                    return serde_json::from_str(&line)
                        .map_err(|e| WorkerError::Failure(e.to_string()))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(WorkerError::Failure("worker exited/closed stdout".into()))
                }
                Err(_) if Instant::now() >= deadline => return Err(WorkerError::Timeout),
                Err(_) => {}
            }
        }
    }
    fn apply_info(&mut self, reply: Reply) -> Result<(), WorkerError> {
        match reply {
            Reply::Info(parameters, latency) => {
                self.parameters = parameters;
                self.latency = latency;
                Ok(())
            }
            other => Err(WorkerError::Failure(format!("unexpected reply {other:?}"))),
        }
    }
    pub fn info(&mut self) -> Result<(), WorkerError> {
        let reply = self.request(Request::Info)?;
        self.apply_info(reply)
    }
    pub fn set(&mut self, id: u32, value: f64) -> Result<(), WorkerError> {
        let reply = self.request(Request::Set(id, value))?;
        self.apply_info(reply)
    }
    pub fn shutdown(&mut self) -> Result<(), WorkerError> {
        match self.request(Request::Quit)? {
            Reply::Bye => Ok(()),
            other => Err(WorkerError::Failure(format!("unexpected reply {other:?}"))),
        }
    }
    /// Fault injection for adapter verification, never exposed as a UI control.
    pub fn verify_fault(&mut self, timeout: bool) -> Result<(), WorkerError> {
        self.request(if timeout {
            Request::Hang
        } else {
            Request::Crash
        })
        .map(|_| ())
    }
}
impl Processor for Worker {
    fn reset(&mut self) -> bool {
        self.reset_error = None;
        match self.request(Request::Reset) {
            Ok(Reply::Reset(supported)) => supported,
            other => {
                self.reset_error = Some(format!("reset failed: {other:?}"));
                false
            }
        }
    }
    fn process(&mut self, input: &[[f64; 2]], output: &mut [[f64; 2]]) -> Result<(), String> {
        if let Some(error) = self.reset_error.take() {
            return Err(error);
        }
        if self.cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        match self
            .request(Request::Block(input.to_vec()))
            .map_err(|e| e.to_string())?
        {
            Reply::Block(frames) if frames.len() == output.len() => {
                output.copy_from_slice(&frames);
                Ok(())
            }
            reply => Err(format!("unexpected reply {reply:?}")),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
