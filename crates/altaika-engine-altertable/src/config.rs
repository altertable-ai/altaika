// Runtime-only. Do not serialize this into profile files.
#[derive(Debug, Clone)]
pub struct AltertableConfig {
    pub flight_host: String,
    pub username: String,
    pub password: String,
    pub insecure: bool,
    pub app_url: Option<String>,
    pub environment_id: Option<String>,
}

impl AltertableConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_vars(|key| std::env::var(key).ok())
    }

    fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        Ok(Self {
            flight_host: get("ALTAIKA_ALTERTABLE_FLIGHT_HOST")
                .unwrap_or_else(|| "flight.altertable.ai:443".to_string()),
            username: required_var(&get, "ALTERTABLE_USER")?,
            password: required_var(&get, "ALTERTABLE_PASSWORD")?,
            insecure: get("ALTAIKA_ALTERTABLE_INSECURE").as_deref() == Some("1"),
            app_url: get("ALTAIKA_ALTERTABLE_APP_URL"),
            environment_id: get("ALTAIKA_ALTERTABLE_ENVIRONMENT_ID"),
        })
    }
}

fn required_var(get: &impl Fn(&str) -> Option<String>, key: &str) -> Result<String, String> {
    get(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reads_runtime_values_without_defaults_for_user() {
        let config = AltertableConfig::from_vars(|key| match key {
            "ALTERTABLE_USER" => Some("agent@example.com".to_string()),
            "ALTERTABLE_PASSWORD" => Some("secret".to_string()),
            "ALTAIKA_ALTERTABLE_FLIGHT_HOST" => Some("localhost:15002".to_string()),
            "ALTAIKA_ALTERTABLE_INSECURE" => Some("1".to_string()),
            "ALTAIKA_ALTERTABLE_APP_URL" => Some("http://localhost:3000".to_string()),
            "ALTAIKA_ALTERTABLE_ENVIRONMENT_ID" => Some("env_123".to_string()),
            _ => None,
        })
        .unwrap();

        assert_eq!(config.flight_host, "localhost:15002");
        assert_eq!(config.username, "agent@example.com");
        assert_eq!(config.password, "secret");
        assert!(config.insecure);
        assert_eq!(config.app_url.as_deref(), Some("http://localhost:3000"));
        assert_eq!(config.environment_id.as_deref(), Some("env_123"));
    }

    #[test]
    fn config_requires_runtime_user() {
        let error = AltertableConfig::from_vars(|_| None).unwrap_err();
        assert_eq!(error, "missing ALTERTABLE_USER");
    }

    #[test]
    fn config_requires_runtime_password() {
        let error = AltertableConfig::from_vars(|key| match key {
            "ALTERTABLE_USER" => Some("agent@example.com".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert_eq!(error, "missing ALTERTABLE_PASSWORD");
    }
}
