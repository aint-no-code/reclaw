use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

pub fn now_unix_ms() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        Err(_) => 0,
    }
}

pub fn to_json_text<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

pub fn from_json_text<T: DeserializeOwned>(value: &str) -> Result<T, String> {
    serde_json::from_str::<T>(value).map_err(|error| error.to_string())
}

pub fn value_to_json_text(value: &Value) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

pub fn json_text_to_value(value: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(value).map_err(|error| error.to_string())
}
