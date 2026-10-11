use std::process::ExitCode;

fn main() -> ExitCode {
    claude_plus::cli::run().unwrap_or_else(|error| {
        eprintln!("claude-plus: {error:#}");
        ExitCode::FAILURE
    })
}
