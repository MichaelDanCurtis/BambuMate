use super::{slugify, strip_brand, spoolscout, BrandAdapter};

pub struct Esun;

impl BrandAdapter for Esun {
    fn brand_name(&self) -> &str {
        "esun"
    }

    fn brand_aliases(&self) -> Vec<&str> {
        vec!["esun3d"]
    }

    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        let product = strip_brand(filament_name, "esun");
        let product = strip_brand(&product, "esun3d");
        let slug = slugify(&product);

        let mut urls = vec![
            format!("https://www.esun3d.com/{}-product/", slug),
            format!("https://www.esun3d.com/e{}-product/", slug),
        ];
        urls.push(spoolscout::fallback_url("esun", filament_name));
        urls
    }
}
