//! Preheat orchestration.
//!
//! Pulls candidates from the pinned-set reader (Q082) and the bundled
//! landmarks manifest, dedupes by tile-id, applies the configured rate
//! limit, and either logs a dry-run summary or submits each as a
//! `Priority::HotPreheat` `BakeJob` to the Q081 producer.

use crate::{LandmarkManifest, PinnedReader, DEFAULT_OSM_SNAPSHOT, DEFAULT_STYLE_VERSION};
use bake_queue::{BakeJob, Priority, Producer};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Which source the preheat run should consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum Tier {
    /// Read only the pinned sorted set (`wb:cache.hot.tiles`).
    PinnedOnly,
    /// Read only the bundled landmarks manifest.
    Landmarks,
    /// Read both, dedupe by tile-id.
    Both,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::PinnedOnly => "pinned-only",
            Tier::Landmarks => "landmarks",
            Tier::Both => "both",
        }
    }
}

/// Summary of a preheat run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreheatOutcome {
    pub candidates: usize,
    pub deduped: usize,
    pub enqueued: usize,
    pub skipped_idempotent: usize,
    pub dry_run: bool,
}

/// Top-level runner.
pub struct PreheatRunner {
    pub tier: Tier,
    pub style_version: u32,
    pub osm_snapshot: String,
    pub rate_limit_per_sec: u32,
    pub dry_run: bool,
}

impl PreheatRunner {
    pub fn new(tier: Tier) -> Self {
        Self {
            tier,
            style_version: DEFAULT_STYLE_VERSION,
            osm_snapshot: DEFAULT_OSM_SNAPSHOT.to_string(),
            rate_limit_per_sec: crate::DEFAULT_RATE_LIMIT,
            dry_run: false,
        }
    }

    pub fn with_dry_run(mut self, dry: bool) -> Self {
        self.dry_run = dry;
        self
    }

    pub fn with_rate_limit(mut self, n: u32) -> Self {
        self.rate_limit_per_sec = n.max(1);
        self
    }

    pub fn with_style(mut self, v: u32) -> Self {
        self.style_version = v;
        self
    }

    pub fn with_osm_snapshot(mut self, s: impl Into<String>) -> Self {
        self.osm_snapshot = s.into();
        self
    }

    /// Build the deduplicated candidate list — pure logic, no IO except
    /// the pinned reader. Returns tile-ids in stable sorted order so
    /// retries (and tests) see deterministic enqueue order.
    pub async fn build_candidates(
        &self,
        pinned: &dyn PinnedReader,
        manifest: &LandmarkManifest,
    ) -> anyhow::Result<Vec<String>> {
        let mut set: BTreeSet<String> = BTreeSet::new();
        match self.tier {
            Tier::PinnedOnly => {
                for t in pinned.read_pinned().await? {
                    set.insert(t);
                }
            }
            Tier::Landmarks => {
                for l in &manifest.landmarks {
                    set.insert(l.tile_id());
                }
            }
            Tier::Both => {
                for t in pinned.read_pinned().await? {
                    set.insert(t);
                }
                for l in &manifest.landmarks {
                    set.insert(l.tile_id());
                }
            }
        }
        Ok(set.into_iter().collect())
    }

    /// Execute the full preheat: load sources, build the candidate list,
    /// and (unless `dry_run`) enqueue each to the Q081 producer.
    pub async fn run(
        &self,
        pinned: Arc<dyn PinnedReader>,
        manifest: &LandmarkManifest,
        producer: &Producer,
    ) -> anyhow::Result<PreheatOutcome> {
        let candidates_pinned_count = match self.tier {
            Tier::PinnedOnly | Tier::Both => pinned.read_pinned().await.map(|v| v.len()).unwrap_or(0),
            Tier::Landmarks => 0,
        };
        let candidates_landmark_count = match self.tier {
            Tier::Landmarks | Tier::Both => manifest.len(),
            Tier::PinnedOnly => 0,
        };
        let total_in = candidates_pinned_count + candidates_landmark_count;

        let deduped = self.build_candidates(pinned.as_ref(), manifest).await?;
        info!(
            tier = self.tier.as_str(),
            pinned = candidates_pinned_count,
            landmarks = candidates_landmark_count,
            deduped = deduped.len(),
            dry_run = self.dry_run,
            rate_limit_per_sec = self.rate_limit_per_sec,
            "preheat candidate list ready"
        );

        let mut outcome = PreheatOutcome {
            candidates: total_in,
            deduped: deduped.len(),
            enqueued: 0,
            skipped_idempotent: 0,
            dry_run: self.dry_run,
        };
        if self.dry_run {
            for tid in &deduped {
                debug!(tile = %tid, "dry-run enqueue intent");
            }
            outcome.enqueued = deduped.len();
            return Ok(outcome);
        }

        // Real enqueue. Pace at rate_limit_per_sec — every N submits we
        // sleep just enough to stay under the budget. Q081 producer
        // already short-circuits on idempotency-key hit.
        let per_tick = self.rate_limit_per_sec.max(1) as usize;
        let mut tick = 0usize;
        for tid in deduped {
            match parse_tile_id(&tid) {
                Ok((z, x, y)) => {
                    let job = BakeJob::new(z, x, y, self.style_version, &self.osm_snapshot)
                        .with_priority(Priority::HotPreheat);
                    match producer.submit(&job).await {
                        Ok(Some(_)) => outcome.enqueued += 1,
                        Ok(None) => outcome.skipped_idempotent += 1,
                        Err(e) => {
                            warn!(tile = %tid, error = %e, "enqueue failed");
                        }
                    }
                }
                Err(e) => warn!(tile = %tid, error = %e, "bad tile-id; skipping"),
            }

            tick += 1;
            if tick >= per_tick {
                tokio::time::sleep(Duration::from_secs(1)).await;
                tick = 0;
            }
        }
        Ok(outcome)
    }
}

fn parse_tile_id(s: &str) -> anyhow::Result<(u32, u32, u32)> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 3 {
        anyhow::bail!("expected z/x/y, got {s:?}");
    }
    Ok((parts[0].parse()?, parts[1].parse()?, parts[2].parse()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pinned::InMemoryPinnedReader;
    use crate::{landmarks::Landmark, landmarks::SeedSource};
    use bake_queue::{mock::MockQueue, Queue, STREAM_HOT};

    fn manifest(items: &[(&str, f64, f64)]) -> LandmarkManifest {
        let landmarks = items
            .iter()
            .map(|(n, lat, lon)| Landmark {
                name: (*n).to_string(),
                lat: *lat,
                lon: *lon,
                seed_source: SeedSource::Curated,
                z: None,
                x: None,
                y: None,
            })
            .collect();
        LandmarkManifest { landmarks }
    }

    #[tokio::test]
    async fn dry_run_counts_dedup_intents_without_enqueueing() {
        // Canonical projection at z=15 for (48.8584, 2.2945) lands on
        // (16592, 11272); pin the same tile so dedupe must collapse.
        let pinned = Arc::new(InMemoryPinnedReader::new(vec!["15/16592/11272".into()]));
        let m = manifest(&[("Eiffel Tower", 48.8584, 2.2945)]); // same tile
        let r = PreheatRunner::new(Tier::Both).with_dry_run(true);
        let mock = Arc::new(MockQueue::new());
        let q: Arc<dyn Queue> = mock.clone();
        let p = Producer::new(q);
        let out = r.run(pinned, &m, &p).await.unwrap();
        assert_eq!(out.deduped, 1, "duplicates collapse across sources");
        assert_eq!(out.enqueued, 1);
        assert!(out.dry_run);
        // Mock stream stays empty because dry-run.
        assert_eq!(mock.backlog_len(STREAM_HOT).await, 0);
    }

    #[tokio::test]
    async fn real_run_enqueues_to_hot_stream() {
        let pinned = Arc::new(InMemoryPinnedReader::new(vec!["15/1/1".into(), "15/2/2".into()]));
        let m = manifest(&[("Iconic Test Site", 0.0, 0.0)]);
        let r = PreheatRunner::new(Tier::Both)
            .with_dry_run(false)
            .with_rate_limit(1000);

        let mock = Arc::new(MockQueue::new());
        let q: Arc<dyn Queue> = mock.clone();
        let p = Producer::new(q);
        let out = r.run(pinned, &m, &p).await.unwrap();
        assert_eq!(out.enqueued, 3);
        assert_eq!(mock.backlog_len(STREAM_HOT).await, 3);
        assert_eq!(mock.backlog_len(bake_queue::STREAM_COLD).await, 0);
    }

    #[tokio::test]
    async fn tier_filter_pinned_only_ignores_manifest() {
        let pinned = Arc::new(InMemoryPinnedReader::new(vec!["15/9/9".into()]));
        let m = manifest(&[("Should Be Ignored", 1.0, 2.0)]);
        let r = PreheatRunner::new(Tier::PinnedOnly).with_dry_run(true);
        let q: Arc<dyn Queue> = Arc::new(MockQueue::new());
        let p = Producer::new(q);
        let out = r.run(pinned, &m, &p).await.unwrap();
        assert_eq!(out.deduped, 1);
    }

    #[tokio::test]
    async fn tier_filter_landmarks_only_ignores_pinned() {
        let pinned = Arc::new(InMemoryPinnedReader::new(vec!["15/9/9".into()]));
        let m = manifest(&[("Eiffel Tower", 48.8584, 2.2945)]);
        let r = PreheatRunner::new(Tier::Landmarks).with_dry_run(true);
        let q: Arc<dyn Queue> = Arc::new(MockQueue::new());
        let p = Producer::new(q);
        let out = r.run(pinned, &m, &p).await.unwrap();
        assert_eq!(out.deduped, 1);
        assert_eq!(out.candidates, 1);
    }

    #[test]
    fn parse_tile_id_round_trip() {
        assert_eq!(parse_tile_id("15/1/2").unwrap(), (15, 1, 2));
        assert!(parse_tile_id("15/1").is_err());
        assert!(parse_tile_id("xx/1/2").is_err());
    }
}

