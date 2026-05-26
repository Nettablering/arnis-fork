//! Minimal Q049 palette table. The full ≈40-key table will land in a
//! later ticket; for now we keep an inline subset so the emitter can pick
//! deterministic, region-appropriate colours.

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub wall: &'static [&'static str],
    pub roof: &'static [&'static str],
}

const NO_RURAL: Palette = Palette {
    wall: &["#8B3A3A", "#F2EAD3", "#3A2D1F"],
    roof: &["#5C3A21", "#3D2914"],
};

const JP_URBAN: Palette = Palette {
    wall: &["#E8E1D3", "#A7A29A", "#3C3936"],
    roof: &["#2B2B2B", "#5A4A3A"],
};

const US_SUBURB: Palette = Palette {
    wall: &["#D9C7A4", "#B86B4B", "#8FAE9E"],
    roof: &["#3D3026", "#5C3A21"],
};

const DEFAULT_PALETTE: Palette = Palette {
    wall: &["#C9B79C", "#9C8A73", "#6F5E4B"],
    roof: &["#4A3A2A", "#2B2B2B"],
};

/// Look up a palette by region key. Unknown keys fall back to a generic
/// muted palette so emitter output is always non-empty.
pub fn palette_for(region_key: &str) -> Palette {
    match region_key {
        k if k.starts_with("NO_") => NO_RURAL,
        k if k.starts_with("JP_") => JP_URBAN,
        k if k.starts_with("US_") => US_SUBURB,
        _ => DEFAULT_PALETTE,
    }
}

/// FNV-1a 64-bit hash of a string. Cheap, deterministic, no_std-friendly —
/// good enough for "pick a palette entry per osm_id".
pub fn fnv1a_64(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

pub fn pick<'a>(items: &'a [&'static str], key: &str, salt: u64) -> &'a str {
    debug_assert!(!items.is_empty(), "palette slice must be non-empty");
    let h = fnv1a_64(key) ^ salt;
    items[(h as usize) % items.len()]
}
