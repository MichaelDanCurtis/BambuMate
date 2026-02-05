use anyhow::Result;
use serde::Serialize;
use serde_json::ser::{PrettyFormatter, Serializer};
use serde_json::{Map, Value};

/// A Bambu Studio filament profile.
///
/// Wraps the raw JSON `Map<String, Value>` to preserve ALL fields (139+)
/// without needing a typed struct for every field. Typed accessors are
/// provided for the fields BambuMate actively manipulates.
pub struct FilamentProfile {
    data: Map<String, Value>,
}

impl FilamentProfile {
    /// Parse a filament profile from a JSON string.
    pub fn from_json(json: &str) -> Result<Self> {
        let data: Map<String, Value> = serde_json::from_str(json)?;
        Ok(Self { data })
    }

    /// Construct a FilamentProfile from an existing Map.
    pub fn from_map(data: Map<String, Value>) -> Self {
        Self { data }
    }

    /// Serialize to JSON string with 4-space indentation (matching Bambu Studio format).
    /// Appends a trailing newline if not already present.
    pub fn to_json_4space(&self) -> Result<String> {
        let mut buf = Vec::new();
        let formatter = PrettyFormatter::with_indent(b"    ");
        let mut ser = Serializer::with_formatter(&mut buf, formatter);
        self.data.serialize(&mut ser)?;
        let mut s = String::from_utf8(buf)?;
        if !s.ends_with('\n') {
            s.push('\n');
        }
        Ok(s)
    }

    // --- Typed accessors (all return Option) ---

    /// Profile display name (bare string field).
    pub fn name(&self) -> Option<&str> {
        self.data.get("name")?.as_str()
    }

    /// Parent profile name for inheritance (bare string field).
    /// Empty string means fully flattened (no parent).
    pub fn inherits(&self) -> Option<&str> {
        self.data.get("inherits")?.as_str()
    }

    /// Filament material identifier (bare string field).
    pub fn filament_id(&self) -> Option<&str> {
        self.data.get("filament_id")?.as_str()
    }

    /// Filament type (e.g., "PLA", "ABS") -- first element of array field.
    pub fn filament_type(&self) -> Option<&str> {
        self.get_first_array_value("filament_type")
    }

    /// Nozzle temperature(s) -- array field with one element per extruder.
    pub fn nozzle_temperature(&self) -> Option<Vec<&str>> {
        self.get_string_array("nozzle_temperature")
    }

    /// Compatible printers -- array of printer+nozzle strings.
    pub fn compatible_printers(&self) -> Option<Vec<&str>> {
        self.get_string_array("compatible_printers")
    }

    /// Settings ID (bare string, present in system profiles).
    pub fn setting_id(&self) -> Option<&str> {
        self.data.get("setting_id")?.as_str()
    }

    /// Filament settings ID -- array field used as display identifier.
    pub fn filament_settings_id(&self) -> Option<Vec<&str>> {
        self.get_string_array("filament_settings_id")
    }

    // --- Helpers ---

    /// Get the first element of a string array field.
    pub fn get_first_array_value(&self, key: &str) -> Option<&str> {
        self.data.get(key)?.as_array()?.first()?.as_str()
    }

    /// Get all elements of a string array field.
    pub fn get_string_array(&self, key: &str) -> Option<Vec<&str>> {
        self.data
            .get(key)?
            .as_array()?
            .iter()
            .map(|v| v.as_str())
            .collect()
    }

    // --- Mutators ---

    /// Set a bare string field (not array-wrapped).
    pub fn set_string(&mut self, key: &str, value: String) {
        self.data.insert(key.to_string(), Value::String(value));
    }

    /// Set a string array field.
    pub fn set_string_array(&mut self, key: &str, values: Vec<String>) {
        let arr: Vec<Value> = values.into_iter().map(Value::String).collect();
        self.data.insert(key.to_string(), Value::Array(arr));
    }

    // --- Raw access ---

    /// Get a reference to the underlying map.
    pub fn raw(&self) -> &Map<String, Value> {
        &self.data
    }

    /// Get a mutable reference to the underlying map.
    pub fn raw_mut(&mut self) -> &mut Map<String, Value> {
        &mut self.data
    }

    /// Number of fields in the profile.
    pub fn field_count(&self) -> usize {
        self.data.len()
    }
}

/// Metadata from a `.info` companion file (user profiles only).
///
/// Format is INI-like with `key = value` lines.
#[derive(Debug, Clone)]
pub struct ProfileMetadata {
    pub sync_info: String,
    pub user_id: String,
    pub setting_id: String,
    pub base_id: String,
    pub updated_time: u64,
}

impl Default for ProfileMetadata {
    fn default() -> Self {
        Self {
            sync_info: String::new(),
            user_id: String::new(),
            setting_id: String::new(),
            base_id: String::new(),
            updated_time: 0,
        }
    }
}

impl ProfileMetadata {
    /// Serialize to INI-like format matching Bambu Studio's output.
    ///
    /// Empty values produce `key =` (no trailing space), matching the
    /// actual format Bambu Studio writes to disk.
    pub fn to_info_string(&self) -> String {
        fn fmt_field(key: &str, value: &str) -> String {
            if value.is_empty() {
                format!("{} =", key)
            } else {
                format!("{} = {}", key, value)
            }
        }

        format!(
            "{}\n{}\n{}\n{}\n{}\n",
            fmt_field("sync_info", &self.sync_info),
            fmt_field("user_id", &self.user_id),
            fmt_field("setting_id", &self.setting_id),
            fmt_field("base_id", &self.base_id),
            fmt_field("updated_time", &self.updated_time.to_string()),
        )
    }

    /// Parse from INI-like format. Handles missing fields gracefully.
    ///
    /// Supports both `key = value` and `key =` (empty value) formats.
    pub fn from_info_string(content: &str) -> Result<Self> {
        let mut meta = ProfileMetadata::default();
        for line in content.lines() {
            // Split on first " = " or " =" at end of line for empty values
            let (key, value) = if let Some(idx) = line.find(" = ") {
                (&line[..idx], &line[idx + 3..])
            } else if line.ends_with(" =") {
                (&line[..line.len() - 2], "")
            } else {
                continue;
            };

            match key.trim() {
                "sync_info" => meta.sync_info = value.to_string(),
                "user_id" => meta.user_id = value.to_string(),
                "setting_id" => meta.setting_id = value.to_string(),
                "base_id" => meta.base_id = value.to_string(),
                "updated_time" => meta.updated_time = value.parse().unwrap_or(0),
                _ => {} // Ignore unknown fields
            }
        }
        Ok(meta)
    }
}
