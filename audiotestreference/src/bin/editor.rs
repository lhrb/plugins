//! The helper alone owns GPUI and its event loop. No device or DSP lives here.
#[path = "../embedding.rs"]
mod embedding;
use gpui::{prelude::*, *};
use std::{
    io::{self, Read},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Default)]
struct Status {
    fault: AtomicBool,
    closed: AtomicBool,
}
struct ShellView {
    status: Arc<Status>,
    // Retain the X11 client while the child window is embedded.
    _embedding: x11rb::rust_connection::RustConnection,
}
impl Render for ShellView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x202328))
            .text_color(rgb(0xe6e8eb))
            .p_8()
            .flex()
            .flex_col()
            .gap_4()
            .child("AudioTestReference")
            .child("Slice 1 · CLAP / GPUI shell experiment · v0.1.0")
            .child("Stereo pass-through · reported latency: 0 samples")
            .child("Audio remains in the host. No mode controls in this experiment.")
            .child(if self.status.fault.load(Ordering::Relaxed) {
                "Fault: nonfinite input replaced with zero (latched until reset/activation)."
            } else {
                "Input safety: no nonfinite input observed since reset/activation."
            })
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--parent" {
        return Err("Use --parent <X11 window ID>".into());
    }
    let parent = args[1].parse::<u32>()?;
    let status = Arc::new(Status::default());
    let reader_status = status.clone();
    std::thread::spawn(move || {
        let mut byte = [0];
        while io::stdin().lock().read_exact(&mut byte).is_ok() {
            reader_status
                .fault
                .store(byte[0] == b'1', Ordering::Relaxed);
        }
        reader_status.closed.store(true, Ordering::Relaxed);
    });
    Application::new().run(move |cx: &mut App| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(660.), px(430.)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: WindowKind::PopUp,
                is_resizable: false,
                is_movable: false,
                is_minimizable: false,
                app_id: Some("audio-test-reference-editor".into()),
                ..Default::default()
            },
            move |window, cx| {
                let connection =
                    embedding::embed(parent, window.scale_factor()).unwrap_or_else(|error| {
                        eprintln!("AudioTestReference embedding: {error}");
                        std::process::exit(1);
                    });
                cx.new(|cx: &mut Context<ShellView>| {
                    let polled_status = status.clone();
                    cx.spawn(async move |view, cx| loop {
                        Timer::after(Duration::from_millis(250)).await;
                        if polled_status.closed.load(Ordering::Relaxed) {
                            let _ = cx.update(|cx| cx.quit());
                            break;
                        }
                        if view.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    })
                    .detach();
                    ShellView {
                        status,
                        _embedding: connection,
                    }
                })
            },
        );
        if let Err(error) = result {
            eprintln!("AudioTestReference window: {error}");
            std::process::exit(1);
        }
    });
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("AudioTestReference editor: {error}");
        std::process::exit(1);
    }
}
