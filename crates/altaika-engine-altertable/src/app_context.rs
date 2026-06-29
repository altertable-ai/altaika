use serde_json::Value;

use crate::config::AltertableConfig;

pub async fn call_internal_tool(
    config: &AltertableConfig,
    tool_name: &str,
    arguments: &Value,
) -> Result<Option<Value>, reqwest::Error> {
    let (Some(app_url), Some(environment_id)) = (&config.app_url, &config.environment_id) else {
        return Ok(None);
    };

    let url = format!(
        "{}/internal/tools/{tool_name}",
        app_url.trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .post(url)
        .header("x-environment-id", environment_id)
        .json(arguments)
        .send()
        .await?
        .error_for_status()?;

    response.json::<Value>().await.map(Some)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use reqwest::StatusCode;

    use super::*;

    #[tokio::test]
    async fn internal_tool_is_skipped_without_app_context() {
        let config = AltertableConfig {
            flight_host: "localhost:15002".to_string(),
            username: "agent@example.com".to_string(),
            password: "secret".to_string(),
            insecure: true,
            app_url: None,
            environment_id: None,
        };

        let result = call_internal_tool(&config, "get_catalog", &serde_json::json!({}))
            .await
            .unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn internal_tool_rejects_error_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: 16\r\n\r\n{\"error\":\"boom\"}",
                )
                .unwrap();
        });
        let config = AltertableConfig {
            flight_host: "localhost:15002".to_string(),
            username: "agent@example.com".to_string(),
            password: "secret".to_string(),
            insecure: true,
            app_url: Some(format!("http://{addr}")),
            environment_id: Some("env_123".to_string()),
        };

        let error = call_internal_tool(&config, "get_catalog", &serde_json::json!({}))
            .await
            .unwrap_err();
        server.join().unwrap();

        assert_eq!(error.status(), Some(StatusCode::INTERNAL_SERVER_ERROR));
    }
}
