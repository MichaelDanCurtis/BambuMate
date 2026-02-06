# Phase 6: AI Print Analysis - Research

**Researched:** 2026-02-05
**Domain:** AI Vision APIs for 3D print defect analysis, Image processing, Leptos UI
**Confidence:** HIGH

## Summary

This phase implements photo-to-defect-report analysis using existing AI vision APIs (Claude, OpenAI, Kimi, OpenRouter). The research found that all configured providers support vision capabilities with structured JSON output, enabling reliable extraction of defect types, severity scores, and confidence levels from 3D print photos.

Key discoveries:
1. The existing `extraction.rs` already implements multi-provider AI clients with structured output support - the vision analysis module can follow the same pattern with minimal changes (adding image content blocks)
2. Claude recommends resizing images to max 1568px on the longest edge for optimal cost/performance - our requirement of 1024px is even more conservative and cost-effective
3. The Phase 5 `RuleEngine` is already integrated with material-based safe ranges via `constraints_for_material()` - we just need to connect the AI-detected defects to this engine
4. Leptos has `leptos-use` with `use_drop_zone` for drag-and-drop file handling in WASM

**Primary recommendation:** Extend the existing `extraction.rs` pattern to create a `vision.rs` module that sends photos with a defect analysis prompt to vision-capable models, returning `Vec<DetectedDefect>` that feeds directly into the existing `RuleEngine`.

## Standard Stack

The established libraries/tools for this domain:

### Core (Already in Project)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `reqwest` | 0.12 | HTTP client for AI APIs | Already used for extraction |
| `serde_json` | 1.0 | JSON parsing/generation | Structured output handling |
| `base64` | 0.22 | Image encoding for APIs | Standard for sending images to vision APIs |

### New Dependencies
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `image` | 0.25 | Image loading and resizing | De facto Rust image library |
| `leptos-use` | 0.15 | Drop zone for Leptos frontend | Official Leptos utilities library |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `image` | `fast_image_resize` | Faster but `image` is simpler and sufficient for our single-image-per-request use case |
| `leptos-use` | Custom drag-drop | More work, less maintained |

**Installation:**
```toml
# src-tauri/Cargo.toml
image = "0.25"
base64 = "0.22"

# Cargo.toml (frontend)
leptos-use = { version = "0.15", features = ["use_drop_zone"] }
```

## Architecture Patterns

### Recommended Project Structure
```
src-tauri/src/
├── analyzer/                  # NEW: AI vision analysis module
│   ├── mod.rs                 # Module exports
│   ├── types.rs               # AnalysisRequest, DefectReport
│   ├── vision.rs              # Vision API calls (like extraction.rs)
│   ├── prompts.rs             # Defect analysis prompts
│   └── image_prep.rs          # Image loading, resizing, base64 encoding
├── mapper/                    # EXISTING: Defect-to-recommendation engine
│   ├── engine.rs              # RuleEngine.evaluate() - already done
│   └── types.rs               # DetectedDefect - already done
└── commands/
    └── analyzer.rs            # NEW: Tauri commands for analysis

src/
├── pages/
│   └── print_analysis.rs      # NEW: Photo upload and analysis page
└── components/
    └── defect_report.rs       # NEW: Defect display with recommendations
```

### Pattern 1: Vision API Request with Structured Output
**What:** Send image + prompt to vision-capable AI, get structured JSON defect report
**When to use:** Every photo analysis request
**Example:**
```rust
// Source: https://platform.claude.com/docs/en/build-with-claude/vision
// Claude Vision API with structured output

let body = serde_json::json!({
    "model": model,
    "max_tokens": 1024,
    "messages": [{
        "role": "user",
        "content": [
            {
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/jpeg",
                    "data": base64_image_data
                }
            },
            {
                "type": "text",
                "text": build_defect_analysis_prompt(&profile_context)
            }
        ]
    }],
    "output_config": {
        "format": {
            "type": "json_schema",
            "schema": defect_report_schema()
        }
    }
});
```

### Pattern 2: Image Preparation Pipeline
**What:** Load image, resize to max dimension, encode to base64
**When to use:** Before sending any image to AI
**Example:**
```rust
// Source: https://docs.rs/image/latest/image/
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;

pub fn prepare_image(image_bytes: &[u8], max_dimension: u32) -> Result<String, String> {
    let img = image::load_from_memory(image_bytes)
        .map_err(|e| format!("Failed to load image: {}", e))?;

    let (width, height) = (img.width(), img.height());
    let resized = if width > max_dimension || height > max_dimension {
        let ratio = max_dimension as f32 / width.max(height) as f32;
        let new_width = (width as f32 * ratio) as u32;
        let new_height = (height as f32 * ratio) as u32;
        img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let mut buffer = Cursor::new(Vec::new());
    resized.write_to(&mut buffer, ImageFormat::Jpeg)
        .map_err(|e| format!("Failed to encode image: {}", e))?;

    Ok(base64::engine::general_purpose::STANDARD.encode(buffer.into_inner()))
}
```

### Pattern 3: Analysis Flow with Profile Context
**What:** Load current profile, analyze photo, run through rule engine, return recommendations
**When to use:** Full analysis pipeline
**Example:**
```rust
pub async fn analyze_print(
    image_bytes: &[u8],
    profile_path: &Path,
    provider: &str,
    model: &str,
    api_key: &str,
) -> Result<AnalysisResult, String> {
    // 1. Prepare image (resize to 1024px max)
    let base64_image = prepare_image(image_bytes, 1024)?;

    // 2. Load profile for context
    let profile = ProfileRegistry::load_profile(profile_path)?;
    let current_values = extract_parameter_values(&profile);
    let material = detect_material_type(&profile);

    // 3. Call vision API with profile context
    let defects = call_vision_api(
        &base64_image,
        &current_values,
        provider,
        model,
        api_key,
    ).await?;

    // 4. Run through existing rule engine
    let engine = RuleEngine::new(default_rules());
    let evaluation = engine.evaluate(&defects, &current_values, &material);

    Ok(AnalysisResult {
        defects,
        recommendations: evaluation.recommendations,
        conflicts: evaluation.conflicts,
        profile_context: current_values,
    })
}
```

### Pattern 4: Leptos Drop Zone
**What:** Drag-and-drop photo upload in Leptos/WASM
**When to use:** Analysis page UI
**Example:**
```rust
// Source: https://leptos-use.rs/elements/use_drop_zone.html
use leptos::prelude::*;
use leptos::html::Div;
use leptos_use::{use_drop_zone_with_options, UseDropZoneOptions, UseDropZoneReturn};

#[component]
pub fn PhotoDropZone(on_file: Callback<Vec<u8>>) -> impl IntoView {
    let drop_zone_el = NodeRef::<Div>::new();

    let on_drop = move |event: UseDropZoneEvent| {
        if let Some(file) = event.files.first() {
            spawn_local(async move {
                let array_buffer = file.array_buffer().await.unwrap();
                let bytes = js_sys::Uint8Array::new(&array_buffer).to_vec();
                on_file.call(bytes);
            });
        }
    };

    let UseDropZoneReturn { is_over_drop_zone, .. } =
        use_drop_zone_with_options(
            drop_zone_el,
            UseDropZoneOptions::default().on_drop(on_drop)
        );

    view! {
        <div
            node_ref=drop_zone_el
            class:drop-zone=true
            class:drop-zone-active=move || is_over_drop_zone.get()
        >
            <p>"Drop a photo of your print here"</p>
            <p class="hint">"or click to browse"</p>
        </div>
    }
}
```

### Anti-Patterns to Avoid
- **Sending full-resolution images:** Always resize to 1024px max before API calls (cost + latency)
- **Hardcoding defect types:** Use defect types from `defect_rules.toml` via `RuleEngine.known_defect_types()`
- **Ignoring material type:** Always detect material from profile for safe-range clamping
- **Skipping confidence thresholds:** Filter out low-confidence detections (< 0.5) before showing to user

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Image resizing | Custom pixel manipulation | `image` crate | Edge cases (aspect ratio, color space), well-tested |
| Base64 encoding | Manual encoding | `base64` crate | Correct padding, variants |
| Defect-to-recommendation mapping | Custom rule matching | Existing `RuleEngine` | Already handles severity scaling, conflicts, safe ranges |
| Material constraints | Hardcoded ranges | `constraints_for_material()` | Already defined per material type |
| Drop zone UI | Custom drag events | `leptos-use` `use_drop_zone` | Cross-browser compatibility |
| AI provider switching | Separate implementations | Extend `extraction.rs` pattern | Same providers, same error handling |

**Key insight:** Phase 5 did the hard work - the `RuleEngine` with `DetectedDefect` input types is exactly what we need. Our job is to get AI to produce `Vec<DetectedDefect>` from photos.

## Common Pitfalls

### Pitfall 1: Inconsistent Defect Type Names
**What goes wrong:** AI returns "Stringing" but engine expects "stringing"
**Why it happens:** LLM doesn't know our exact enum values
**How to avoid:** Include exact valid defect type names in prompt, validate against `known_defect_types()`
**Warning signs:** Empty recommendations for photos with obvious defects

### Pitfall 2: Severity Score Calibration
**What goes wrong:** AI always returns 0.9+ severity, recommendations too aggressive
**Why it happens:** No calibration reference in prompt
**How to avoid:** Provide severity scale guidance: "0.3 = minor/cosmetic, 0.5 = noticeable, 0.7 = significant, 0.9 = severe/functional"
**Warning signs:** All recommendations at maximum adjustment amounts

### Pitfall 3: Missing Profile Context Causes Bad Recommendations
**What goes wrong:** Recommends changes that would make things worse
**Why it happens:** AI doesn't know current settings
**How to avoid:** Include key current values in prompt (nozzle temp, bed temp, retraction, flow ratio)
**Warning signs:** Recommendations to lower temp when already at minimum safe range

### Pitfall 4: Image Format/Quality Issues
**What goes wrong:** API rejects image or gives poor analysis
**Why it happens:** Unsupported format, corrupt data, too small
**How to avoid:** Convert all inputs to JPEG, validate minimum size (200px), check for decode errors
**Warning signs:** Vision API errors, "I can't see the image clearly" responses

### Pitfall 5: Token Cost Explosion
**What goes wrong:** Single analysis costs $0.50+ instead of expected $0.05
**Why it happens:** Large images, verbose prompts, requesting unnecessary detail
**How to avoid:** Enforce 1024px max, keep prompts focused, use concise JSON schema
**Warning signs:** Unexpectedly high API bills

## Code Examples

Verified patterns from official sources:

### Defect Analysis JSON Schema
```rust
// Schema for structured output that matches DetectedDefect
pub fn defect_report_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "defects": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "defect_type": {
                            "type": "string",
                            "enum": ["stringing", "warping", "layer_adhesion",
                                    "elephants_foot", "under_extrusion",
                                    "over_extrusion", "z_banding"]
                        },
                        "severity": {
                            "type": "number",
                            "description": "0.0-1.0 scale: 0.3=minor, 0.5=noticeable, 0.7=significant, 0.9=severe"
                        },
                        "confidence": {
                            "type": "number",
                            "description": "0.0-1.0 confidence in detection accuracy"
                        }
                    },
                    "required": ["defect_type", "severity", "confidence"],
                    "additionalProperties": false
                }
            },
            "overall_quality": {
                "type": "string",
                "enum": ["excellent", "good", "acceptable", "poor", "failed"]
            },
            "notes": {
                "type": "string",
                "description": "Brief observation about the print"
            }
        },
        "required": ["defects", "overall_quality"],
        "additionalProperties": false
    })
}
```

### Defect Analysis Prompt Template
```rust
pub fn build_defect_analysis_prompt(
    current_settings: &HashMap<String, f32>,
    material_type: &str,
) -> String {
    format!(r#"Analyze this 3D print photo for defects.

Current print settings:
- Material: {material}
- Nozzle temperature: {nozzle_temp}C
- Bed temperature: {bed_temp}C
- Retraction: {retraction}mm
- Flow ratio: {flow}

Identify any defects from this list:
- stringing: Fine threads between parts
- warping: Corners lifting from bed
- layer_adhesion: Weak layer bonds, delamination
- elephants_foot: First layer bulges outward
- under_extrusion: Gaps, missing material
- over_extrusion: Blobs, rough surfaces
- z_banding: Horizontal lines at regular intervals

For each defect found, rate:
- severity: 0.3=minor/cosmetic, 0.5=noticeable, 0.7=significant, 0.9=severe
- confidence: How certain you are this defect is present

If no defects are visible, return an empty defects array."#,
        material = material_type,
        nozzle_temp = current_settings.get("nozzle_temperature").unwrap_or(&200.0),
        bed_temp = current_settings.get("cool_plate_temp").unwrap_or(&60.0),
        retraction = current_settings.get("filament_retraction_length").unwrap_or(&0.8),
        flow = current_settings.get("filament_flow_ratio").unwrap_or(&1.0),
    )
}
```

### OpenAI Vision API Call Pattern
```rust
// OpenAI uses different structure for vision
// Source: https://platform.openai.com/docs/guides/images-vision
async fn call_openai_vision(
    api_key: &str,
    model: &str,
    prompt: &str,
    base64_image: &str,
    schema: &serde_json::Value,
) -> Result<String, String> {
    let client = build_api_client()?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:image/jpeg;base64,{}", base64_image),
                        "detail": "low"  // Use "low" for cost efficiency
                    }
                },
                {
                    "type": "text",
                    "text": prompt
                }
            ]
        }],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "defect_report",
                "strict": true,
                "schema": schema
            }
        }
    });

    // ... send request and parse response
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Base64 only for images | URL source blocks supported | Nov 2025 | Can skip encoding for web-hosted images |
| `output_format` parameter | `output_config.format` | Late 2025 | More flexible structured output |
| Beta header required | GA for structured outputs | Late 2025 | No special headers needed |

**Deprecated/outdated:**
- Claude `output_format` parameter (use `output_config.format` instead)
- OpenAI Vision preview models (use GPT-4o or newer)

## Open Questions

Things that couldn't be fully resolved:

1. **Kimi K2 Vision Support**
   - What we know: Kimi has standard OpenAI-compatible API
   - What's unclear: Whether Kimi K2 supports vision inputs
   - Recommendation: Test at implementation time; fall back to text-only if unsupported

2. **OpenRouter Model Routing**
   - What we know: OpenRouter is OpenAI-compatible
   - What's unclear: Which models support vision through OpenRouter
   - Recommendation: Document supported models; let user select appropriately

3. **Optimal Image Detail Level for OpenAI**
   - What we know: OpenAI offers "low", "high", "auto" detail levels
   - What's unclear: Whether "low" is sufficient for 3D print analysis
   - Recommendation: Start with "low" for cost savings, add "high" option if quality is insufficient

## Sources

### Primary (HIGH confidence)
- [Claude Vision Documentation](https://platform.claude.com/docs/en/build-with-claude/vision) - Image formats, size limits, API structure, token costs
- [Claude Structured Outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs) - JSON schema, `output_config.format` parameter
- [leptos-use use_drop_zone](https://leptos-use.rs/elements/use_drop_zone.html) - Drop zone implementation pattern

### Secondary (MEDIUM confidence)
- [image crate docs](https://docs.rs/image/latest/image/) - Rust image processing
- [fast_image_resize](https://docs.rs/fast_image_resize) - SIMD-optimized resizing
- Existing `src-tauri/src/scraper/extraction.rs` - AI provider patterns

### Tertiary (LOW confidence)
- Web search results for 3D print defect detection - general domain knowledge

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Using existing project patterns and well-documented APIs
- Architecture: HIGH - Follows established extraction.rs pattern, uses existing RuleEngine
- Pitfalls: MEDIUM - Based on general vision API experience, may discover more during implementation

**Research date:** 2026-02-05
**Valid until:** 2026-03-05 (30 days - APIs are stable)
