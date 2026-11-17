mod inspector_ui;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--worker") {
        let result = if args.len() == 4 {
            clap_probe::hosting::worker_main(&args[2], &args[3])
        } else {
            Err("invalid internal worker arguments".into())
        };
        if let Err(error) = result {
            eprintln!("Worker: {error}");
            std::process::exit(1);
        }
        return;
    }
    inspector_ui::run();
}
