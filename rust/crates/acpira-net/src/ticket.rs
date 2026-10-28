//! The pairing ticket a computer shows as a QR code or an `acpira://pair/…` link: who to dial, where it was last seen,
//! and a one-time secret. The dial itself authenticates the computer (its EndpointId is in here), so the secret only has
//! to prove the device saw this screen. A small binary layout keeps the QR code dense enough to scan from across a desk.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh::{EndpointAddr, EndpointId, RelayUrl, TransportAddr};

pub const LINK_PREFIX: &str = "acpira://pair/";
const VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairTicket {
  pub id: EndpointId,
  /// The computer's name as its settings page shows it, so the device can say what it is pairing with
  pub name: String,
  pub secret: [u8; 32],
  /// Its home relay when the ticket was made
  pub relay: Option<RelayUrl>,
  /// LAN, Tailscale and public addresses it had then
  pub addrs: Vec<SocketAddr>,
}

impl PairTicket {
  pub fn endpoint_addr(&self) -> EndpointAddr {
    let addrs = self.addrs.iter().copied().map(TransportAddr::Ip).chain(self.relay.clone().map(TransportAddr::Relay));
    EndpointAddr::from_parts(self.id, addrs)
  }

  pub fn encode(&self) -> String {
    let mut b = vec![VERSION];
    b.extend_from_slice(self.id.as_bytes());
    b.extend_from_slice(&self.secret);
    put_str(&mut b, clip(&self.name, 64));
    put_str(&mut b, self.relay.as_ref().map(|u| u.as_str()).unwrap_or(""));
    let addrs: Vec<&SocketAddr> = self.addrs.iter().take(12).collect();
    b.push(addrs.len() as u8);
    for a in addrs {
      match a.ip() {
        IpAddr::V4(v4) => {
          b.push(4);
          b.extend_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => {
          b.push(6);
          b.extend_from_slice(&v6.octets());
        }
      }
      b.extend_from_slice(&a.port().to_be_bytes());
    }
    URL_SAFE_NO_PAD.encode(b)
  }

  pub fn link(&self) -> String {
    format!("{LINK_PREFIX}{}", self.encode())
  }

  /// A ticket, its link, or either pasted with whitespace around it
  pub fn decode(s: &str) -> Result<Self> {
    let s = s.trim();
    let s = s.strip_prefix(LINK_PREFIX).unwrap_or(s);
    let b = URL_SAFE_NO_PAD.decode(s).map_err(|_| anyhow!("not a pairing code"))?;
    let mut r = Reader { b: &b, pos: 0 };
    if r.u8()? != VERSION {
      bail!("this pairing code comes from a different Acpira version");
    }
    let id = EndpointId::from_bytes(&r.array::<32>()?).map_err(|e| anyhow!("bad endpoint id: {e}"))?;
    let secret = r.array::<32>()?;
    let name = r.str()?;
    let relay = match r.str()? {
      s if s.is_empty() => None,
      s => Some(s.parse::<RelayUrl>().map_err(|e| anyhow!("bad relay url: {e}"))?),
    };
    let mut addrs = Vec::new();
    for _ in 0..r.u8()? {
      let ip = match r.u8()? {
        4 => IpAddr::V4(Ipv4Addr::from(r.array::<4>()?)),
        6 => IpAddr::V6(Ipv6Addr::from(r.array::<16>()?)),
        f => bail!("bad address family {f}"),
      };
      addrs.push(SocketAddr::new(ip, u16::from_be_bytes(r.array::<2>()?)));
    }
    Ok(Self { id, name, secret, relay, addrs })
  }
}

fn clip(s: &str, max: usize) -> &str {
  let mut end = s.len().min(max);
  while !s.is_char_boundary(end) {
    end -= 1;
  }
  s.get(..end).unwrap_or_default()
}

fn put_str(b: &mut Vec<u8>, s: &str) {
  let s = clip(s, 255);
  b.push(s.len() as u8);
  b.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
  b: &'a [u8],
  pos: usize,
}

impl Reader<'_> {
  fn take(&mut self, n: usize) -> Result<&[u8]> {
    let s = self.b.get(self.pos..self.pos + n).ok_or_else(|| anyhow!("pairing code cut short"))?;
    self.pos += n;
    Ok(s)
  }
  fn u8(&mut self) -> Result<u8> {
    Ok(self.take(1)?[0])
  }
  fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
    let mut a = [0u8; N];
    a.copy_from_slice(self.take(N)?);
    Ok(a)
  }
  fn str(&mut self) -> Result<String> {
    let n = self.u8()? as usize;
    Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_ticket_survives_its_link_and_stays_short() {
    let t = PairTicket {
      id: iroh::SecretKey::generate().public(),
      name: "Mac mini 的电脑".into(),
      secret: [9; 32],
      relay: Some("https://usw1-1.relay.n0.iroh.link./".parse().unwrap()),
      addrs: vec!["192.168.71.85:51433".parse().unwrap(), "100.83.164.70:51433".parse().unwrap(), "[240e:b8f:965b:f500::1]:50483".parse().unwrap()],
    };
    let link = t.link();
    assert!(link.len() < 260, "{} chars", link.len());
    assert_eq!(PairTicket::decode(&link).unwrap(), t);
    assert_eq!(PairTicket::decode(&format!("  {}\n", t.encode())).unwrap(), t);
    assert!(PairTicket::decode("acpira://pair/abc").is_err());
    assert!(PairTicket::decode(link.get(..link.len() - 6).unwrap()).is_err());
  }
}
