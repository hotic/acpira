//! The Acpira host binary: the engine's command line (acpira_host::cli, where the subcommands are listed)

fn main() {
  let args: Vec<String> = std::env::args().skip(1).collect();
  std::process::exit(acpira_host::cli::run(args));
}
