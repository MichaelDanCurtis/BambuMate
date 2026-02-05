use super::{slugify, strip_brand, spoolscout, BrandAdapter};

pub struct Overture;

impl BrandAdapter for Overture {
    fn brand_name(&self) -> &str {
        "overture"
    }

    fn brand_aliases(&self) -> Vec<&str> {
        vec!["overture3d"]
    }

    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        let product = strip_brand(filament_name, "overture");
        let product = strip_brand(&product, "overture3d");
        let slug = slugify(&product);
        let full_slug = slugify(filament_name);

        vec![
            format!("https://overture3d.com/products/{}", full_slug),
            format!("https://overture3d.com/products/{}", slug),
            spoolscout::fallback_url("overture", filament_name),
        ]
    }
}
