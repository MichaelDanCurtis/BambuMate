use std::time::Duration;

use serde_json;
use tracing::{error, info, warn};

use super::prompts::{build_extraction_prompt, filament_specs_json_schema};
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
        max_speed_mm_s: json["max_speed_mm_s"].as_u64().map(|v| v as u16),
        fan_speed_percent: json["fan_speed_percent"].as_u64().map(|v| v as u8),
        retraction_distance_mm: json["retraction_distance_mm"]
            .as_f64()
            .map(|v| v as f32),
        retraction_speed_mm_s: json["retraction_speed_mm_s"].as_u64().map(|v| v as u16),
        density_g_cm3: json["density_g_cm3"].as_f64().map(|v| v as f32),
        diameter_mm: json["diameter_mm"].as_f64().map(|v| v as f32),
        source_url: json["source_url"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        extraction_confidence: json["confidence"]
            .as_f64()
            .unwrap_or(0.0) as f32,
    })
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

/// Call the Anthropic Claude API with structured output.
async fn call_claude(
    api_key: &str,
    model: &str,
    prompt: &str,
    schema: &serde_json::Value,
) -> Result<String, String> {
    let client = build_api_client()?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "output_config": {
            "format": {
                "type": "json_schema",
                "schema": schema
            }
        }
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
        "max_tokens": 1024,
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
        "max_tokens": 1024,
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

/// Call the OpenRouter API with structured output (json_schema response_format).
/// Uses the same format as OpenAI since OpenRouter is OpenAI-compatible.
async fn call_openrouter(
    api_key: &str,
    model: &str,
    prompt: &str,
    schema: &serde_json::Value,
) -> Result<String, String> {
    let client = build_api_client()?;

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
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
            "max_speed_mm_s": 200,
            "fan_speed_percent": 100,
            "retraction_distance_mm": 0.8,
            "retraction_speed_mm_s": 30,
            "density_g_cm3": 1.24,
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
        assert_eq!(specs.max_speed_mm_s, Some(200));
        assert_eq!(specs.fan_speed_percent, Some(100));
        assert_eq!(specs.retraction_distance_mm, Some(0.8));
        assert_eq!(specs.retraction_speed_mm_s, Some(30));
        assert_eq!(specs.density_g_cm3, Some(1.24));
        assert_eq!(specs.extraction_confidence, 0.85);
        // source_url and diameter_mm default to empty/"" and None
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
            "max_speed_mm_s": null,
            "fan_speed_percent": null,
            "retraction_distance_mm": null,
            "retraction_speed_mm_s": null,
            "density_g_cm3": null,
            "confidence": 0.3
        });

        let specs = map_response_to_specs(&json, "Test PLA").unwrap();
        assert_eq!(specs.nozzle_temp_min, None);
        assert_eq!(specs.nozzle_temp_max, Some(210));
        assert_eq!(specs.bed_temp_min, None);
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
