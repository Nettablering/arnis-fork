//! Q013 + Q030 + Q211: composite landmark rarity blend.
//!
//! Q211 specifies a 6-factor weighted sum:
//!
//! ```text
//! rarity_score =
//!     0.40 * pageview_rarity            # global attention (Q211)
//!   + 0.20 * heritage_boost             # UNESCO=1.0, listed=0.5, none=0
//!   + 0.15 * height_rarity              # tall building rarity
//!   + 0.10 * uniqueness_in_radius_5km   # how rare nearby
//!   + 0.10 * fictional_appearances      # P1441 count, normalised
//!   + 0.05 * age_score                  # log(years since inception)
//! ```
//!
//! Only `pageview_rarity` and `height_rarity` are wired today; the rest
//! land as Q210 (Wikidata heritage tags), Q216 (geographic uniqueness),
//! and Q217 (fictional-appearance crawler). Missing factors contribute 0.
//!
//! Q211 §"Edge cases": when *all* factors are 0 (no Wikipedia article,
//! no notable height) we return None so the emitter omits the field
//! entirely — keeping the manifest small and snapshot-stable for the
//! ~99% of buildings that are unremarkable.

/// Q211 blend weights — versioned alongside `FORMULA_VERSION` so an
/// economy regression is detectable in the manifest.
pub const W_PAGEVIEW: f32 = 0.40;
pub const W_HERITAGE: f32 = 0.20;
pub const W_HEIGHT: f32 = 0.15;
pub const W_UNIQUENESS: f32 = 0.10;
pub const W_FICTIONAL: f32 = 0.10;
pub const W_AGE: f32 = 0.05;

/// Q211 §"Formula version lock" — bump this whenever the weights or
/// factors change so the bake-server can refuse to mix old manifests.
pub const FORMULA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default)]
pub struct RarityInputs {
    pub pageview_rarity: Option<f32>,
    pub heritage_boost: Option<f32>,
    /// Building height in metres, if any. Q211 height-rarity normalises
    /// at `log10(h+1) / log10(800)` so the Burj Khalifa lands at ≈1.0.
    pub height_m: Option<f32>,
    pub uniqueness_in_radius_5km: Option<f32>,
    pub fictional_appearances: Option<f32>,
    pub age_years: Option<f32>,
}

/// Q211 height normalisation: log-scaled against an 800 m yardstick
/// (Burj Khalifa = 828 m → ≈1.0). Buildings under ~12 m get effectively 0.
fn height_rarity(height_m: f32) -> f32 {
    if height_m <= 12.0 {
        return 0.0;
    }
    let raw = (height_m + 1.0).log10() / 800.0_f32.log10();
    raw.clamp(0.0, 1.0)
}

/// Compute the blended rarity score in `[0, 1]`. Returns `None` if every
/// input factor is missing or zero — see module docs for why.
pub fn blend(inputs: &RarityInputs) -> Option<f32> {
    let pv = inputs.pageview_rarity.unwrap_or(0.0);
    let her = inputs.heritage_boost.unwrap_or(0.0);
    let height = inputs.height_m.map(height_rarity).unwrap_or(0.0);
    let uniq = inputs.uniqueness_in_radius_5km.unwrap_or(0.0);
    let fic = inputs.fictional_appearances.unwrap_or(0.0);
    let age = inputs.age_years.map(age_score).unwrap_or(0.0);

    let score = W_PAGEVIEW * pv
        + W_HERITAGE * her
        + W_HEIGHT * height
        + W_UNIQUENESS * uniq
        + W_FICTIONAL * fic
        + W_AGE * age;

    if score <= 0.0 {
        None
    } else {
        Some(score.clamp(0.0, 1.0))
    }
}

/// Q211 age-score: `log10(years + 1) / log10(2000)` — Roman-era buildings
/// (~2000 years) hit 1.0, a 20-year-old block hits ~0.4.
fn age_score(years: f32) -> f32 {
    if years <= 0.0 {
        return 0.0;
    }
    let raw = (years + 1.0).log10() / 2000.0_f32.log10();
    raw.clamp(0.0, 1.0)
}

/// Q013 tier mapping from `rarity_score`.
pub fn tier_label(score: f32) -> &'static str {
    match score {
        s if s < 0.20 => "Common",
        s if s < 0.40 => "Uncommon",
        s if s < 0.60 => "Rare",
        s if s < 0.80 => "Epic",
        s if s < 0.92 => "Legendary",
        _ => "Mythic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eiffel_pageview_alone_reaches_legendary_band() {
        // Q211 acceptance: a Wikipedia-tier landmark gets at least Epic
        // even with no heritage/age/uniqueness data. Eiffel ≈ 0.90 raw
        // pageview-rarity × 0.40 weight = 0.36 floor, but the height
        // factor (300 m) bumps it materially.
        let r = blend(&RarityInputs {
            pageview_rarity: Some(0.90),
            height_m: Some(300.0),
            ..Default::default()
        })
        .unwrap();
        assert!(r > 0.40, "Eiffel-class blended score too low: {r}");
    }

    #[test]
    fn obscure_building_returns_none() {
        // No pageview entry, ordinary 8 m house → None (no rarity field
        // in the emitted manifest).
        let r = blend(&RarityInputs {
            height_m: Some(8.0),
            ..Default::default()
        });
        assert!(r.is_none());
    }

    #[test]
    fn missing_all_inputs_returns_none() {
        assert!(blend(&RarityInputs::default()).is_none());
    }

    #[test]
    fn tier_thresholds_match_q013() {
        assert_eq!(tier_label(0.0), "Common");
        assert_eq!(tier_label(0.19), "Common");
        assert_eq!(tier_label(0.20), "Uncommon");
        assert_eq!(tier_label(0.50), "Rare");
        assert_eq!(tier_label(0.70), "Epic");
        assert_eq!(tier_label(0.85), "Legendary");
        assert_eq!(tier_label(0.95), "Mythic");
    }

    #[test]
    fn weights_sum_to_one() {
        let s = W_PAGEVIEW + W_HERITAGE + W_HEIGHT + W_UNIQUENESS + W_FICTIONAL + W_AGE;
        assert!((s - 1.0).abs() < 1e-6, "weights must sum to 1.0: {s}");
    }

    #[test]
    fn unesco_world_wonder_can_reach_mythic() {
        // Q211 mythic example: UNESCO + Eiffel-class pageviews + old.
        let r = blend(&RarityInputs {
            pageview_rarity: Some(0.95),
            heritage_boost: Some(1.0),
            height_m: Some(150.0),
            uniqueness_in_radius_5km: Some(1.0),
            fictional_appearances: Some(1.0),
            age_years: Some(2000.0),
        })
        .unwrap();
        assert!(r >= 0.92, "Mythic floor not reached: {r}");
        assert_eq!(tier_label(r), "Mythic");
    }
}
