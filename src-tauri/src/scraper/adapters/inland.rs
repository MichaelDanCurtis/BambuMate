use super::{spoolscout, BrandAdapter};

pub struct Inland;

impl BrandAdapter for Inland {
    fn brand_name(&self) -> &str {
        "inland"
    }

    /// Inland is Micro Center's house brand. Product pages have minimal specs.
    /// SpoolScout is the primary source.
    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        vec![spoolscout::fallback_url("inland", filament_name)]
    }
}
