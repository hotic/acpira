// A GUI program on Windows: a login task starts it without a console window (cli.rs attaches the other commands to the
// console they were typed in)
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
  std::process::exit(acpira_gateway::cli::main(Vec::new(), std::env::args().skip(1).collect()));
}
