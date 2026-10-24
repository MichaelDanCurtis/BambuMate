//! Putting an answer together: the scored materials, the products for each,
//! and the page's printer line. Pure, so the command stays a thin shell.

use serde::Serialize;

use super::needs::{with_default, Need};
use super::products::{products_for, Product};
use super::score::{printer_line, score, Excluded, Geometry, MaterialFit, PrinterFacts};
use crate::printer::service::PrinterView;
use crate::scraper::catalog::CatalogEntry;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AdvisedFit {
    #[serde(flatten)]
    pub fit: MaterialFit,
    pub products: Vec<Product>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Advice {
    /// What was scored: tidied, with easy print when nothing was chosen.
    pub needs: Vec<Need>,
    /// Empty when nothing on this printer meets the needs; the page then
    /// shows `score::NOTHING_FITS`.
    pub fits: Vec<AdvisedFit>,
    pub ruled_out: Vec<Excluded>,
    pub printer_line: Option<String>,
    /// Why the model couldn't be read; the advice was scored without geometry.
    pub model_error: Option<String>,
    pub catalog_empty: bool,
}

pub struct AdviceInput<'a> {
    pub needs: &'a [Need],
    pub view: &'a PrinterView,
    /// `Ok(None)` without a model; `Err` is why the model couldn't be read.
    pub geometry: Result<Option<Geometry>, String>,
    pub catalog: &'a [CatalogEntry],
    pub preferred: &'a [String],
}

pub fn build_advice(input: AdviceInput<'_>) -> Advice {
    let facts = PrinterFacts::from_view(input.view);
    let (geometry, model_error) = match input.geometry {
        Ok(g) => (g, None),
        Err(e) => (None, Some(e)),
    };
    let scored = score(input.needs, &facts, geometry.as_ref());
    let fits = scored
        .fits
        .into_iter()
        .map(|fit| {
            let products = products_for(
                fit.material,
                &input.view.slots,
                input.catalog,
                input.preferred,
            );
            AdvisedFit { fit, products }
        })
        .collect();
    Advice {
        needs: with_default(input.needs),
        fits,
        ruled_out: scored.ruled_out,
        printer_line: printer_line(&facts, input.view.configured),
        model_error,
        catalog_empty: input.catalog.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advisor::materials::MaterialId;
    use crate::advisor::products::test_support::{entry, slot};
    use crate::advisor::products::ProductStatus;
    use crate::advisor::score::NO_PRINTER_LINE;
    use crate::printer::client::ConnectionState;
    use crate::printer::service::PrinterSummary;
    use crate::printer::slots::SlotView;
    use crate::printer::state::{Nozzle, PrinterState};

    fn view_with(model: &str, nozzle: &str, slots: Vec<SlotView>) -> PrinterView {
        PrinterView {
            configured: true,
            printer: Some(PrinterSummary {
                ip: "192.168.1.20".into(),
                serial: "0948AB000000001".into(),
                name: "Workshop".into(),
                model: model.into(),
                firmware: None,
                ca_verified: false,
            }),
            connection: ConnectionState::Connected,
            state: Some(PrinterState {
                nozzles: vec![Nozzle {
                    nozzle_type: Some(nozzle.into()),
                    diameter: Some(0.4),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            slots,
            errors: Vec::new(),
        }
    }

    fn input<'a>(
        needs: &'a [Need],
        view: &'a PrinterView,
        catalog: &'a [CatalogEntry],
    ) -> AdviceInput<'a> {
        AdviceInput {
            needs,
            view,
            geometry: Ok(None),
            catalog,
            preferred: &[],
        }
    }

    #[test]
    fn without_a_printer_it_assumes_an_enclosed_one() {
        let view = PrinterView::unconfigured();
        let a = build_advice(input(&[Need::OutdoorUv, Need::Heat80], &view, &[]));
        assert_eq!(a.printer_line.as_deref(), Some(NO_PRINTER_LINE));
        assert_eq!(a.fits[0].fit.material, MaterialId::Asa);
        assert!(a.catalog_empty);
        assert!(a.fits[0].products.is_empty());
        assert_eq!(a.model_error, None);
    }

    #[test]
    fn a_model_that_fails_to_load_still_gets_advice() {
        let view = PrinterView::unconfigured();
        let a = build_advice(AdviceInput {
            geometry: Err("The STL file is cut short.".into()),
            ..input(&[Need::Strong], &view, &[])
        });
        assert_eq!(a.model_error.as_deref(), Some("The STL file is cut short."));
        assert_eq!(a.fits.len(), 3);
    }

    #[test]
    fn ams_loaded_products_come_first() {
        let view = view_with("H2D", "HS01", vec![slot("A2", "PLA", "PLA Basic", true)]);
        let catalog = vec![
            entry("Polymaker", "PLA", "PolyLite PLA"),
            entry("Bambu Lab", "PLA", "Basic"),
        ];
        let a = build_advice(input(&[], &view, &catalog));
        assert_eq!(a.printer_line, None);
        assert_eq!(a.needs, [Need::EasyPrint]);
        let pla = &a.fits[0];
        assert_eq!(pla.fit.material, MaterialId::Pla);
        assert_eq!(pla.products[0].status, ProductStatus::Loaded);
        assert_eq!(pla.products[0].slot.as_deref(), Some("A2"));
        assert_eq!(pla.products[1].brand, "Polymaker");
        assert!(!a.catalog_empty);
    }

    #[test]
    fn on_an_a1_with_a_stainless_nozzle_the_rest_is_not_on_this_printer() {
        let view = view_with("A1", "stainless_steel", vec![]);
        let a = build_advice(input(&[Need::OutdoorUv, Need::Strong], &view, &[]));
        let excluded: Vec<MaterialId> = a.ruled_out.iter().map(|e| e.material).collect();
        use crate::advisor::materials::MaterialId::*;
        assert_eq!(excluded, [PlaCf, PetCf, Abs, Asa, Pa, PaCf, Pc]);
        let fits: Vec<MaterialId> = a.fits.iter().map(|f| f.fit.material).collect();
        assert_eq!(fits, [Petg, Pla, PlaPlus]);
    }

    #[test]
    fn the_json_is_flat_per_fit() {
        let view = PrinterView::unconfigured();
        let v = serde_json::to_value(build_advice(input(&[Need::Flexible], &view, &[]))).unwrap();
        assert_eq!(v["needs"], serde_json::json!(["flexible"]));
        assert_eq!(v["fits"][0]["material"], "tpu");
        assert_eq!(v["fits"][0]["name"], "TPU 95A");
        assert_eq!(v["fits"][0]["reasons"][0]["verdict"], "met");
        assert!(v["fits"][0]["products"].is_array());
        assert_eq!(v["catalog_empty"], true);
    }

    #[test]
    fn empty_fits_pass_through_for_the_page_to_explain() {
        let view = view_with("A1", "stainless_steel", vec![]);
        let a = build_advice(input(&[Need::Heat100], &view, &[]));
        assert!(a.fits.is_empty());
        assert!(!a.ruled_out.is_empty());
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["fits"], serde_json::json!([]));
        assert!(v["ruled_out"][0]["reasons"].is_array());
    }

    #[test]
    fn geometry_notes_reach_the_fit_and_no_model_error_is_set() {
        use crate::advisor::score::{Geometry, OVERHANG_NOTE};
        let view = PrinterView::unconfigured();
        let g = Geometry {
            footprint_mm: [40.0, 40.0],
            height_mm: 30.0,
            overhang: true,
            small_bed_contact: false,
            tall_thin: false,
        };
        let a = build_advice(AdviceInput {
            geometry: Ok(Some(g)),
            ..input(&[Need::Strong], &view, &[])
        });
        assert_eq!(a.model_error, None);
        assert!(a
            .fits
            .iter()
            .all(|f| f.fit.geometry_notes.iter().any(|n| n == OVERHANG_NOTE)));
    }

    #[test]
    fn points_stay_out_of_the_json() {
        let view = PrinterView::unconfigured();
        let v = serde_json::to_value(build_advice(input(&[Need::Strong], &view, &[]))).unwrap();
        let fit = &v["fits"][0];
        assert!(fit.get("points").is_none());
        assert!(fit.get("needs_points").is_none());
        assert!(fit["total"].is_number());
        assert!(fit["fit_pct"].is_number());
        for key in [
            "products",
            "printer_notes",
            "geometry_notes",
            "material_notes",
        ] {
            assert!(fit[key].is_array(), "{key}");
        }
        for key in ["printer_line", "model_error", "catalog_empty"] {
            assert!(v.get(key).is_some(), "{key}");
        }
    }
}
