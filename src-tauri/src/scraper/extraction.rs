use std::time::Duration;

use serde_json;
use tracing::{error, info, warn};

use super::prompts::{build_extraction_prompt, build_html_extraction_prompt, build_knowledge_prompt, filament_specs_json_schema};
use super::types::FilamentSpecs;
use super::validation::validate_specs;

/// Extract filament specifications from page text using an LLM provider.
///
/// Sends the page text with a structured extraction prompt to the specified
/// AI provider, parses the structured JSON response into a FilamentSpecs,
/// and validates the result against physical constraints.
///
/// # Arguments
/// * `page_text` - Plain text content of the manufacturer page (already converted from HTML)
/// * `filament_name` - The filament name to extract specs for
/// * `provider` - AI provider: "claude", "openai", "kimi", or "openrouter"
/// * `model` - Model identifier (e.g., "claude-sonnet-4-20250514", "gpt-4o")
/// * `api_key` - API key for the provider (retrieved from keychain at command layer)
///
/// # Errors
/// Returns descriptive error messages for:
/// - Unsupported provider
/// - Network timeouts (60s)
/// - Non-2xx HTTP responses
/// - Invalid JSON from LLM
/// - JSON that doesn't match FilamentSpecs schema
pub async fn extract_specs(
    page_text: &str,
    filament_name: &str,
    provider: &str,
    model: &str,
    api_key: &str,
) -> Result<FilamentSpecs, String> {
    let prompt = build_extraction_prompt(filament_name, page_text);
    let schema = filament_specs_json_schema();

    info!(
        "Extracting specs for '{}' using provider '{}' model '{}'",
        filament_name, provider, model
    );

    // Call the appropriate provider API
    let response_text = match provider {
        "claude" => call_claude(api_key, model, &prompt, &schema).await?,
        "openai" => call_openai(api_key, model, &prompt, &schema).await?,
        "kimi" => call_kimi(api_key, model, &prompt).await?,
        "openrouter" => call_openrouter(api_key, model, &prompt, &schema).await?,
        _ => {
            let msg = format!(
                "Unsupported AI provider: '{}'. Supported: claude, openai, kimi, openrouter",
                provider
            );
            error!("{}", msg);
            return Err(msg);
        }
    };

    // Parse LLM response into intermediate JSON first
    let response_text = strip_markdown_json(&response_text);
    let response_json: serde_json::Value = serde_json::from_str(&response_text).map_err(|e| {
        let truncated = if response_text.len() > 500 {
            format!("{}...", &response_text[..500])
        } else {
            response_text.clone()
        };
        let msg = format!(
            "Failed to parse LLM response as JSON: {}. Raw response (first 500 chars): {}",
            e, truncated
        );
        error!("{}", msg);
        msg
    })?;

    // Map the LLM response JSON to our FilamentSpecs struct.
    // The LLM schema uses "confidence" but our struct uses "extraction_confidence",
    // and the LLM schema doesn't include source_url or diameter_mm.
    let specs = map_response_to_specs(&response_json, filament_name).map_err(|e| {
        let msg = format!("LLM response JSON does not match FilamentSpecs schema: {}", e);
        error!("{}", msg);
        msg
    })?;

    // Validate against physical constraints
    let warnings = validate_specs(&specs);
    for w in &warnings {
        warn!(
            "Validation warning for '{}': {} (field: {}, value: {})",
            filament_name, w.message, w.field, w.value
        );
    }

    info!(
        "Extracted specs for '{}': confidence={}, warnings={}",
        filament_name,
        specs.extraction_confidence,
        warnings.len()
    );

    Ok(specs)
}

/// Extract filament specifications from raw HTML using an LLM provider.
///
/// Unlike `extract_specs` which receives pre-converted plain text, this function
/// sends the raw HTML directly to the LLM. This preserves structural information
/// (tables, meta tags, JSON-LD, spec lists) that gets lost during HTML-to-text
/// conversion, resulting in significantly better extraction accuracy.
///
/// The HTML is truncated to ~50K characters to stay within context limits.
pub async fn extract_specs_from_html(
    html: &str,
    filament_name: &str,
    provider: &str,
    model: &str,
    api_key: &str,
) -> Result<FilamentSpecs, String> {
    let prompt = build_html_extraction_prompt(filament_name, html);
    let schema = filament_specs_json_schema();

    info!(
        "Extracting specs from HTML for '{}' using provider '{}' model '{}'",
        filament_name, provider, model
    );

    // Call the appropriate provider API
    let response_text = match provider {
        "claude" => call_claude(api_key, model, &prompt, &schema).await?,
        "openai" => call_openai(api_key, model, &prompt, &schema).await?,
        "kimi" => call_kimi(api_key, model, &prompt).await?,
        "openrouter" => call_openrouter(api_key, model, &prompt, &schema).await?,
        _ => {
            let msg = format!(
                "Unsupported AI provider: '{}'. Supported: claude, openai, kimi, openrouter",
                provider
            );
            error!("{}", msg);
            return Err(msg);
        }
    };

    // Parse LLM response into intermediate JSON
    let response_text = strip_markdown_json(&response_text);
    let response_json: serde_json::Value = serde_json::from_str(&response_text).map_err(|e| {
        let truncated = if response_text.len() > 500 {
            format!("{}...", &response_text[..500])
        } else {
            response_text.clone()
        };
        let msg = format!(
            "Failed to parse LLM response as JSON: {}. Raw response (first 500 chars): {}",
            e, truncated
        );
        error!("{}", msg);
        msg
    })?;

    let specs = map_response_to_specs(&response_json, filament_name).map_err(|e| {
        let msg = format!("LLM response JSON does not match FilamentSpecs schema: {}", e);
        error!("{}", msg);
        msg
    })?;

    // Validate against physical constraints
    let warnings = validate_specs(&specs);
    for w in &warnings {
        warn!(
            "Validation warning for '{}': {} (field: {}, value: {})",
            filament_name, w.message, w.field, w.value
        );
    }

    info!(
        "Extracted specs from HTML for '{}': confidence={}, warnings={}",
        filament_name,
        specs.extraction_confidence,
        warnings.len()
    );

    Ok(specs)
}

/// Generate filament specifications from AI knowledge (no web scraping needed).
///
/// This is the ultimate fallback when:
/// - The filament is not in the catalog
/// - Web scraping fails or returns no useful content
/// - The user just wants quick recommendations
///
/// The AI uses its training knowledge about 3D printing filaments to provide
/// reasonable settings based on the material type and any brand-specific
/// knowledge it may have.
///
/// # Arguments
/// * `filament_name` - The filament name (e.g., "Sunlu PLA 2.0", "eSUN PETG")
/// * `provider` - AI provider: "claude", "openai", "kimi", or "openrouter"
/// * `model` - Model identifier
/// * `api_key` - API key for the provider
pub async fn generate_specs_from_knowledge(
    filament_name: &str,
    provider: &str,
    model: &str,
    api_key: &str,
) -> Result<FilamentSpecs, String> {
    let prompt = build_knowledge_prompt(filament_name);
    let schema = filament_specs_json_schema();

    info!(
        "Generating specs from AI knowledge for '{}' using provider '{}' model '{}'",
        filament_name, provider, model
    );

    // Call the appropriate provider API
    let response_text = match provider {
        "claude" => call_claude(api_key, model, &prompt, &schema).await?,
        "openai" => call_openai(api_key, model, &prompt, &schema).await?,
        "kimi" => call_kimi(api_key, model, &prompt).await?,
        "openrouter" => call_openrouter(api_key, model, &prompt, &schema).await?,
        _ => {
            let msg = format!(
                "Unsupported AI provider: '{}'. Supported: claude, openai, kimi, openrouter",
                provider
            );
            error!("{}", msg);
            return Err(msg);
        }
    };

    // Parse LLM response into intermediate JSON
    let response_text = strip_markdown_json(&response_text);
    let response_json: serde_json::Value = serde_json::from_str(&response_text).map_err(|e| {
        let truncated = if response_text.len() > 500 {
            format!("{}...", &response_text[..500])
        } else {
            response_text.clone()
        };
        let msg = format!(
            "Failed to parse LLM response as JSON: {}. Raw response: {}",
            e, truncated
        );
        error!("{}", msg);
        msg
    })?;

    // Map the LLM response JSON to our FilamentSpecs struct
    let mut specs = map_response_to_specs(&response_json, filament_name).map_err(|e| {
        let msg = format!("LLM response JSON does not match FilamentSpecs schema: {}", e);
        error!("{}", msg);
        msg
    })?;

    // Mark as AI-generated (no source URL)
    specs.source_url = "ai-knowledge".to_string();

    // Validate against physical constraints
    let warnings = validate_specs(&specs);
    for w in &warnings {
        warn!(
            "Validation warning for '{}': {} (field: {}, value: {})",
            filament_name, w.message, w.field, w.value
        );
    }

    info!(
        "Generated specs from knowledge for '{}': confidence={}, warnings={}",
        filament_name,
        specs.extraction_confidence,
        warnings.len()
    );

    Ok(specs)
}

/// Map the raw LLM response JSON to our FilamentSpecs struct.
/// Handles field name differences (confidence -> extraction_confidence)
/// and adds default values for fields not in the LLM schema (source_url, diameter_mm).
fn map_response_to_specs(
    json: &serde_json::Value,
    filament_name: &str,
) -> Result<FilamentSpecs, String> {
    let name = json["name"]
        .as_str()
        .unwrap_or(filament_name)
        .to_string();
    let brand = json["brand"]
        .as_str()
        .ok_or("Missing 'brand' field")?
        .to_string();
    let material = json["material"]
        .as_str()
        .ok_or("Missing 'material' field")?
        .to_string();

    Ok(FilamentSpecs {
        name,
        brand,
        material,
        nozzle_temp_min: json["nozzle_temp_min"].as_u64().map(|v| v as u16),
        nozzle_temp_max: json["nozzle_temp_max"].as_u64().map(|v| v as u16),
        bed_temp_min: json["bed_temp_min"].as_u64().map(|v| v as u16),
        bed_temp_max: json["bed_temp_max"].as_u64().map(|v| v as u16),

        // Actual printing temperatures
        nozzle_temperature: json["nozzle_temperature"].as_u64().map(|v| v as u16),
        nozzle_temperature_initial_layer: json["nozzle_temperature_initial_layer"].as_u64().map(|v| v as u16),

        // Per-plate bed temperatures
        hot_plate_temp: json["hot_plate_temp"].as_u64().map(|v| v as u16),
        hot_plate_temp_initial_layer: json["hot_plate_temp_initial_layer"].as_u64().map(|v| v as u16),
        cool_plate_temp: json["cool_plate_temp"].as_u64().map(|v| v as u16),
        cool_plate_temp_initial_layer: json["cool_plate_temp_initial_layer"].as_u64().map(|v| v as u16),
        eng_plate_temp: json["eng_plate_temp"].as_u64().map(|v| v as u16),
        eng_plate_temp_initial_layer: json["eng_plate_temp_initial_layer"].as_u64().map(|v| v as u16),
        textured_plate_temp: json["textured_plate_temp"].as_u64().map(|v| v as u16),
        textured_plate_temp_initial_layer: json["textured_plate_temp_initial_layer"].as_u64().map(|v| v as u16),

        // Flow & volumetric speed
        max_volumetric_speed: json["max_volumetric_speed"].as_f64().map(|v| v as f32),
        filament_flow_ratio: json["filament_flow_ratio"].as_f64().map(|v| v as f32),
        pressure_advance: json["pressure_advance"].as_f64().map(|v| v as f32),

        // Fan/cooling curve
        fan_min_speed: json["fan_min_speed"].as_u64().map(|v| v as u8),
        fan_max_speed: json["fan_max_speed"].as_u64().map(|v| v as u8),
        overhang_fan_speed: json["overhang_fan_speed"].as_u64().map(|v| v as u8),
        close_fan_the_first_x_layers: json["close_fan_the_first_x_layers"].as_u64().map(|v| v as u8),
        additional_cooling_fan_speed: json["additional_cooling_fan_speed"].as_u64().map(|v| v as u8),

        // Legacy fan field
        fan_speed_percent: json["fan_speed_percent"].as_u64().map(|v| v as u8),

        // Cooling slowdown
        slow_down_layer_time: json["slow_down_layer_time"].as_u64().map(|v| v as u8),
        slow_down_min_speed: json["slow_down_min_speed"].as_u64().map(|v| v as u16),

        // Retraction
        retraction_distance_mm: json["retraction_distance_mm"].as_f64().map(|v| v as f32),
        retraction_speed_mm_s: json["retraction_speed_mm_s"].as_u64().map(|v| v as u16),
        deretraction_speed_mm_s: json["deretraction_speed_mm_s"].as_u64().map(|v| v as u16),

        // Overhang/bridge
        bridge_speed: json["bridge_speed"].as_u64().map(|v| v as u16),

        // Physical properties
        density_g_cm3: json["density_g_cm3"].as_f64().map(|v| v as f32),
        diameter_mm: json["diameter_mm"].as_f64().map(|v| v as f32),
        temperature_vitrification: json["temperature_vitrification"].as_u64().map(|v| v as u16),
        filament_cost: json["filament_cost"].as_f64().map(|v| v as f32),

        // Legacy speed
        max_speed_mm_s: json["max_speed_mm_s"].as_u64().map(|v| v as u16),

        // Metadata
        source_url: json["source_url"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        extraction_confidence: json["confidence"]
            .as_f64()
            .unwrap_or(0.0) as f32,
    })
}

/// Strip markdown code fences from LLM response if present.
/// Some providers (especially without strict JSON mode) wrap JSON in ```json ... ```.
fn strip_markdown_json(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        // Remove opening fence (with optional language tag)
        let after_open = if let Some(pos) = trimmed.find('\n') {
            &trimmed[pos + 1..]
        } else {
            trimmed
        };
        // Remove closing fence
        let cleaned = after_open.trim_end();
        if cleaned.ends_with("```") {
            cleaned[..cleaned.len() - 3].trim().to_string()
        } else {
            cleaned.to_string()
        }
    } else {
        trimmed.to_string()
    }
}

/// Build a reqwest client with a 60-second timeout for LLM API calls.
fn build_api_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Handle API response: check status and extract body text.
async fn handle_api_response(
    response: reqwest::Response,
    provider: &str,
) -> Result<String, String> {
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".to_string());
        let truncated = if body.len() > 1024 {
            format!("{}...", &body[..1024])
        } else {
            body
        };
        let msg = format!(
            "LLM API error: {} from {} - {}",
            status, provider, truncated
        );
        error!("{}", msg);
        return Err(msg);
    }
    response
        .text()
        .await
        .map_err(|e| format!("Failed to read API response body from {}: {}", provider, e))
}

/// Call the Anthropic Claude API with JSON output.
/// Uses prompt-based schema guidance instead of strict json_schema mode,
/// because Anthropic limits union-typed parameters to 16 (we have 33 nullable fields).
async fn call_claude(
    api_key: &str,
    model: &str,
    prompt: &str,
    _schema: &serde_json::Value,
) -> Result<String, String> {
    let client = build_api_client()?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 2048,
        "system": "You are a filament specification extraction assistant. Always respond with valid JSON only, no markdown formatting or code blocks.",
        "messages": [
            {"role": "user", "content": prompt}
        ]
    });

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            let msg = if e.is_timeout() {
                format!("LLM API timeout after 60s for provider 'claude'")
            } else {
                format!("LLM API request failed for claude: {}", e)
            };
            error!("{}", msg);
            msg
        })?;

    let body_text = handle_api_response(response, "claude").await?;

    // Parse Anthropic response format: { "content": [{"type": "text", "text": "..."}] }
    let resp_json: serde_json::Value = serde_json::from_str(&body_text).map_err(|e| {
        let msg = format!("Failed to parse Claude API response wrapper: {}", e);
        error!("{}", msg);
        msg
    })?;

    resp_json["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| {
            let msg = "No text content in Claude API response".to_string();
            error!("{}", msg);
            msg
        })
}

/// Call the OpenAI API with structured output (json_schema response_format).
async fn call_openai(
    api_key: &str,
    model: &str,
    prompt: &str,
    schema: &serde_json::Value,
) -> Result<String, String> {
    let client = build_api_client()?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 2048,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "filament_specs",
                "strict": true,
                "schema": schema
            }
        }
    });

    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            let msg = if e.is_timeout() {
                format!("LLM API timeout after 60s for provider 'openai'")
            } else {
                format!("LLM API request failed for openai: {}", e)
            };
            error!("{}", msg);
            msg
        })?;

    let body_text = handle_api_response(response, "openai").await?;

    // Parse OpenAI response format: { "choices": [{"message": {"content": "..."}}] }
    let resp_json: serde_json::Value = serde_json::from_str(&body_text).map_err(|e| {
        let msg = format!("Failed to parse OpenAI API response wrapper: {}", e);
        error!("{}", msg);
        msg
    })?;

    resp_json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| {
            let msg = "No content in OpenAI API response".to_string();
            error!("{}", msg);
            msg
        })
}

/// Call the Kimi (Moonshot) API.
/// Uses standard JSON mode with prompt-based enforcement since
/// Kimi structured output support is unverified.
async fn call_kimi(
    api_key: &str,
    model: &str,
    prompt: &str,
) -> Result<String, String> {
    let client = build_api_client()?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 2048,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "response_format": {
            "type": "json_object"
        }
    });

    let response = client
        .post("https://api.moonshot.cn/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            let msg = if e.is_timeout() {
                format!("LLM API timeout after 60s for provider 'kimi'")
            } else {
                format!("LLM API request failed for kimi: {}", e)
            };
            error!("{}", msg);
            msg
        })?;

    let body_text = handle_api_response(response, "kimi").await?;

    // Parse Kimi response format (same as OpenAI): { "choices": [{"message": {"content": "..."}}] }
    let resp_json: serde_json::Value = serde_json::from_str(&body_text).map_err(|e| {
        let msg = format!("Failed to parse Kimi API response wrapper: {}", e);
        error!("{}", msg);
        msg
    })?;

    resp_json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| {
            let msg = "No content in Kimi API response".to_string();
            error!("{}", msg);
            msg
        })
}

/// Call the OpenRouter API with JSON output.
/// Uses json_object mode instead of strict json_schema, because when routing
/// to Anthropic models, the strict schema fails with 33+ nullable parameters.
async fn call_openrouter(
    api_key: &str,
    model: &str,
    prompt: &str,
    _schema: &serde_json::Value,
) -> Result<String, String> {
    let client = build_api_client()?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 2048,
        "messages": [
            {"role": "system", "content": "You are a filament specification extraction assistant. Always respond with valid JSON only, no markdown formatting or code blocks."},
            {"role": "user", "content": prompt}
        ],
        "response_format": {
            "type": "json_object"
        }
    });

    let response = client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            let msg = if e.is_timeout() {
                format!("LLM API timeout after 60s for provider 'openrouter'")
            } else {
                format!("LLM API request failed for openrouter: {}", e)
            };
            error!("{}", msg);
            msg
        })?;

    let body_text = handle_api_response(response, "openrouter").await?;

    // Parse OpenRouter response (same as OpenAI format)
    let resp_json: serde_json::Value = serde_json::from_str(&body_text).map_err(|e| {
        let msg = format!("Failed to parse OpenRouter API response wrapper: {}", e);
        error!("{}", msg);
        msg
    })?;

    resp_json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| {
            let msg = "No content in OpenRouter API response".to_string();
            error!("{}", msg);
            msg
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_response_to_specs_full() {
        let json = serde_json::json!({
            "name": "Polymaker PLA Pro",
            "brand": "Polymaker",
            "material": "PLA",
            "nozzle_temp_min": 190,
            "nozzle_temp_max": 220,
            "bed_temp_min": 25,
            "bed_temp_max": 60,
            "nozzle_temperature": 210,
            "nozzle_temperature_initial_layer": 215,
            "hot_plate_temp": 55,
            "hot_plate_temp_initial_layer": 55,
            "cool_plate_temp": 50,
            "cool_plate_temp_initial_layer": 50,
            "eng_plate_temp": 55,
            "eng_plate_temp_initial_layer": 55,
            "textured_plate_temp": 55,
            "textured_plate_temp_initial_layer": 55,
            "max_volumetric_speed": 21.0,
            "filament_flow_ratio": 0.98,
            "pressure_advance": 0.04,
            "fan_min_speed": 100,
            "fan_max_speed": 100,
            "overhang_fan_speed": 100,
            "close_fan_the_first_x_layers": 1,
            "additional_cooling_fan_speed": 80,
            "fan_speed_percent": 100,
            "slow_down_layer_time": 8,
            "slow_down_min_speed": 20,
            "max_speed_mm_s": 200,
            "retraction_distance_mm": 0.8,
            "retraction_speed_mm_s": 30,
            "deretraction_speed_mm_s": 30,
            "bridge_speed": 25,
            "density_g_cm3": 1.24,
            "temperature_vitrification": 55,
            "filament_cost": 24.99,
            "confidence": 0.85
        });

        let specs = map_response_to_specs(&json, "Polymaker PLA Pro").unwrap();
        assert_eq!(specs.name, "Polymaker PLA Pro");
        assert_eq!(specs.brand, "Polymaker");
        assert_eq!(specs.material, "PLA");
        assert_eq!(specs.nozzle_temp_min, Some(190));
        assert_eq!(specs.nozzle_temp_max, Some(220));
        assert_eq!(specs.bed_temp_min, Some(25));
        assert_eq!(specs.bed_temp_max, Some(60));
        assert_eq!(specs.nozzle_temperature, Some(210));
        assert_eq!(specs.nozzle_temperature_initial_layer, Some(215));
        assert_eq!(specs.hot_plate_temp, Some(55));
        assert_eq!(specs.hot_plate_temp_initial_layer, Some(55));
        assert_eq!(specs.cool_plate_temp, Some(50));
        assert_eq!(specs.cool_plate_temp_initial_layer, Some(50));
        assert_eq!(specs.eng_plate_temp, Some(55));
        assert_eq!(specs.eng_plate_temp_initial_layer, Some(55));
        assert_eq!(specs.textured_plate_temp, Some(55));
        assert_eq!(specs.textured_plate_temp_initial_layer, Some(55));
        assert_eq!(specs.max_volumetric_speed, Some(21.0));
        assert_eq!(specs.filament_flow_ratio, Some(0.98));
        assert_eq!(specs.pressure_advance, Some(0.04));
        assert_eq!(specs.fan_min_speed, Some(100));
        assert_eq!(specs.fan_max_speed, Some(100));
        assert_eq!(specs.overhang_fan_speed, Some(100));
        assert_eq!(specs.close_fan_the_first_x_layers, Some(1));
        assert_eq!(specs.additional_cooling_fan_speed, Some(80));
        assert_eq!(specs.fan_speed_percent, Some(100));
        assert_eq!(specs.slow_down_layer_time, Some(8));
        assert_eq!(specs.slow_down_min_speed, Some(20));
        assert_eq!(specs.max_speed_mm_s, Some(200));
        assert_eq!(specs.retraction_distance_mm, Some(0.8));
        assert_eq!(specs.retraction_speed_mm_s, Some(30));
        assert_eq!(specs.deretraction_speed_mm_s, Some(30));
        assert_eq!(specs.bridge_speed, Some(25));
        assert_eq!(specs.density_g_cm3, Some(1.24));
        assert_eq!(specs.temperature_vitrification, Some(55));
        assert_eq!(specs.filament_cost, Some(24.99));
        assert_eq!(specs.extraction_confidence, 0.85);
        assert_eq!(specs.source_url, "");
        assert_eq!(specs.diameter_mm, None);
    }

    #[test]
    fn test_map_response_to_specs_with_nulls() {
        let json = serde_json::json!({
            "name": "Test PLA",
            "brand": "TestBrand",
            "material": "PLA",
            "nozzle_temp_min": null,
            "nozzle_temp_max": 210,
            "bed_temp_min": null,
            "bed_temp_max": null,
            "nozzle_temperature": null,
            "nozzle_temperature_initial_layer": null,
            "hot_plate_temp": null,
            "hot_plate_temp_initial_layer": null,
            "cool_plate_temp": null,
            "cool_plate_temp_initial_layer": null,
            "eng_plate_temp": null,
            "eng_plate_temp_initial_layer": null,
            "textured_plate_temp": null,
            "textured_plate_temp_initial_layer": null,
            "max_volumetric_speed": null,
            "filament_flow_ratio": null,
            "pressure_advance": null,
            "fan_min_speed": null,
            "fan_max_speed": null,
            "overhang_fan_speed": null,
            "close_fan_the_first_x_layers": null,
            "additional_cooling_fan_speed": null,
            "fan_speed_percent": null,
            "slow_down_layer_time": null,
            "slow_down_min_speed": null,
            "max_speed_mm_s": null,
            "retraction_distance_mm": null,
            "retraction_speed_mm_s": null,
            "deretraction_speed_mm_s": null,
            "bridge_speed": null,
            "density_g_cm3": null,
            "temperature_vitrification": null,
            "filament_cost": null,
            "confidence": 0.3
        });

        let specs = map_response_to_specs(&json, "Test PLA").unwrap();
        assert_eq!(specs.nozzle_temp_min, None);
        assert_eq!(specs.nozzle_temp_max, Some(210));
        assert_eq!(specs.bed_temp_min, None);
        assert_eq!(specs.nozzle_temperature, None);
        assert_eq!(specs.hot_plate_temp, None);
        assert_eq!(specs.max_volumetric_speed, None);
        assert_eq!(specs.fan_min_speed, None);
        assert_eq!(specs.bridge_speed, None);
        assert_eq!(specs.temperature_vitrification, None);
        assert_eq!(specs.filament_cost, None);
        assert_eq!(specs.extraction_confidence, 0.3);
    }

    #[test]
    fn test_map_response_to_specs_missing_brand() {
        let json = serde_json::json!({
            "name": "Test PLA",
            "material": "PLA",
            "confidence": 0.5
        });

        let result = map_response_to_specs(&json, "Test PLA");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("brand"));
    }

    #[test]
    fn test_map_response_to_specs_missing_material() {
        let json = serde_json::json!({
            "name": "Test PLA",
            "brand": "TestBrand",
            "confidence": 0.5
        });

        let result = map_response_to_specs(&json, "Test PLA");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("material"));
    }

    #[tokio::test]
    async fn test_extract_specs_unsupported_provider() {
        let result = extract_specs("some text", "PLA", "invalid_provider", "model", "key").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Unsupported AI provider"),
            "Expected unsupported provider error, got: {}",
            err
        );
        assert!(err.contains("invalid_provider"));
    }

    #[test]
    fn test_build_api_client_succeeds() {
        let client = build_api_client();
        assert!(client.is_ok());
    }

    #[test]
    fn test_map_response_uses_filament_name_as_fallback() {
        let json = serde_json::json!({
            "brand": "TestBrand",
            "material": "PLA",
            "confidence": 0.5
        });

        let specs = map_response_to_specs(&json, "Fallback Name").unwrap();
        assert_eq!(specs.name, "Fallback Name");
    }
}
