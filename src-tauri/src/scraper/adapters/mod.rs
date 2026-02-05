mod bambu;
mod creality;
mod elegoo;
mod esun;
mod hatchbox;
mod inland;
mod overture;
mod polymaker;
mod prusament;
pub mod spoolscout;
mod sunlu;

/// Trait for brand-specific URL resolution.
/// Each brand adapter knows how to construct candidate URLs
/// for a given filament product name.
pub trait BrandAdapter: Send + Sync {
    /// The canonical brand name (lowercase).
    fn brand_name(&self) -> &str;

    /// Alternative names/spellings for the brand (lowercase).
    fn brand_aliases(&self) -> Vec<&str> {
        vec![]
    }

    /// Given a filament name (with brand prefix removed),
    /// return candidate URLs to scrape in priority order.
    fn resolve_urls(&self, filament_name: &str) -> Vec<String>;

    /// Optional brand-specific search URL for discovery.
    fn search_url(&self, _query: &str) -> Option<String> {
        None
    }
}

/// Return instances of all registered brand adapters.
pub fn all_adapters() -> Vec<Box<dyn BrandAdapter>> {
    vec![
        Box::new(polymaker::Polymaker),
        Box::new(esun::Esun),
        Box::new(hatchbox::Hatchbox),
        Box::new(overture::Overture),
        Box::new(inland::Inland),
        Box::new(prusament::Prusament),
        Box::new(sunlu::Sunlu),
        Box::new(bambu::Bambu),
        Box::new(creality::Creality),
        Box::new(elegoo::Elegoo),
        Box::new(spoolscout::SpoolScout),
    ]
}

/// Find a brand adapter matching the given filament name.
/// Performs case-insensitive matching against brand_name() and brand_aliases().
/// Matches against the first word(s) of the filament name.
/// Returns None if no brand matches (caller should use SpoolScout fallback).
pub fn find_adapter(filament_name: &str) -> Option<Box<dyn BrandAdapter>> {
    let lower = filament_name.to_lowercase();

    for adapter in all_adapters() {
        // Skip SpoolScout -- it's a fallback, not a brand to match
        if adapter.brand_name() == "spoolscout" {
            continue;
        }

        // Check brand_name
        if lower.starts_with(adapter.brand_name()) {
            // Ensure it's a word boundary (next char is space, hyphen, or end)
            let after = &lower[adapter.brand_name().len()..];
            if after.is_empty() || after.starts_with(' ') || after.starts_with('-') {
                return Some(adapter);
            }
        }

        // Check aliases
        for alias in adapter.brand_aliases() {
            if lower.starts_with(alias) {
                let after = &lower[alias.len()..];
                if after.is_empty() || after.starts_with(' ') || after.starts_with('-') {
                    return Some(adapter);
                }
            }
        }
    }

    None
}

/// Convert a product name to a URL-friendly slug.
/// Lowercase, replace spaces/underscores with hyphens, remove non-alphanumeric
/// chars (except hyphens), collapse multiple hyphens.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());

    for c in name.to_lowercase().chars() {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if c == ' ' || c == '_' || c == '-' {
            slug.push('-');
        }
        // Other characters are dropped
    }

    // Collapse multiple hyphens
    let mut result = String::with_capacity(slug.len());
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }

    // Trim leading/trailing hyphens
    result.trim_matches('-').to_string()
}

/// Extract the product portion of a filament name by removing the brand prefix.
/// E.g., "Polymaker PLA Pro" -> "PLA Pro" when brand is "polymaker".
pub fn strip_brand(filament_name: &str, brand: &str) -> String {
    let lower = filament_name.to_lowercase();
    let brand_lower = brand.to_lowercase();

    if lower.starts_with(&brand_lower) {
        let rest = &filament_name[brand_lower.len()..];
        rest.trim_start_matches(|c: char| c == ' ' || c == '-')
            .to_string()
    } else {
        filament_name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_basic() {
        assert_eq!(slugify("PLA Pro Silk"), "pla-pro-silk");
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("PLA+ Pro"), "pla-pro");
        assert_eq!(slugify("e-PLA (Silk)"), "e-pla-silk");
    }

    #[test]
    fn test_slugify_multiple_spaces() {
        assert_eq!(slugify("PLA  Pro   Silk"), "pla-pro-silk");
    }

    #[test]
    fn test_slugify_underscores() {
        assert_eq!(slugify("PLA_Pro_Silk"), "pla-pro-silk");
    }

    #[test]
    fn test_slugify_leading_trailing() {
        assert_eq!(slugify(" PLA Pro "), "pla-pro");
    }

    #[test]
    fn test_find_adapter_polymaker() {
        let adapter = find_adapter("Polymaker PLA Pro");
        assert!(adapter.is_some());
        assert_eq!(adapter.unwrap().brand_name(), "polymaker");
    }

    #[test]
    fn test_find_adapter_esun() {
        let adapter = find_adapter("eSUN PLA+");
        assert!(adapter.is_some());
        assert_eq!(adapter.unwrap().brand_name(), "esun");
    }

    #[test]
    fn test_find_adapter_esun_alias() {
        let adapter = find_adapter("esun3d PLA+");
        assert!(adapter.is_some());
        assert_eq!(adapter.unwrap().brand_name(), "esun");
    }

    #[test]
    fn test_find_adapter_bambu_lab() {
        let adapter = find_adapter("Bambu Lab PLA Basic");
        assert!(adapter.is_some());
        assert_eq!(adapter.unwrap().brand_name(), "bambu");
    }

    #[test]
    fn test_find_adapter_bambulab_alias() {
        let adapter = find_adapter("bambulab PLA Basic");
        assert!(adapter.is_some());
        assert_eq!(adapter.unwrap().brand_name(), "bambu");
    }

    #[test]
    fn test_find_adapter_unknown_brand() {
        let adapter = find_adapter("Unknown Brand X");
        assert!(adapter.is_none());
    }

    #[test]
    fn test_find_adapter_case_insensitive() {
        assert!(find_adapter("POLYMAKER PLA Pro").is_some());
        assert!(find_adapter("polymaker pla pro").is_some());
        assert!(find_adapter("Polymaker PLA Pro").is_some());
    }

    #[test]
    fn test_find_adapter_all_brands() {
        let brands = vec![
            ("Polymaker PLA Pro", "polymaker"),
            ("eSUN PLA+", "esun"),
            ("Hatchbox PLA", "hatchbox"),
            ("Overture PLA Pro", "overture"),
            ("Inland PLA+", "inland"),
            ("Prusament PLA", "prusament"),
            ("SUNLU PLA", "sunlu"),
            ("Bambu Lab PLA Basic", "bambu"),
            ("Creality PLA", "creality"),
            ("ELEGOO PLA", "elegoo"),
        ];

        for (name, expected_brand) in brands {
            let adapter = find_adapter(name);
            assert!(
                adapter.is_some(),
                "Expected adapter for '{}' but got None",
                name
            );
            assert_eq!(
                adapter.unwrap().brand_name(),
                expected_brand,
                "Wrong brand for '{}'",
                name
            );
        }
    }

    #[test]
    fn test_all_adapters_count() {
        let adapters = all_adapters();
        // 10 brands + SpoolScout = 11
        assert_eq!(adapters.len(), 11);
    }

    #[test]
    fn test_strip_brand() {
        assert_eq!(strip_brand("Polymaker PLA Pro", "polymaker"), "PLA Pro");
        assert_eq!(strip_brand("eSUN PLA+", "esun"), "PLA+");
        assert_eq!(strip_brand("Unknown PLA", "other"), "Unknown PLA");
    }

    #[test]
    fn test_adapter_resolve_urls_returns_nonempty() {
        let adapters = all_adapters();
        for adapter in &adapters {
            if adapter.brand_name() == "spoolscout" {
                continue;
            }
            let urls = adapter.resolve_urls("PLA Pro");
            assert!(
                !urls.is_empty(),
                "Adapter '{}' returned no URLs",
                adapter.brand_name()
            );
        }
    }
}
