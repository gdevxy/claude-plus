fn main() {
    if let Err(error) = claude_plus::cli::run() {
        eprintln!("claude-plus: {error:#}");
        std::process::exit(1);
    }
}
