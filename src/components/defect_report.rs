//! Defect report display component.
//!
//! Shows detected defects, recommendations, and conflicts in a readable format.

use leptos::prelude::*;

use crate::components::patterns::{StateKind, StateText};
use crate::components::severity::severity_tier;
use crate::pages::analysis_view::defects_by_severity;
use crate::pages::print_analysis::{Conflict, DefectReport, RecommendationDisplay};

/// Display component for analysis results.
#[component]
pub fn DefectReportDisplay(
    defect_report: DefectReport,
    recommendations: Vec<RecommendationDisplay>,
    conflicts: Vec<Conflict>,
    material_type: String,
    /// Optional profile path for apply functionality
    #[prop(default = None)]
    profile_path: Option<String>,
    /// Callback when user clicks Apply Changes button
    #[prop(default = None)]
    on_apply_click: Option<Callback<()>>,
) -> impl IntoView {
    // Clone for use in apply section
    let has_profile = profile_path.is_some();
    let has_recs = !recommendations.is_empty();
    view! {
        <div class="defect-report">
            <style>{include_str!("defect_report.css")}</style>

            // Detected defects (the overall quality is the page hero)
            <div class="defects-section">
                <h2 class="nd-label">"Defects"</h2>
                {defect_report.notes.clone().map(|notes| view! {
                    <p class="quality-notes">{notes}</p>
                })}
                {if defect_report.defects.is_empty() {
                    view! {
                        <StateText kind=StateKind::Empty message="No defects detected" />
                    }.into_any()
                } else {
                    view! {
                        <ol class="defects-list">
                            {defects_by_severity(&defect_report.defects).into_iter().enumerate().map(|(i, d)| view! {
                                <DefectCard
                                    number=i + 1
                                    defect_type=d.defect_type
                                    severity=d.severity
                                    confidence=d.confidence
                                />
                            }).collect::<Vec<_>>()}
                        </ol>
                    }.into_any()
                }}
            </div>

            // Recommendations
            {(!recommendations.is_empty()).then(|| {
                let mat = material_type.clone();
                view! {
                    <div class="recommendations-section">
                        <h2 class="nd-label">"Recommended Changes"</h2>
                        <p class="section-subtitle">
                            "Adjustments for " <strong>{mat}</strong> " filament:"
                        </p>
                        <div class="recommendations-list">
                            {recommendations.iter().map(|rec| view! {
                                <RecommendationCard recommendation=rec.clone() />
                            }).collect::<Vec<_>>()}
                        </div>
                    </div>
                }
            })}

            // Apply button (only if profile_path provided and recommendations exist)
            {(has_recs && has_profile).then(|| {
                let on_click = on_apply_click.clone();
                view! {
                    <div class="apply-section">
                        <button
                            class="btn btn-primary apply-btn"
                            on:click=move |_| {
                                if let Some(ref cb) = on_click {
                                    cb.run(());
                                }
                            }
                        >
                            "Apply Changes to Profile"
                        </button>
                        <p class="apply-hint">"A backup will be created before any changes are made."</p>
                    </div>
                }
            })}

            // Conflicts warning
            {(!conflicts.is_empty()).then(|| view! {
                <div class="conflicts-section">
                    <h2 class="nd-label">"Conflicts Detected"</h2>
                    <p class="conflicts-warning">
                        "Some defects require opposite adjustments. Review carefully:"
                    </p>
                    <div class="conflicts-list">
                        {conflicts.iter().map(|c| view! {
                            <ConflictCard conflict=c.clone() />
                        }).collect::<Vec<_>>()}
                    </div>
                </div>
            })}
        </div>
    }
}

/// Individual defect card.
#[component]
fn DefectCard(number: usize, defect_type: String, severity: f32, confidence: f32) -> impl IntoView {
    let tier = severity_tier(severity);
    view! {
        <li class="defect-card" data-tier=tier.key()>
            <span class="defect-number nd-mono">{format!("{number:02}")}</span>
            <span class="defect-name">{defect_display_name(&defect_type)}</span>
            <span class=format!("severity-badge severity-{}", tier.key())>{tier.label()}</span>
            <span class="defect-meta nd-mono">
                {format!("Severity {:.0}% · confidence {:.0}%", severity * 100.0, confidence * 100.0)}
            </span>
        </li>
    }
}

/// Individual recommendation card.
#[component]
fn RecommendationCard(recommendation: RecommendationDisplay) -> impl IntoView {
    let clamped_class = if recommendation.was_clamped {
        "recommendation-card was-clamped"
    } else {
        "recommendation-card"
    };

    view! {
        <div class=clamped_class>
            <div class="rec-header">
                <span class="rec-parameter">{recommendation.parameter_label.clone()}</span>
                <span class="rec-priority">
                    {if recommendation.priority == 1 { "Primary" } else { "Secondary" }}
                </span>
            </div>
            <div class="rec-change">
                <span class="change-display">{recommendation.change_display.clone()}</span>
            </div>
            <div class="rec-rationale">
                <span class="rationale-text">{recommendation.rationale.clone()}</span>
            </div>
            <div class="rec-meta">
                <span class="meta-defect">"For: " {defect_display_name(&recommendation.defect)}</span>
                {recommendation.was_clamped.then(|| view! {
                    <span class="meta-clamped" title="Value was limited to safe operating range">
                        "[!] Clamped to safe range"
                    </span>
                })}
            </div>
        </div>
    }
}

/// Conflict card.
#[component]
fn ConflictCard(conflict: Conflict) -> impl IntoView {
    let defects_text = conflict
        .conflicting_defects
        .iter()
        .map(|d| defect_display_name(d))
        .collect::<Vec<_>>()
        .join(", ");

    view! {
        <div class="conflict-card">
            <div class="conflict-icon">"[!]"</div>
            <div class="conflict-content">
                <div class="conflict-param">{conflict.parameter.clone()}</div>
                <div class="conflict-desc">{conflict.description.clone()}</div>
                <div class="conflict-defects">
                    "Affected by: "
                    {defects_text}
                </div>
            </div>
        </div>
    }
}

/// Convert defect type ID to display name.
fn defect_display_name(defect_type: &str) -> String {
    match defect_type {
        "stringing" => "Stringing/Oozing".to_string(),
        "warping" => "Warping".to_string(),
        "layer_adhesion" => "Poor Layer Adhesion".to_string(),
        "elephants_foot" => "Elephant's Foot".to_string(),
        "under_extrusion" => "Under-Extrusion".to_string(),
        "over_extrusion" => "Over-Extrusion".to_string(),
        "z_banding" => "Z-Banding".to_string(),
        _ => defect_type.replace('_', " "),
    }
}
