//! The model catalogue as the agent sees it: the snapshot built into the binary, or the engine's daily refresh at
//! `<root>/catalog/models.json` when that is newer and in the current format (the same choice the engine makes). It
//! fills a configured model's missing limits and prices each request

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

use acpira_shared::model_catalog::{Catalog, CatalogFile, CatalogModel, Cost, model_key};
use acpira_shared::providers::ProviderModel;

static SNAPSHOT: LazyLock<Arc<Catalog>> = LazyLock::new(|| Arc::new(Catalog::new(CatalogFile::snapshot())));

type Stamp = Option<(SystemTime, u64)>;

pub struct CatalogCache {
  path: PathBuf,
  state: parking_lot::Mutex<Option<(Stamp, Arc<Catalog>)>>,
}

impl CatalogCache {
  pub fn new(home: &std::path::Path) -> Self {
    CatalogCache { path: home.join("catalog").join("models.json"), state: parking_lot::Mutex::new(None) }
  }

  /// The current catalogue, re-read when the engine rewrote its cache
  pub fn get(&self) -> Arc<Catalog> {
    let stamp = std::fs::metadata(&self.path).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
    let mut st = self.state.lock();
    if let Some((s, c)) = &*st
      && *s == stamp
    {
      return c.clone();
    }
    let cached = stamp
      .and_then(|_| std::fs::read_to_string(&self.path).ok())
      .and_then(|t| serde_json::from_str::<CatalogFile>(&t).ok())
      .filter(|f| f.supersedes(&SNAPSHOT.file));
    let catalog = cached.map(|f| Arc::new(Catalog::new(f))).unwrap_or_else(|| SNAPSHOT.clone());
    *st = Some((stamp, catalog.clone()));
    catalog
  }
}

/// The catalogue entry for a configured model id (gateway prefixes, case, separators and date suffixes ignored)
pub fn lookup<'a>(catalog: &'a Catalog, id: &str) -> Option<&'a CatalogModel> {
  catalog.get(&model_key(id))
}

/// Fill the limits the configuration leaves open from the catalogue, marked as estimated; returns the list prices
pub fn complete(model: &mut ProviderModel, catalog: &Catalog) -> Option<Cost> {
  let entry = lookup(catalog, &model.id)?;
  for (field, slot, value) in [("context", &mut model.context, entry.context), ("output", &mut model.output, entry.output)] {
    if slot.is_none()
      && let Some(v) = value
    {
      *slot = Some(v);
      if !model.estimated.iter().any(|e| e == field) {
        model.estimated.push(field.to_owned());
      }
    }
  }
  entry.cost
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_hand_entered_id_gets_the_catalogues_limits_and_prices() {
    let catalog = Catalog::new(CatalogFile::snapshot());
    let mut model: ProviderModel = serde_json::from_value(serde_json::json!({ "id": "deepseek/DeepSeek-V4-Flash", "context": 64000 })).unwrap();
    let cost = complete(&mut model, &catalog).expect("listed with prices");
    assert_eq!(model.context, Some(64000), "a configured value stays");
    assert!(model.output.is_some_and(|o| o > 0));
    assert_eq!(model.estimated, ["output"]);
    assert!(cost.input > 0.0 && cost.output > 0.0);
    let mut unknown: ProviderModel = serde_json::from_value(serde_json::json!({ "id": "my-local-model" })).unwrap();
    assert!(complete(&mut unknown, &catalog).is_none());
    assert_eq!((unknown.context, unknown.output), (None, None));
  }

  #[test]
  fn a_newer_cache_in_the_current_format_wins_over_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let cache = CatalogCache::new(dir.path());
    assert!(Arc::ptr_eq(&cache.get(), &SNAPSHOT));
    std::fs::create_dir_all(dir.path().join("catalog")).unwrap();
    let mut file = CatalogFile::snapshot();
    file.fetched_at = "9999-01-01T00:00:00Z".into();
    file.models.truncate(1);
    file.format = 1;
    std::fs::write(dir.path().join("catalog/models.json"), file.to_json()).unwrap();
    assert!(Arc::ptr_eq(&cache.get(), &SNAPSHOT), "an older format is not used");
    file.format = acpira_shared::model_catalog::FORMAT;
    file.models[0].name = "From the cache".into();
    std::fs::write(dir.path().join("catalog/models.json"), file.to_json()).unwrap();
    assert_eq!(cache.get().file.models[0].name, "From the cache");
  }
}
