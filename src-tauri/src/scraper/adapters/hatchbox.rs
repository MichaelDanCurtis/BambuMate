use super::{slugify, strip_brand, spoolscout, BrandAdapter};

pub struct Hatchbox;

impl BrandAdapter for Hatchbox {
    fn brand_name(&self) -> &str {
        "hatchbox"
    }

    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        let product = strip_brand(filament_name, "hatchbox");
        let slug = slugify(&product);
        let full_slug = slugify(filament_name);

        vec![
            format!("https://www.hatchbox3d.com/products/{}", full_slug),
            format!("https://www.hatchbox3d.com/products/{}", slug),
            spoolscout::fallback_url("hatchbox", filament_name),
        ]
    }
}
