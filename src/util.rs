use crate::types::Duration;

pub(crate) fn duration_from_f64<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    let secs: f64 = serde::de::Deserialize::deserialize(deserializer)?;
    Ok(Duration::from_secs_f64(secs))
}
