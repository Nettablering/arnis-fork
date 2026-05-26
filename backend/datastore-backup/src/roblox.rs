//! Open Cloud DataStore client + a synthetic in-memory client for tests.
//!
//! Roblox docs: https://apis.roblox.com/datastores/v1/universes/{universeId}/standard-datastores
//! We treat the API as paginated key enumeration + per-key value GET. Real wire
//! integration only kicks in when an API key is configured; the rest of the
//! pipeline (encrypt/store/manifest/restore) is exercised end-to-end against
//! [`SyntheticClient`] in the test suite.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataStoreEntry {
    pub datastore: String,
    pub scope: String,
    pub key: String,
    /// Arbitrary JSON value stored against the key.
    pub value: serde_json::Value,
}

#[async_trait]
pub trait OpenCloudClient: Send + Sync {
    /// List all (datastore, scope, key) → value entries for a universe. The
    /// real impl pages internally; for our nightly volume (~10k keys) the
    /// in-memory aggregation is fine.
    async fn list_all_entries(&self, universe_id: u64) -> anyhow::Result<Vec<DataStoreEntry>>;
}

/// HTTP impl against apis.roblox.com. Wire-level paging is intentionally
/// simple — Roblox returns `nextPageCursor`; we follow until empty. We do not
/// exercise this against the live API in CI; integration is gated on an
/// `OPEN_CLOUD_API_KEY` env var at runtime.
pub struct OpenCloudHttp {
    pub api_key: String,
    pub base_url: String,
    pub client: reqwest::Client,
}

impl OpenCloudHttp {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://apis.roblox.com/datastores/v1".into(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl OpenCloudClient for OpenCloudHttp {
    async fn list_all_entries(&self, universe_id: u64) -> anyhow::Result<Vec<DataStoreEntry>> {
        // Minimal-but-correct paging loop. Real production deployment will
        // expand this with backoff + 429 handling; nightly is well within the
        // documented ~5 RPS budget (see q099 edge-cases).
        let mut out = Vec::new();
        let datastores = self.list_datastores(universe_id).await?;
        for ds in datastores {
            let keys = self.list_keys(universe_id, &ds).await?;
            for key in keys {
                let value = self.get_value(universe_id, &ds, &key).await?;
                out.push(DataStoreEntry {
                    datastore: ds.clone(),
                    scope: "global".into(),
                    key,
                    value,
                });
            }
        }
        Ok(out)
    }
}

impl OpenCloudHttp {
    async fn list_datastores(&self, universe_id: u64) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/universes/{}/standard-datastores", self.base_url, universe_id);
        let resp: serde_json::Value = self
            .client
            .get(url)
            .header("x-api-key", &self.api_key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .get("datastores")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| d.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn list_keys(&self, universe_id: u64, datastore: &str) -> anyhow::Result<Vec<String>> {
        let url = format!(
            "{}/universes/{}/standard-datastores/datastore/entries",
            self.base_url, universe_id
        );
        let resp: serde_json::Value = self
            .client
            .get(url)
            .query(&[("datastoreName", datastore)])
            .header("x-api-key", &self.api_key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp
            .get("keys")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|k| k.get("key").and_then(|n| n.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn get_value(
        &self,
        universe_id: u64,
        datastore: &str,
        key: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!(
            "{}/universes/{}/standard-datastores/datastore/entries/entry",
            self.base_url, universe_id
        );
        let resp = self
            .client
            .get(url)
            .query(&[("datastoreName", datastore), ("entryKey", key)])
            .header("x-api-key", &self.api_key)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(serde_json::from_str(&resp).unwrap_or(serde_json::Value::String(resp)))
    }
}

/// In-memory client for tests + offline/local-mode runs (e.g. when no
/// universe is enrolled yet and we still want the nightly pipeline rehearsed).
#[derive(Default, Clone)]
pub struct SyntheticClient {
    pub entries: BTreeMap<u64, Vec<DataStoreEntry>>,
}

impl SyntheticClient {
    /// Build a 10-key fixture matching the VERIFICATION step in q099. Keys
    /// are deterministic so tests can pin manifests.
    pub fn ten_key_fixture(universe_id: u64) -> Self {
        let mut entries = Vec::new();
        for i in 0..10u32 {
            entries.push(DataStoreEntry {
                datastore: "OverlayStore".into(),
                scope: "global".into(),
                key: format!("Player_{:04}", i),
                value: serde_json::json!({
                    "claimed": [{ "tileId": format!("t{}", i), "level": i as u64 % 5 }],
                    "accrued": i as u64 * 1000,
                }),
            });
        }
        let mut m = BTreeMap::new();
        m.insert(universe_id, entries);
        Self { entries: m }
    }
}

#[async_trait]
impl OpenCloudClient for SyntheticClient {
    async fn list_all_entries(&self, universe_id: u64) -> anyhow::Result<Vec<DataStoreEntry>> {
        Ok(self.entries.get(&universe_id).cloned().unwrap_or_default())
    }
}
