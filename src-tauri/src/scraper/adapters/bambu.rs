use super::{slugify, strip_brand, spoolscout, BrandAdapter};

pub struct Bambu;

impl BrandAdapter for Bambu {
    fn brand_name(&self) -> &str {
        "bambu"
    }

    fn brand_aliases(&self) -> Vec<&str> {
        vec!["bambulab", "bambu lab"]
    }

    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        let product = strip_brand(filament_name, "bambu lab");
        let product = strip_brand(&product, "bambulab");
        let product = strip_brand(&product, "bambu");
        let slug = slugify(&product);
        let full_slug = slugify(filament_name);

        let mut urls = vec![
            format!("https://us.store.bambulab.com/products/{}", full_slug),
            format!("https://us.store.bambulab.com/products/{}", slug),
        ];
        urls.push(spoolscout::fallback_url("bambu", filament_name));
        urls
    }
}
