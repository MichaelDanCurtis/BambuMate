//! Quality-tier classification and recommendation selection.
//!
//! Tier bar for BambuMate is `>= 4`. Recommendation rule:
//! **latest non-preview model in the cheapest tier that meets the min-quality bar.**
//!
//! Fallback chain if the filter is empty:
//! 1. tier >= 4, non-preview
//! 2. tier >= 3, non-preview
//! 3. any visible (allow previews)
//!
//! Within the winning cohort, we group by release month and pick the cheapest
//! by `input_cost + 3 * output_cost` (output-weighted since BambuMate generates
//! long structured JSON).

use chrono::NaiveDate;

use super::catalog::CatalogEntry;

/// Minimum quality tier considered "recommendation-worthy".
pub const MIN_QUALITY_TIER: u8 = 4;

/// Heuristic 1–5 quality tier keyed off provider + model id + display name.
///
/// - 5: current flagship family (Opus, GPT-5, Gemini 3 Pro, Kimi K2.7)
/// - 4: workhorse (Sonnet, GPT-4o, Gemini Flash, Kimi K2.5 / K2.6)
/// - 3: efficient / small (Haiku, GPT-4o-mini, Flash-Lite)
/// - 2: legacy / niche
/// - 1: text-only or otherwise unsuitable (rare — mostly filtered before this)
pub fn classify_tier(provider_key: &str, id: &str, name: &str) -> u8 {
    let hay = format!("{} {}", id, name).to_lowercase();

    match provider_key {
        // models.dev key for Anthropic
        "anthropic" => {
            if hay.contains("opus") {
                5
            } else if hay.contains("sonnet") {
                4
            } else if hay.contains("haiku") {
                3
            } else {
                2
            }
        }
        "openai" => {
            if hay.contains("gpt-5") || hay.contains("gpt-6") || hay.contains("o1-pro") {
                5
            } else if (hay.contains("gpt-4o") && !hay.contains("mini"))
                || (hay.contains("gpt-4.1")
                    && !hay.contains("mini")
                    && !hay.contains("nano"))
                || hay.contains("o1")
                || hay.contains("o3")
                || hay.contains("o4")
            {
                4
            } else if hay.contains("mini") || hay.contains("nano") {
                3
            } else {
                2
            }
        }
        // models.dev key for Kimi
        "moonshotai" => {
            if hay.contains("kimi-k2.7") {
                5
            } else if hay.contains("kimi-k2.5") || hay.contains("kimi-k2.6") {
                4
            } else if hay.contains("kimi-k2") || hay.contains("moonshot-v1-128k") {
                3
            } else {
                2
            }
        }
        "openrouter" => {
            // Look for family fingerprints in the id.
            if hay.contains("opus") || hay.contains("gpt-5") || hay.contains("gemini-3") {
                5
            } else if hay.contains("sonnet")
                || hay.contains("gpt-4o")
                || hay.contains("gemini-2.5-pro")
                || hay.contains("kimi-k2.5")
                || hay.contains("kimi-k2.6")
                || hay.contains("kimi-k2.7")
            {
                4
            } else if hay.contains("haiku") || hay.contains("mini") || hay.contains("flash") {
                3
            } else {
                2
            }
        }
        _ => 3, // Unknown provider: safe middle ground.
    }
}

/// Cost score used to break ties within a release cohort. Output cost is
/// weighted 3× because BambuMate typically emits long JSON.
fn cost_score(e: &CatalogEntry) -> f32 {
    let i = e.input_cost.unwrap_or(f32::INFINITY);
    let o = e.output_cost.unwrap_or(f32::INFINITY);
    i + 3.0 * o
}

fn release_month(e: &CatalogEntry) -> Option<(i32, u32)> {
    e.release_date.map(|d| (year(d), month(d)))
}

fn year(d: NaiveDate) -> i32 {
    use chrono::Datelike;
    d.year()
}
fn month(d: NaiveDate) -> u32 {
    use chrono::Datelike;
    d.month()
}

/// Pick the recommended model id from a vision-filtered candidate list.
///
/// Returns `None` when the candidate list is empty.
pub fn pick_recommended(candidates: &[CatalogEntry]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }

    type StageFn = fn(&CatalogEntry) -> bool;
    let stages: [StageFn; 3] = [
        |e| e.quality_tier >= MIN_QUALITY_TIER && !e.is_preview,
        |e| e.quality_tier >= 3 && !e.is_preview,
        |_| true,
    ];

    for stage in stages {
        let pool: Vec<&CatalogEntry> = candidates.iter().filter(|e| stage(e)).collect();
        if pool.is_empty() {
            continue;
        }
        // Find latest release month present in the pool.
        let latest = pool
            .iter()
            .filter_map(|e| release_month(e))
            .max();
        let cohort: Vec<&CatalogEntry> = if let Some(target) = latest {
            pool.iter()
                .copied()
                .filter(|e| release_month(e) == Some(target))
                .collect()
        } else {
            // No release_date info at all — fall back to whole pool.
            pool.clone()
        };
        // Pick cheapest (output-weighted) inside cohort.
        let winner = cohort.into_iter().min_by(|a, b| {
            cost_score(a)
                .partial_cmp(&cost_score(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if let Some(w) = winner {
            return Some(w.id.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(
        id: &str,
        tier: u8,
        preview: bool,
        date: Option<&str>,
        input_cost: f32,
        output_cost: f32,
    ) -> CatalogEntry {
        CatalogEntry {
            id: id.into(),
            name: id.into(),
            input_modalities: vec!["text".into(), "image".into()],
            release_date: date.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
            input_cost: Some(input_cost),
            output_cost: Some(output_cost),
            context: Some(200_000),
            is_preview: preview,
            quality_tier: tier,
        }
    }

    #[test]
    fn picks_latest_non_preview_at_tier_bar() {
        let candidates = vec![
            make("old-4", 4, false, Some("2024-01-01"), 3.0, 15.0),
            make("new-5-preview", 5, true, Some("2026-03-01"), 15.0, 75.0),
            make("new-4", 4, false, Some("2026-02-01"), 3.0, 15.0),
            make("new-3", 3, false, Some("2026-04-01"), 0.25, 1.25),
        ];
        assert_eq!(pick_recommended(&candidates), Some("new-4".to_string()));
    }

    #[test]
    fn picks_cheapest_in_same_release_month() {
        let candidates = vec![
            make("expensive", 4, false, Some("2026-02-15"), 10.0, 30.0),
            make("cheap", 4, false, Some("2026-02-05"), 3.0, 15.0),
        ];
        assert_eq!(pick_recommended(&candidates), Some("cheap".to_string()));
    }

    #[test]
    fn falls_back_to_tier_3_when_no_tier_4() {
        let candidates = vec![make("t3", 3, false, Some("2026-01-01"), 1.0, 3.0)];
        assert_eq!(pick_recommended(&candidates), Some("t3".to_string()));
    }

    #[test]
    fn falls_back_to_preview_when_nothing_stable() {
        let candidates = vec![make("t5-preview", 5, true, Some("2026-05-01"), 5.0, 20.0)];
        assert_eq!(
            pick_recommended(&candidates),
            Some("t5-preview".to_string())
        );
    }

    #[test]
    fn empty_returns_none() {
        assert!(pick_recommended(&[]).is_none());
    }

    #[test]
    fn classify_tier_known_families() {
        assert!(classify_tier("anthropic", "claude-3-5-sonnet-latest", "Claude Sonnet") >= 4);
        assert!(classify_tier("anthropic", "claude-3-5-haiku", "Claude Haiku") == 3);
        assert!(classify_tier("openai", "gpt-4o", "GPT-4o") == 4);
        assert!(classify_tier("openai", "gpt-4o-mini", "GPT-4o mini") == 3);
        assert!(classify_tier("moonshotai", "kimi-k2.6", "Kimi K2.6") == 4);
        assert!(classify_tier("moonshotai", "kimi-k2.7-code", "Kimi K2.7 Code") == 5);
    }
}
