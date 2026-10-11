use acpira_agent::mock::{MockModel, Reply};

static SERIAL: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[test]
fn an_explicit_engine_proxy_keeps_loopback_direct() {
  let _guard = SERIAL.lock();
  let proxy = MockModel::start();
  let local = MockModel::start();
  proxy.push(Reply::Status(502, "proxy must not receive local traffic".into()));
  local.push(Reply::Status(200, "local endpoint".into()));
  acpira_host::net_proxy::set_setting(proxy.base_url().trim_end_matches("/v1"));
  let client: ureq::Agent = acpira_host::net_proxy::ureq_config(ureq::Agent::config_builder()
    .http_status_as_error(false).timeout_global(Some(std::time::Duration::from_secs(2)))).build().into();
  let result = client.get(&local.base_url()).call();
  assert!(proxy.requests().is_empty(), "loopback request went to the proxy: result={result:?}, requests={:?}", proxy.requests());
}

#[test]
fn an_explicit_socks_setting_reaches_the_engine_http_client() {
  let _guard = SERIAL.lock();
  acpira_host::net_proxy::set_setting("socks5h://127.0.0.1:19081");
  let config = acpira_host::net_proxy::ureq_config(ureq::Agent::config_builder().proxy(None)).build();
  assert!(config.proxy().is_some(), "the accepted SOCKS setting was silently discarded");
}
