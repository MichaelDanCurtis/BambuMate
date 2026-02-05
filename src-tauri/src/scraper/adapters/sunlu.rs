use super::{slugify, strip_brand, spoolscout, BrandAdapter};

pub struct Sunlu;

impl BrandAdapter for Sunlu {
    fn brand_name(&self) -> &str {
        "sunlu"
    }

    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        let product = strip_brand(filament_name, "sunlu");
        let slug = slugify(&product);
        let full_slug = slugify(filament_name);

        let mut urls = vec![
            format!("https://store.sunlu.com/products/{}", full_slug),
            format!("https://store.sunlu.com/products/{}", slug),
        ];
        urls.push(spoolscout::fallback_url("sunlu", filament_name));
        urls
    }
}
