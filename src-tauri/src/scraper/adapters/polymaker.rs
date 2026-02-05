use super::{slugify, strip_brand, spoolscout, BrandAdapter};

pub struct Polymaker;

impl BrandAdapter for Polymaker {
    fn brand_name(&self) -> &str {
        "polymaker"
    }

    fn resolve_urls(&self, filament_name: &str) -> Vec<String> {
        let product = strip_brand(filament_name, "polymaker");
        let slug = slugify(&product);
        let full_slug = slugify(filament_name);

        let mut urls = vec![
            format!("https://us.polymaker.com/products/{}", full_slug),
            format!("https://us.polymaker.com/products/{}", slug),
        ];
        urls.push(spoolscout::fallback_url("polymaker", filament_name));
        urls
    }

    fn search_url(&self, query: &str) -> Option<String> {
        Some(format!(
            "https://us.polymaker.com/search?q={}",
            urlencoded(query)
        ))
    }
}

fn urlencoded(s: &str) -> String {
    s.replace(' ', "+")
}
