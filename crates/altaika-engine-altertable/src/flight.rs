use std::time::Duration;

use arrow::record_batch::RecordBatch;
use arrow_flight::FlightInfo;
use arrow_flight::sql::client::FlightSqlServiceClient;
use futures::TryStreamExt;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

use crate::config::AltertableConfig;

type FlightResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct FlightClient {
    inner: FlightSqlServiceClient<Channel>,
}

impl FlightClient {
    pub async fn connect(config: &AltertableConfig) -> FlightResult<Self> {
        let scheme = if config.insecure { "http" } else { "https" };
        let url = format!("{scheme}://{}", config.flight_host);
        let mut endpoint = Endpoint::from_shared(url)?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60));

        if !config.insecure {
            let host = config
                .flight_host
                .split(':')
                .next()
                .unwrap_or(&config.flight_host);
            endpoint = endpoint.tls_config(ClientTlsConfig::new().domain_name(host.to_string()))?;
        }

        let channel = endpoint.connect().await?;
        let mut inner = FlightSqlServiceClient::new(channel);
        inner.handshake(&config.username, &config.password).await?;

        Ok(Self { inner })
    }

    pub async fn execute_sql(&mut self, sql: &str) -> FlightResult<Vec<RecordBatch>> {
        let flight_info = self.inner.execute(sql.to_string(), None).await?;
        self.collect_results(flight_info).await
    }

    async fn collect_results(&mut self, flight_info: FlightInfo) -> FlightResult<Vec<RecordBatch>> {
        let mut batches = Vec::new();
        for endpoint in flight_info.endpoint {
            if let Some(ticket) = endpoint.ticket {
                let mut stream = self.inner.do_get(ticket).await?;
                while let Some(batch) = stream.try_next().await? {
                    batches.push(batch);
                }
            }
        }
        Ok(batches)
    }
}
