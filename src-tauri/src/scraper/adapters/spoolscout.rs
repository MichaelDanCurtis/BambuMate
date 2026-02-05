use super::{slugify, BrandAdapter};

pub struct SpoolScout;

impl BrandAdapter for SpoolScout {
    fn brand_name(&self) -> &str {
        "spoolscout"
    }

    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        // SpoolScout is used as a fallback; generic URL based on full name
        let slug = slugify(filament_name);
        vec![format!("https://www.spoolscout.com/data-sheets/{}", slug)]
    }
}

/// Construct a SpoolScout fallback URL for a given brand and filament name.
/// Other adapters call this to include SpoolScout as a secondary source.
pub fn fallback_url(brand: &str, filament_name: &str) -> String {
    let product = super::strip_brand(filament_name, brand);
    let product_slug = slugify(&product);
    let brand_slug = slugify(brand);
    format!(
        "https://www.spoolscout.com/data-sheets/{}/{}",
        brand_slug, product_slug
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_url() {
        let url = fallback_url("polymaker", "Polymaker PLA Pro");
        assert_eq!(url, "https://www.spoolscout.com/data-sheets/polymaker/pla-pro");
    }

    #[test]
    fn test_fallback_url_esun() {
        let url = fallback_url("esun", "eSUN PLA+");
        assert_eq!(url, "https://www.spoolscout.com/data-sheets/esun/pla");
    }

    #[test]
    fn test_spoolscout_adapter_resolve_urls() {
        let adapter = SpoolScout;
        let urls = adapter.resolve_urls("Polymaker PLA Pro");
        assert_eq!(urls.len(), 1);
        assert!(urls[0].contains("spoolscout.com"));
    }
}
