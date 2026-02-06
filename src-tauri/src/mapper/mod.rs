//! Defect-to-parameter mapping engine for 3D print troubleshooting.
//!
//! This module provides a TOML-driven rule engine that translates detected
//! print defects into ranked, conflict-aware profile parameter recommendations.
//!
//! # Architecture
//!
//! - **Rules**: Loaded from TOML config at startup (or embedded defaults)
//! - **Evaluation**: Defects + current profile -> ranked recommendations
//! - **Conflicts**: Automatically detected when fixes contradict each other
//! - **Safe ranges**: All recommendations clamped to material-specific limits
//!
//! # Example
//!
//! ```ignore
//! use bambumate::mapper::{RuleEngine, default_rules, DetectedDefect};
//! use bambumate::scraper::types::MaterialType;
//!
//! let engine = RuleEngine::new(default_rules());
//!
//! let defects = vec![DetectedDefect {
//!     defect_type: "stringing".to_string(),
//!     severity: 0.7,
//!     confidence: 0.9,
//! }];
//!
//! let current_values = std::collections::HashMap::from([
//!     ("nozzle_temperature".to_string(), 215.0),
//!     ("filament_retraction_length".to_string(), 0.8),
//! ]);
//!
//! let result = engine.evaluate(&defects, &current_values, &MaterialType::PLA);
//!
//! for rec in result.recommendations {
//!     println!("{}: {} -> {} ({})",
//!         rec.parameter, rec.current_value, rec.recommended_value, rec.rationale);
//! }
//!
//! for conflict in result.conflicts {
//!     println!("Warning: {} - {}", conflict.parameter, conflict.description);
//! }
//! ```

mod engine;
mod rules;
mod types;

pub use engine::RuleEngine;
pub use rules::{default_rules, load_rules};
pub use types::*;
