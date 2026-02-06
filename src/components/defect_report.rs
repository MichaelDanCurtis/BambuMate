//! Defect report display component.
//!
//! Shows detected defects, recommendations, and conflicts in a readable format.

use leptos::prelude::*;

use crate::pages::print_analysis::{Conflict, DefectReport, RecommendationDisplay};

/// Display component for analysis results.
#[component]
pub fn DefectReportDisplay(
    defect_report: DefectReport,
    recommendations: Vec<RecommendationDisplay>,
    conflicts: Vec<Conflict>,
    material_type: String,
) -> impl IntoView {
    view! {
        <div class="defect-report">
            <style>{include_str!("defect_report.css")}</style>

            // Overall quality badge
            <div class="quality-section">
                <h3>"Overall Quality"</h3>
                <QualityBadge quality=defect_report.overall_quality.clone() />
                {defect_report.notes.clone().map(|notes| view! {
                    <p class="quality-notes">{notes}</p>
                })}
            </div>

            // Detected defects
            <div class="defects-section">
                <h3>"Detected Defects"</h3>
                {if defect_report.defects.is_empty() {
                    view! {
                        <p class="no-defects">"No defects detected - your print looks great!"</p>
                    }.into_any()
                } else {
                    view! {
                        <div class="defects-list">
                            {defect_report.defects.iter().map(|d| view! {
                                <DefectCard
                                    defect_type=d.defect_type.clone()
                                    severity=d.severity
                                    confidence=d.confidence
                                />
                            }).collect::<Vec<_>>()}
                        </div>
                    }.into_any()
                }}
            </div>

            // Recommendations
            {(!recommendations.is_empty()).then(|| {
                let mat = material_type.clone();
                view! {
                    <div class="recommendations-section">
                        <h3>"Recommended Changes"</h3>
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

            // Conflicts warning
            {(!conflicts.is_empty()).then(|| view! {
                <div class="conflicts-section">
                    <h3>"Conflicts Detected"</h3>
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

/// Quality badge component.
#[component]
fn QualityBadge(quality: String) -> impl IntoView {
    let (badge_class, icon) = match quality.as_str() {
        "excellent" => ("quality-badge quality-excellent", "[check]"),
        "good" => ("quality-badge quality-good", "[check]"),
        "acceptable" => ("quality-badge quality-acceptable", "~"),
        "poor" => ("quality-badge quality-poor", "!"),
        "failed" => ("quality-badge quality-failed", "X"),
        _ => ("quality-badge", "?"),
    };

    view! {
        <span class=badge_class>
            <span class="quality-icon">{icon}</span>
            <span class="quality-text">{quality.to_uppercase()}</span>
        </span>
    }
}

/// Individual defect card.
#[component]
fn DefectCard(defect_type: String, severity: f32, confidence: f32) -> impl IntoView {
    let display_name = defect_display_name(&defect_type);
    let severity_label = severity_label(severity);
    let severity_class = format!("severity-badge severity-{}", severity_label.to_lowercase());

    view! {
        <div class="defect-card">
            <div class="defect-header">
                <span class="defect-name">{display_name}</span>
                <span class=severity_class>
                    {severity_label}
                </span>
            </div>
            <div class="defect-details">
                <div class="detail-row">
                    <span class="detail-label">"Severity:"</span>
                    <div class="severity-bar">
                        <div class="severity-fill" style=format!("width: {}%", severity * 100.0)></div>
                    </div>
                    <span class="detail-value">{format!("{:.0}%", severity * 100.0)}</span>
                </div>
                <div class="detail-row">
                    <span class="detail-label">"Confidence:"</span>
                    <span class="detail-value">{format!("{:.0}%", confidence * 100.0)}</span>
                </div>
            </div>
        </div>
    }
}

/// Individual recommendation card.
#[component]
fn RecommendationCard(recommendation: RecommendationDisplay) -> impl IntoView {
    let clamped_class = if recommendation.was_clamped { "recommendation-card was-clamped" } else { "recommendation-card" };

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
    let defects_text = conflict.conflicting_defects.iter()
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

/// Convert severity value to label.
fn severity_label(severity: f32) -> String {
    if severity < 0.3 {
        "Minor".to_string()
    } else if severity < 0.5 {
        "Noticeable".to_string()
    } else if severity < 0.7 {
        "Significant".to_string()
    } else {
        "Severe".to_string()
    }
}
