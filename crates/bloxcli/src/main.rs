//! BloxCLI binary entrypoint. All parsing, dispatch and command
//! logic lives in the library (`bloxcli::cli` / `bloxcli::commands`);
//! this file only forwards process exit status.

fn main() -> std::process::ExitCode {
    bloxcli::cli::run_main()
}
