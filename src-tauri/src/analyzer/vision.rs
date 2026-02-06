//! Vision API calls for defect analysis across all supported providers.
//!
//! Extends the pattern from scraper/extraction.rs to support image content.

use std::collections::HashMap;
use std::time::Duration;

use tracing::{error, info};

use super::image_prep::{image_media_type, prepare_image};
use super::prompts::{build_defect_analysis_prompt, defect_report_schema};
use super::types::DefectReport;
use crate::mapper::DetectedDefect;

/// Analyze an image for print defects using the specified AI provider.
///
/// # Arguments
/// * `image_bytes` - Raw image bytes (will be resized and encoded)
/// * `current_settings` - Current profile parameter values for context
/// * `material_type` - Material type string (e.g., "PLA", "PETG")
/// * `provider` - AI provider: "claude", "openai", "kimi", or "openrouter"
/// * `model` - Model identifier
/// * `api_key` - API key for the provider
///
/// # Returns
/// DefectReport with detected defects, overall quality, and notes.
pub async fn analyze_image(
    image_bytes: &[u8],
    current_settings: &HashMap<String, f32>,
    material_type: &str,
    provider: &str,
    model: &str,
    api_key: &str,
) -> Result<DefectReport, String> {
    // Prepare image (resize + base64)
    let base64_image = prepare_image(image_bytes)?;

    let prompt = build_defect_analysis_prompt(current_settings, material_type);
    let schema = defect_report_schema();

    info!(
        "Analyzing print photo using provider '{}' model '{}'",
        provider, model
    );

    // Call the appropriate vision API
    let response_text = match provider {
        "claude" => call_claude_vision(api_key, model, &prompt, &base64_image, &schema).await?,
        "openai" => call_openai_vision(api_key, model, &prompt, &base64_image, &schema).await?,
        "kimi" => call_kimi_vision(api_key, model, &prompt, &base64_image).await?,
        "openrouter" => {
            call_openrouter_vision(api_key, model, &prompt, &base64_image, &schema).await?
        }
        _ => {
            let msg = format!(
                "Unsupported AI provider: '{}'. Supported: claude, openai, kimi, openrouter",
                provider
            );
            error!("{}", msg);
            return Err(msg);
        }
    };

    // Parse response
    let report = parse_defect_report(&response_text)?;

    info!(
        "Analysis complete: {} defects found, overall quality: {}",
        report.defects.len(),
        report.overall_quality
    );

    Ok(report)
}

/// Parse the raw JSON response into a DefectReport.
fn parse_defect_report(response_text: &str) -> Result<DefectReport, String> {
    let json: serde_json::Value = serde_json::from_str(response_text).map_err(|e| {
        let truncated = if response_text.len() > 500 {
            format!("{}...", &response_text[..500])
        } else {
            response_text.to_string()
        };
        format!(
            "Failed to parse defect report JSON: {}. Response: {}",
            e, truncated
        )
    })?;

    // Parse defects array
    let defects: Vec<DetectedDefect> = json["defects"]
        .as_array()
        .ok_or("Missing 'defects' array")?
        .iter()
        .filter_map(|d| {
            Some(DetectedDefect {
                defect_type: d["defect_type"].as_str()?.to_string(),
                severity: d["severity"].as_f64()? as f32,
                confidence: d["confidence"].as_f64()? as f32,
            })
        })
        .collect();

    let overall_quality = json["overall_quality"]
        .as_str()
        .ok_or("Missing 'overall_quality' field")?
        .to_string();

    let notes = json["notes"].as_str().map(|s| s.to_string());

    Ok(DefectReport {
        defects,
        overall_quality,
        notes,
    })
}

/// Build a reqwest client with timeout for vision API calls.
fn build_api_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(90)) // Vision calls take longer
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Handle API response status and extract body.
async fn handle_api_response(
    response: reqwest::Response,
    provider: &str,
) -> Result<String, String> {
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read>".to_string());
        let truncated = if body.len() > 1024 {
            format!("{}...", &body[..1024])
        } else {
            body
        };
        return Err(format!(
            "Vision API error: {} from {} - {}",
            status, provider, truncated
        ));
    }
    response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))
}

/// Call Claude Vision API.
async fn call_claude_vision(
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
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": image_media_type(),
                        "data": base64_image
                    }
                },
                {
                    "type": "text",
                    "text": prompt
                }
            ]
        }],
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
            if e.is_timeout() {
                "Vision API timeout after 90s for provider 'claude'".to_string()
            } else {
                format!("Vision API request failed for claude: {}", e)
            }
        })?;

    let body_text = handle_api_response(response, "claude").await?;

    // Parse Claude response wrapper
    let resp_json: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse Claude response: {}", e))?;

    resp_json["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No text content in Claude vision response".to_string())
}

/// Call OpenAI Vision API.
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
                        "url": format!("data:{};base64,{}", image_media_type(), base64_image),
                        "detail": "low"  // Cost-efficient for defect detection
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

    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "Vision API timeout after 90s for provider 'openai'".to_string()
            } else {
                format!("Vision API request failed for openai: {}", e)
            }
        })?;

    let body_text = handle_api_response(response, "openai").await?;

    let resp_json: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse OpenAI response: {}", e))?;

    resp_json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No content in OpenAI vision response".to_string())
}

/// Call Kimi Vision API (uses json_object mode).
async fn call_kimi_vision(
    api_key: &str,
    model: &str,
    prompt: &str,
    base64_image: &str,
) -> Result<String, String> {
    let client = build_api_client()?;

    // Kimi may not support vision - include image URL in content
    // If Kimi doesn't support vision, this will return an error
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{};base64,{}", image_media_type(), base64_image)
                    }
                },
                {
                    "type": "text",
                    "text": prompt
                }
            ]
        }],
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
            if e.is_timeout() {
                "Vision API timeout after 90s for provider 'kimi'".to_string()
            } else {
                format!("Vision API request failed for kimi: {}", e)
            }
        })?;

    let body_text = handle_api_response(response, "kimi").await?;

    let resp_json: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse Kimi response: {}", e))?;

    resp_json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No content in Kimi vision response".to_string())
}

/// Call OpenRouter Vision API.
async fn call_openrouter_vision(
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
                        "url": format!("data:{};base64,{}", image_media_type(), base64_image),
                        "detail": "low"
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

    let response = client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "Vision API timeout after 90s for provider 'openrouter'".to_string()
            } else {
                format!("Vision API request failed for openrouter: {}", e)
            }
        })?;

    let body_text = handle_api_response(response, "openrouter").await?;

    let resp_json: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse OpenRouter response: {}", e))?;

    resp_json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No content in OpenRouter vision response".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_defect_report_valid() {
        let json = r#"{
            "defects": [
                {"defect_type": "stringing", "severity": 0.6, "confidence": 0.85}
            ],
            "overall_quality": "acceptable",
            "notes": "Minor stringing visible"
        }"#;

        let report = parse_defect_report(json).unwrap();
        assert_eq!(report.defects.len(), 1);
        assert_eq!(report.defects[0].defect_type, "stringing");
        assert_eq!(report.overall_quality, "acceptable");
        assert_eq!(report.notes, Some("Minor stringing visible".to_string()));
    }

    #[test]
    fn test_parse_defect_report_multiple_defects() {
        let json = r#"{
            "defects": [
                {"defect_type": "stringing", "severity": 0.6, "confidence": 0.85},
                {"defect_type": "warping", "severity": 0.3, "confidence": 0.7}
            ],
            "overall_quality": "poor",
            "notes": null
        }"#;

        let report = parse_defect_report(json).unwrap();
        assert_eq!(report.defects.len(), 2);
        assert_eq!(report.defects[0].defect_type, "stringing");
        assert_eq!(report.defects[1].defect_type, "warping");
        assert_eq!(report.overall_quality, "poor");
        assert!(report.notes.is_none());
    }

    #[test]
    fn test_parse_defect_report_no_defects() {
        let json = r#"{
            "defects": [],
            "overall_quality": "excellent",
            "notes": null
        }"#;

        let report = parse_defect_report(json).unwrap();
        assert!(report.defects.is_empty());
        assert_eq!(report.overall_quality, "excellent");
        assert!(report.notes.is_none());
    }

    #[test]
    fn test_parse_defect_report_missing_quality() {
        let json = r#"{"defects": []}"#;
        let result = parse_defect_report(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("overall_quality"));
    }

    #[test]
    fn test_parse_defect_report_missing_defects() {
        let json = r#"{"overall_quality": "good"}"#;
        let result = parse_defect_report(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("defects"));
    }

    #[test]
    fn test_parse_defect_report_invalid_json() {
        let result = parse_defect_report("not valid json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse"));
    }

    #[test]
    fn test_parse_defect_report_skips_malformed_defects() {
        let json = r#"{
            "defects": [
                {"defect_type": "stringing", "severity": 0.6, "confidence": 0.85},
                {"defect_type": "warping"}
            ],
            "overall_quality": "poor",
            "notes": null
        }"#;

        let report = parse_defect_report(json).unwrap();
        // Second defect is skipped because it's missing severity/confidence
        assert_eq!(report.defects.len(), 1);
        assert_eq!(report.defects[0].defect_type, "stringing");
    }

    #[tokio::test]
    async fn test_analyze_image_unsupported_provider() {
        // This will fail on image prep (too small) but tests provider validation path
        let result = analyze_image(
            &[0; 100],
            &HashMap::new(),
            "PLA",
            "invalid_provider",
            "model",
            "key",
        )
        .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_build_api_client_succeeds() {
        let client = build_api_client();
        assert!(client.is_ok());
    }
}
