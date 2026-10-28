#[cfg(unix)]
#[tokio::main]
async fn main() {
  acpira_gateway::cli::main().await;
}

#[cfg(not(unix))]
fn main() {
  eprintln!("acpira-gateway runs on macOS and Linux only for now");
  std::process::exit(2);
}
