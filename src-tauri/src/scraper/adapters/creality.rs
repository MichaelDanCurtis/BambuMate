use super::{slugify, strip_brand, spoolscout, BrandAdapter};

pub struct Creality;

impl BrandAdapter for Creality {
    fn brand_name(&self) -> &str {
        "creality"
    }

    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        let product = strip_brand(filament_name, "creality");
        let slug = slugify(&product);
        let full_slug = slugify(filament_name);

        vec![
            format!("https://store.creality.com/products/{}", full_slug),
            format!("https://store.creality.com/products/{}", slug),
            spoolscout::fallback_url("creality", filament_name),
        ]
    }
}
