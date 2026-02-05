use super::{slugify, strip_brand, spoolscout, BrandAdapter};

pub struct Prusament;

impl BrandAdapter for Prusament {
    fn brand_name(&self) -> &str {
        "prusament"
    }

    fn brand_aliases(&self) -> Vec<&str> {
        vec!["prusa"]
    }

    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        let product = strip_brand(filament_name, "prusament");
        let product = strip_brand(&product, "prusa");
        let slug = slugify(&product);

        let mut urls = vec![
            format!("https://www.prusa3d.com/product/prusament-{}/", slug),
        ];
        urls.push(spoolscout::fallback_url("prusament", filament_name));
        urls
    }
}
