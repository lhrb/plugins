#[path = "../editor_ui/mod.rs"]
mod editor_ui;

fn main() {
    if let Err(error) = editor_ui::run() {
        eprintln!("NAM editor: {error}");
        std::process::exit(1);
    }
}
