use std::collections::HashSet;
use std::time::Duration;

use crate::net_utils::normalize_port;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Instance-specific configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstanceConfig {
    #[serde(deserialize_with = "deserialize_port")]
    pub stratum_port: String,
    pub min_share_diff: f64,
    #[serde(default, deserialize_with = "deserialize_optional_port")]
    pub prom_port: Option<String>, // Optional per-instance prom port
    pub log_to_file: Option<bool>, // Optional per-instance logging
    #[serde(default, deserialize_with = "deserialize_optional_duration_ms", serialize_with = "serialize_optional_duration_ms")]
    pub block_wait_time: Option<Duration>,
    pub extranonce_size: Option<u8>,
    // Instance-specific settings that can override global defaults
    pub var_diff: Option<bool>,
    pub shares_per_min: Option<u32>,
    pub var_diff_stats: Option<bool>,
    pub pow2_clamp: Option<bool>,
}

/// Global configuration (shared across all instances)
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct GlobalConfig {
    pub karlsend_address: String,
    #[serde(deserialize_with = "deserialize_duration_ms", serialize_with = "serialize_duration_ms")]
    pub block_wait_time: Duration,
    pub print_stats: bool,
    pub log_to_file: bool, // Default for instances that don't specify
    pub health_check_port: String,
    #[serde(deserialize_with = "deserialize_port")]
    pub web_dashboard_port: String,
    pub var_diff: bool,
    pub shares_per_min: u32,
    pub var_diff_stats: bool,
    pub extranonce_size: u8,
    pub pow2_clamp: bool,
    #[serde(deserialize_with = "deserialize_coinbase_tag_suffix")]
    pub coinbase_tag_suffix: Option<String>,
    /// Build the full FishHash dataset (~4.8 GB RAM) instead of the light cache for share validation.
    pub full_dataset: bool,
    /// Lowest difficulty vardiff may assign. FishHash GPUs need sub-1 difficulties.
    pub vardiff_min_diff: f64,
    /// Solo mining: send the network difficulty to miners instead of vardiff (Go bridge `solo_mining`).
    pub solo_mining: bool,
    /// Use `karlsentest:` as the default prefix for wallet addresses given without one (Go bridge `testnet_mining`).
    pub testnet_mining: bool,
}

/// Bridge configuration (supports both single and multi-instance modes)
#[derive(Debug, Serialize)]
pub struct BridgeConfig {
    pub global: GlobalConfig,
    pub instances: Vec<InstanceConfig>,
}

#[derive(Serialize)]
struct BridgeConfigYaml<'a> {
    #[serde(flatten)]
    global: &'a GlobalConfig,
    instances: &'a [InstanceConfig],
}

// Custom deserializers

/// Deserialize a port string and normalize it
fn deserialize_port<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(normalize_port(&s))
}

/// Deserialize an optional port string and normalize it
fn deserialize_optional_port<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.and_then(|s| {
        let normalized = normalize_port(&s);
        if normalized.is_empty() { None } else { Some(normalized) }
    }))
}

/// Deserialize a duration from milliseconds (supports both int and float)
fn deserialize_duration_ms<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let value: serde_yaml::Value = Deserialize::deserialize(deserializer)?;
    parse_duration_value(&value).map_err(D::Error::custom)
}

/// Parse a duration given in milliseconds (number) or as a Go-style string ("500ms", "2s", "1m", "1500").
fn parse_duration_value(v: &serde_yaml::Value) -> Result<Duration, String> {
    match v {
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_u64() {
                Ok(Duration::from_millis(i))
            } else if let Some(f) = n.as_f64().filter(|f| f.is_finite() && *f >= 0.0) {
                Ok(Duration::from_millis(f as u64))
            } else {
                Err("duration must be a non-negative number of milliseconds".into())
            }
        }
        serde_yaml::Value::String(s) => {
            let s = s.trim();
            let (num, unit) = match s.find(|c: char| c.is_ascii_alphabetic()) {
                Some(i) => (&s[..i], &s[i..]),
                None => (s, "ms"),
            };
            let value: f64 = num.trim().parse().map_err(|_| format!("invalid duration '{s}'"))?;
            if !value.is_finite() || value < 0.0 {
                return Err(format!("invalid duration '{s}'"));
            }
            let ms = match unit {
                "ms" => value,
                "s" => value * 1000.0,
                "m" => value * 60_000.0,
                "h" => value * 3_600_000.0,
                _ => return Err(format!("invalid duration unit in '{s}' (use ms, s, m or h)")),
            };
            Ok(Duration::from_millis(ms as u64))
        }
        _ => Err("duration must be a number (ms) or a string like \"2s\"".into()),
    }
}

/// Deserialize an optional duration from milliseconds
fn deserialize_optional_duration_ms<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let value: Option<serde_yaml::Value> = Option::deserialize(deserializer)?;
    value.map(|v| parse_duration_value(&v).map_err(D::Error::custom)).transpose()
}

/// Deserialize coinbase_tag_suffix, converting empty strings to None
fn deserialize_coinbase_tag_suffix<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = Option::<String>::deserialize(deserializer)?;
    Ok(s.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    }))
}

// Custom serializers

/// Serialize a duration as milliseconds (u64)
fn serialize_duration_ms<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_u64(duration.as_millis() as u64)
}

/// Serialize an optional duration as milliseconds (u64)
fn serialize_optional_duration_ms<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match duration {
        Some(d) => serializer.serialize_some(&(d.as_millis() as u64)),
        None => serializer.serialize_none(),
    }
}

/// Raw config structure for deserialization (handles both single and multi-instance modes)
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct BridgeConfigRaw {
    #[serde(flatten)]
    global: GlobalConfig,

    // Multi-instance mode
    #[serde(default)]
    instances: Option<Vec<InstanceConfig>>,

    // Single-instance mode (for backward compatibility)
    #[serde(default, deserialize_with = "deserialize_optional_port")]
    stratum_port: Option<String>,
    #[serde(default)]
    min_share_diff: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_optional_port")]
    prom_port: Option<String>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            karlsend_address: "localhost:42110".to_string(),
            block_wait_time: Duration::from_millis(1000),
            print_stats: true,
            log_to_file: true,
            health_check_port: String::new(),
            web_dashboard_port: String::new(),
            var_diff: true,
            shares_per_min: 20,
            var_diff_stats: false,
            extranonce_size: 0,
            pow2_clamp: false,
            coinbase_tag_suffix: None,
            full_dataset: false,
            vardiff_min_diff: crate::share_handler::DEFAULT_VARDIFF_MIN_DIFF,
            solo_mining: false,
            testnet_mining: false,
        }
    }
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            stratum_port: ":5555".to_string(),
            min_share_diff: 1.0,
            prom_port: None,
            log_to_file: None,
            block_wait_time: None,
            extranonce_size: None,
            var_diff: None,
            shares_per_min: None,
            var_diff_stats: None,
            pow2_clamp: None,
        }
    }
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self { global: GlobalConfig::default(), instances: vec![InstanceConfig::default()] }
    }
}

impl BridgeConfig {
    pub fn from_yaml(content: &str) -> Result<Self, anyhow::Error> {
        // Deserialize using serde_yaml
        let raw: BridgeConfigRaw = serde_yaml::from_str(content)?;

        // Post-process: Handle single-instance mode
        let instances = if let Some(instances) = raw.instances {
            // Multi-instance mode

            // Validate: instances cannot be empty
            if instances.is_empty() {
                return Err(anyhow::anyhow!("instances array cannot be empty"));
            }

            // Validate: required fields are present (serde will error if missing, but we check anyway)
            for (idx, instance) in instances.iter().enumerate() {
                if instance.stratum_port.is_empty() {
                    return Err(anyhow::anyhow!("Instance {} missing required 'stratum_port'", idx));
                }
                if !instance.min_share_diff.is_finite() || instance.min_share_diff < 0.0 {
                    return Err(anyhow::anyhow!("Instance {} has invalid 'min_share_diff' {}", idx, instance.min_share_diff));
                }
            }

            instances
        } else {
            // Single-instance mode (backward compatible)
            let mut instance = InstanceConfig { prom_port: raw.prom_port, ..InstanceConfig::default() };
            if let Some(stratum_port) = raw.stratum_port {
                instance.stratum_port = stratum_port;
            }
            if let Some(min_share_diff) = raw.min_share_diff {
                instance.min_share_diff = min_share_diff;
            }

            vec![instance]
        };

        // Validate: duplicate ports
        let mut ports = HashSet::new();
        for instance in &instances {
            if !ports.insert(&instance.stratum_port) {
                return Err(anyhow::anyhow!("Duplicate stratum_port: {}", instance.stratum_port));
            }
        }

        Ok(BridgeConfig { global: raw.global, instances })
    }

    pub(crate) fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        let yaml = BridgeConfigYaml { global: &self.global, instances: &self.instances };
        serde_yaml::to_string(&yaml)
    }
}
