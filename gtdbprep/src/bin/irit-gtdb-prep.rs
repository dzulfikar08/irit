//! `irit-gtdb-prep ARGS` is `gtdbprep prep ARGS`.
fn main() -> std::process::ExitCode {
    gtdbprep::cli::main(Some("prep"))
}
