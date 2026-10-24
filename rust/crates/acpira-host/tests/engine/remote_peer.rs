//! Remote run's wire (`remote/peer.rs`): this engine reaches the host's engine through the helper's `engine` op as a
//! peer, the host starts that engine when none runs and keeps it past the connection, and a second link finds the same
//! one. The host is this machine behind the fake ssh, with its home under the test's directory

#![cfg(unix)]

use std::time::Duration;

use acpira_host::remote::peer::{PeerHello, PeerLink};
use acpira_shared::remote::{ConnectionOp, RunMode};
use serde_json::{Value, json};

use crate::remote_connections::Lab;

const ME: PeerHello<'static> = PeerHello { version: "test", capabilities: &["toast"], host_language: "en" };

async fn next_of(link: &mut PeerLink, pred: impl Fn(&Value) -> bool) -> Value {
  tokio::time::timeout(Duration::from_secs(30), async {
    loop {
      let v = link.recv().await.expect("the host's engine closed the connection");
      if pred(&v) {
        return v;
      }
    }
  })
  .await
  .expect("no such envelope from the host's engine")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_reaches_the_hosts_engine_which_outlives_the_connection() {
  let lab = Lab::new();
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into(), mode: RunMode::Remote }).await.unwrap();
  let client = connections.ensure("gen1", Duration::from_secs(30)).await.unwrap();

  let mut link = PeerLink::connect(&client, None, ME).await.expect("connected to the host's engine");
  let pid = link.hello_ok["sidecar"]["pid"].as_u64().expect("the engine's pid") as i32;
  assert_ne!(pid as u32, std::process::id());
  link.send(&json!({ "type": "attachView", "viewId": "P", "host": "sidebar", "initial": { "mostRecent": true } }));
  link.send(&json!({ "type": "webviewMessage", "viewId": "P", "message": { "type": "ready" } }));
  let init = next_of(&mut link, |v| v["type"] == "hostMessage" && v["message"]["type"] == "init").await;
  assert_eq!(init["viewId"], "P");
  drop(link);

  // The engine stays for the next connection, which finds it rather than starting another
  tokio::time::sleep(Duration::from_millis(300)).await;
  // SAFETY: kill with signal 0 only checks that the process exists
  assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "the host's engine outlives the connection");
  let again = PeerLink::connect(&client, None, ME).await.expect("connected again");
  assert_eq!(again.hello_ok["sidecar"]["pid"].as_u64(), Some(pid as u64));
  drop(again);

  // SAFETY: SIGTERM to the engine this test started; it runs its normal teardown
  unsafe { libc::kill(pid, libc::SIGTERM) };
  let gone = async {
    // SAFETY: as above, existence only
    while unsafe { libc::kill(pid, 0) } == 0 {
      tokio::time::sleep(Duration::from_millis(50)).await;
    }
  };
  tokio::time::timeout(Duration::from_secs(10), gone).await.expect("the host's engine ended on SIGTERM");
  connections.dispose();
}
