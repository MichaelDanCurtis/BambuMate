use super::{slugify, strip_brand, spoolscout, BrandAdapter};

pub struct Elegoo;

impl BrandAdapter for Elegoo {
    fn brand_name(&self) -> &str {
        "elegoo"
    }

    /// ELEGOO product pages load specs dynamically (JavaScript).
    /// SpoolScout is preferred for spec data.
    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        let product = strip_brand(filament_name, "elegoo");
        let slug = slugify(&product);
        let full_slug = slugify(filament_name);

        vec![
            format!("https://us.elegoo.com/products/{}", full_slug),
            format!("https://us.elegoo.com/products/{}", slug),
            spoolscout::fallback_url("elegoo", filament_name),
        ]
    }
}
