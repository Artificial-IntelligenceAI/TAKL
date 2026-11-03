//! `takl program.takl` runs a program. `--interpret` keeps it in the interpreter, as on an iPad or in a browser;
//! `--quick` and `--speed` send every procedure to Cranelift at that strength from the start.

fn main() {
    let mut mode = takl::Mode::Tiered;
    let mut path = None;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--interpret" => mode = takl::Mode::Interpret,
            "--quick" => mode = takl::Mode::Quick,
            "--speed" => mode = takl::Mode::Speed,
            "--eager" => mode = takl::Mode::TieredEager,
            other => path = Some(other.to_string()),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: takl [--interpret | --quick | --speed] program.takl");
        std::process::exit(2);
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("TAKL couldn't read {}", path);
        std::process::exit(2);
    };
    // Deep recursion runs on a thread with room for 10,000 calls of compiled and interpreted code.
    let worker = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || takl::run_with(&path, &text, mode, true).exit_code);
    let code = worker.ok().and_then(|handle| handle.join().ok()).unwrap_or(1);
    std::process::exit(code as i32);
}
