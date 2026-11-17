//! Trusted local binaries only. Each worker owns one CLAP instance for its lifetime.
use clap_probe::clap::Plugin;
use clap_probe::*;
use std::{
    fs,
    io::{self, BufRead, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}

unsafe fn worker(args: &[String]) -> Result<()> {
    let mut plugin = Plugin::load(&args[0], &args[1])?;
    println!("OK ready");
    io::stdout().flush()?;
    let (tx, commands) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        // Keep main-thread callbacks serviced even while this loaded slot is idle.
        plugin.service()?;
        let line = match commands.recv_timeout(Duration::from_millis(10)) {
            Ok(line) => line?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let words: Vec<_> = line.split_whitespace().collect();
        match words.as_slice() {
            ["snapshot"] => println!("OK snapshot {}", plugin.snapshot()?),
            ["set", id, value] => {
                plugin.set(id.parse()?, value.parse()?)?;
                println!("OK set {}", plugin.snapshot()?);
            }
            ["render", reset, path] => {
                plugin.render(*reset == "reset", Path::new(path))?;
                println!("OK render {} time={}", plugin.snapshot()?, plugin.time);
            }
            ["crash"] => std::process::abort(),
            ["hang"] => loop {
                thread::sleep(Duration::from_secs(60));
            },
            ["quit"] => {
                // Acknowledge only after teardown, so the parent's timeout covers it too.
                drop(plugin);
                println!("OK quit");
                io::stdout().flush()?;
                return Ok(());
            }
            _ => return Err("invalid worker command".into()),
        }
        plugin.service()?;
        io::stdout().flush()?;
    }
    Ok(())
}

struct Worker {
    name: String,
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    timeout: Duration,
}
impl Worker {
    fn spawn(name: &str, path: &str, id: &str, timeout: Duration) -> Result<Self> {
        let mut child = Command::new(std::env::current_exe()?)
            .args(["worker", path, id])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in io::BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let mut worker = Self {
            name: name.into(),
            child,
            stdin,
            lines,
            timeout,
        };
        worker.response()?;
        Ok(worker)
    }
    fn response(&mut self) -> Result<String> {
        let deadline = std::time::Instant::now() + self.timeout;
        loop {
            match self
                .lines
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            {
                Ok(line) => {
                    println!("{}: {line}", self.name);
                    if line.starts_with("OK ") {
                        return Ok(line);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.child.kill()?;
                    let status = self.child.wait()?;
                    return Err(format!(
                        "{} configured timeout {:?}; killed {status}",
                        self.name, self.timeout
                    )
                    .into());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let status = match self.child.try_wait()? {
                        Some(status) => status,
                        None => {
                            self.child.kill()?;
                            self.child.wait()?
                        }
                    };
                    return Err(
                        format!("{} worker exited/closed stdout {status}", self.name).into(),
                    );
                }
            }
        }
    }
    fn command(&mut self, command: &str) -> Result<String> {
        writeln!(self.stdin, "{command}")?;
        self.stdin.flush()?;
        self.response()
    }
    fn render(&mut self, dir: &Path, label: &str, reset: bool) -> Result<Vec<f32>> {
        let path = dir.join(format!("{}-{label}.f32le", self.name));
        self.command(&format!(
            "render {} {}",
            if reset { "reset" } else { "continue" },
            path.display()
        ))?;
        read_samples(&path)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn read_samples(path: &Path) -> Result<Vec<f32>> {
    let bytes = fs::read(path)?;
    require(
        bytes.len() == RENDER_BLOCKS * BLOCK * 2 * 4,
        "unexpected render length",
    )?;
    let samples: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    require(samples.iter().all(|v| v.is_finite()), "nonfinite render")?;
    Ok(samples)
}
fn compare(label: &str, a: &[f32], b: &[f32]) {
    let (peak, rms) = difference(a, b);
    println!(
        "COMPARE {label}: peak={peak:.12e} rms={rms:.12e} tolerance={TOLERANCE} within={}",
        peak <= TOLERANCE
    );
}
fn run(args: &[String]) -> Result<()> {
    require(
        args.len() == 10,
        "run ABS_PATH_A ID_A PARAM_A VALUE_A ABS_PATH_B ID_B PARAM_B VALUE_B OUTPUT_DIR TIMEOUT_MS",
    )?;
    let dir = fs::canonicalize(&args[8])?;
    require(
        !dir.to_string_lossy().contains(char::is_whitespace),
        "output path cannot contain whitespace",
    )?;
    let mut input_bytes = Vec::new();
    for b in 0..PRE_BLOCKS + RENDER_BLOCKS {
        let block = input(b);
        for i in 0..BLOCK {
            for channel in &block {
                input_bytes.extend_from_slice(&channel[i].to_le_bytes());
            }
        }
    }
    fs::write(dir.join("input.f32le"), input_bytes)?;
    let timeout = Duration::from_millis(args[9].parse()?);
    require(!timeout.is_zero(), "timeout must be positive")?;
    let mut a = Worker::spawn("oxide", &args[0], &args[1], timeout)?;
    let mut b = Worker::spawn("chow", &args[4], &args[5], timeout)?;
    // Both selected plugins remain loaded throughout all renders and interventions.
    let a1 = a.render(&dir, "repeat1", true)?;
    let a2 = a.render(&dir, "repeat2", true)?;
    compare("oxide-reset-repeat", &a1, &a2);
    let b1 = b.render(&dir, "repeat1", true)?;
    let b2 = b.render(&dir, "repeat2", true)?;
    compare("chow-reset-repeat", &b1, &b2);
    let mut ac = Worker::spawn("oxide-control", &args[0], &args[1], timeout)?;
    let mut bc = Worker::spawn("chow-control", &args[4], &args[5], timeout)?;
    // Mirror all prior CHOW renders: reset alone did not reproduce its initial state.
    let bc1 = bc.render(&dir, "repeat1", true)?;
    let bc2 = bc.render(&dir, "repeat2", true)?;
    compare("chow-fresh-instance-first-trial", &b1, &bc1);
    compare("chow-matched-history-second-trial", &b2, &bc2);
    let before_b = b.command("snapshot")?;
    let bs = b.render(&dir, "history-start", true)?;
    let bcs = bc.render(&dir, "history-start", true)?;
    compare("chow-control-baseline", &bs, &bcs);
    a.command(&format!("set {} {}", args[2], args[3]))?;
    let changed_a = a.render(&dir, "edited", true)?;
    compare("oxide-parameter-effect", &a2, &changed_a);
    require(
        before_b == b.command("snapshot")?,
        "oxide edit changed chow settings",
    )?;
    let bh = b.render(&dir, "history-continue", false)?;
    let bch = bc.render(&dir, "history-continue", false)?;
    compare("chow-history-after-oxide-edit-reset", &bh, &bch);
    compare(
        "chow-continuation-versus-start-history-sensitivity",
        &bs,
        &bh,
    );
    // Apply the same new setting to A's control, then establish equal starting histories.
    ac.command(&format!("set {} {}", args[2], args[3]))?;
    let before_a = a.command("snapshot")?;
    let ast = a.render(&dir, "history-start", true)?;
    let acst = ac.render(&dir, "history-start", true)?;
    compare("oxide-control-baseline", &ast, &acst);
    b.command(&format!("set {} {}", args[6], args[7]))?;
    let changed_b = b.render(&dir, "edited", true)?;
    compare("chow-parameter-effect", &b2, &changed_b);
    require(
        before_a == a.command("snapshot")?,
        "chow edit changed oxide settings",
    )?;
    let ah = a.render(&dir, "history-continue", false)?;
    let ach = ac.render(&dir, "history-continue", false)?;
    compare("oxide-history-after-chow-edit-reset", &ah, &ach);
    compare(
        "oxide-continuation-versus-start-history-sensitivity",
        &ast,
        &ah,
    );
    // Retain an actual completed peer render across both controlled worker faults.
    let retained = dir.join("chow-edited.f32le");
    let retained_bytes = fs::read(&retained)?;
    let crash = a.command("crash").expect_err("crash should fail");
    println!("RECOVERY crash reported: {crash}");
    require(
        fs::read(&retained)? == retained_bytes,
        "peer result changed after crash",
    )?;
    b.command("snapshot")?;
    let mut replacement = Worker::spawn("oxide-replacement", &args[0], &args[1], timeout)?;
    let timed = replacement
        .command("hang")
        .expect_err("hang should time out");
    println!("RECOVERY timeout reported: {timed}");
    require(
        fs::read(&retained)? == retained_bytes,
        "peer result changed after timeout",
    )?;
    b.command("snapshot")?;
    println!(
        "RECOVERY retained {} bytes; surviving peer responsive",
        retained_bytes.len()
    );
    for worker in [&mut b, &mut ac, &mut bc] {
        worker.command("quit")?;
        require(worker.child.wait()?.success(), "worker shutdown failed")?;
    }
    println!("CHECKPOINT slice 1 finished; numerical variation is evidence, not automatically a host defect");
    Ok(())
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("worker") if args.len() == 3 => unsafe { worker(&args[1..]) },
        Some("inspect") if args.len() == 4 => (|| { let mut w = Worker::spawn("inspect", &args[1], &args[2], Duration::from_millis(args[3].parse()?))?; w.command("snapshot")?; w.command("quit")?; require(w.child.wait()?.success(), "shutdown failed") })(),
        Some("run") => run(&args[1..]),
        _ => Err("usage: clap-probe inspect ABS_PATH ID TIMEOUT_MS | run PATH_A ID_A PARAM_A VALUE_A PATH_B ID_B PARAM_B VALUE_B EXISTING_OUTPUT_DIR TIMEOUT_MS".into()),
    };
    if let Err(error) = result {
        eprintln!("UNSUPPORTED/FAILED: {error}");
        std::process::exit(1);
    }
}
