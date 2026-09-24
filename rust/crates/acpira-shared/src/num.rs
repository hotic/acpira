//! JS numbers on the wire: records written by the TS host may carry an integer field as `12.0` or `1.7e12`, and a
//! peer may send a float where an integer is expected. These deserializers accept any finite number and round it,
//! so one odd field never makes a whole record unreadable

use serde::{Deserialize, Deserializer};
use serde_json::Value;

fn as_i64(v: &Value) -> Option<i64> {
  match v {
    Value::Number(n) => n
      .as_i64()
      .or_else(|| n.as_u64().map(|u| u.min(i64::MAX as u64) as i64))
      .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.round() as i64)),
    _ => None,
  }
}

pub fn lenient_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
  let v = Value::deserialize(d)?;
  as_i64(&v).ok_or_else(|| serde::de::Error::custom("expected a number"))
}

pub fn lenient_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
  Ok(lenient_i64(d)?.max(0) as u64)
}

pub fn lenient_opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
  Ok(as_i64(&Value::deserialize(d)?))
}

pub fn lenient_opt_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
  Ok(as_i64(&Value::deserialize(d)?).map(|n| n.max(0) as u64))
}

/// A JS number rendered the way `String(n)` / `JSON.stringify` would: integral values without a fraction
pub fn js_number(n: f64) -> Value {
  if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
    Value::from(n as i64)
  } else {
    serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null)
  }
}

/// Serialize an f64 like JSON.stringify does (no `.0` on integral values)
pub fn ser_js_f64<S: serde::Serializer>(n: &f64, s: S) -> Result<S::Ok, S::Error> {
  if n.is_finite() && n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 { s.serialize_i64(*n as i64) } else { s.serialize_f64(*n) }
}

pub fn ser_js_opt_f64<S: serde::Serializer>(n: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
  match n {
    Some(n) => ser_js_f64(n, s),
    None => s.serialize_none(),
  }
}

/// A JS number: deserializes from any JSON number, serializes like JSON.stringify (integral values without `.0`)
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
pub struct Num(pub f64);

impl Num {
  pub fn get(self) -> f64 {
    self.0
  }
}

impl From<f64> for Num {
  fn from(v: f64) -> Self {
    Num(v)
  }
}

impl From<i64> for Num {
  fn from(v: i64) -> Self {
    Num(v as f64)
  }
}

impl serde::Serialize for Num {
  fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
    ser_js_f64(&self.0, s)
  }
}

impl<'de> Deserialize<'de> for Num {
  fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
    f64::deserialize(d).map(Num)
  }
}
