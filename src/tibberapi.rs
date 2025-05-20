use crate::utils;
use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::ValueEnum;
use log::{debug, error, info, trace, warn};
use reqwest::blocking;
use serde::{Deserialize, Serialize};
use std::thread;
use std::time::Duration;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum ConnectMode {
    Auto,
    Never,
    Always,
}

#[derive(Debug)]
pub struct TibberClient {
    pub connect_mode: ConnectMode,
    access_token: String,
    home_id: Option<String>,

    max_retries: u32,
    initial_delay_ms: u64,
    max_delay_ms: u64,

    client: blocking::Client,
    api_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GraphQLResponse {
    data: Option<ViewerData>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ViewerData {
    viewer: Viewer,
}

#[derive(Debug, Serialize, Deserialize)]
struct Viewer {
    home: Option<Home>,
    homes: Option<Vec<Home>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Home {
    pub id: Option<String>,

    #[serde(rename = "appNickname")]
    pub app_nickname: Option<String>,

    #[serde(rename = "currentSubscription")]
    pub current_subscription: Option<Subscription>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Subscription {
    #[serde(rename = "priceInfo")]
    pub price_info: PriceInfo,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PriceInfo {
    pub today: Vec<PricePoint>,
    pub tomorrow: Vec<PricePoint>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct PricePoint {
    pub total: f64,
    #[serde(rename = "startsAt")]
    pub starts_at: DateTime<Utc>,
}

impl TibberClient {
    pub fn try_new(
        connect_mode: ConnectMode,
        access_token: Option<&str>,
        home_id: Option<&str>,
        max_retries: u32,
        initial_delay_ms: u64,
        max_delay_ms: u64,
    ) -> Result<Self> {
        if connect_mode != ConnectMode::Never && access_token.is_none() {
            error!("Access token is required when connect mode is not Never");
            return Err(anyhow::anyhow!(
                "Access token is required when connect mode is not Never"
            ));
        }

        debug!("Creating TibberClient with connect_mode={:?}", connect_mode);
        if let Some(home_id) = home_id {
            debug!("Using home_id: {}", home_id);
        }

        Ok(Self {
            connect_mode: connect_mode,
            access_token: access_token.unwrap_or("").to_string(),
            home_id: home_id.map(|s| s.to_string()),
            client: blocking::Client::new(),
            max_retries: max_retries,
            initial_delay_ms: initial_delay_ms,
            max_delay_ms: max_delay_ms,
            api_url: "https://api.tibber.com/v1-beta/gql".to_string(),
        })
    }

    pub fn adjusted_clone(
        &self,
        max_retries: u32,
        initial_delay_ms: u64,
        max_delay_ms: u64,
    ) -> Self {
        Self::try_new(
            self.connect_mode,
            Some(&self.access_token),
            self.home_id.as_deref(),
            max_retries,
            initial_delay_ms,
            max_delay_ms,
        )
        .expect("Unable to clone client")
    }

    #[cfg(test)]
    pub fn set_api_url(&mut self, api_url: String) {
        self.api_url = api_url;
    }

    fn execute_tibber_query(&self, query: &str) -> Result<GraphQLResponse> {
        debug!("Executing Tibber GraphQL query");
        trace!("Query: {}", query);

        let response = self
            .client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .json(&serde_json::json!({
                "query": query
            }))
            .send()?;

        if !response.status().is_success() {
            let status = response.status();
            let response_text = response.text()?;
            error!(
                "HTTP request failed with status {}: {}",
                status, response_text
            );
            return Err(anyhow::anyhow!(
                "HTTP request failed with status {}: {}",
                status,
                response_text
            ));
        }

        debug!("Received successful response from Tibber API");
        let response_text = response.text()?;
        trace!("Response: {}", response_text);

        let gql_response = serde_json::from_str::<GraphQLResponse>(&response_text)?;
        debug!("Successfully parsed GraphQL response");

        Ok(gql_response)
    }

    pub fn fetch_home_ids(&self) -> Vec<Home> {
        info!("Fetching home IDs from Tibber API");
        let query = r#"{viewer{homes{id appNickname}}}"#;
        let response = match self.execute_tibber_query(query) {
            Ok(resp) => resp,
            Err(e) => {
                error!("Failed to fetch home IDs: {}", e);
                return Vec::new();
            }
        };

        let homes = response.data.unwrap().viewer.homes.unwrap();
        debug!("Found {} homes", homes.len());
        homes
    }

    fn fetch_price_info_no_retry(&self) -> Result<PriceInfo> {
        debug!("Fetching price info from Tibber API");
        let home_selector = if let Some(home_id) = &self.home_id {
            debug!("Using specified home ID: {}", home_id);
            format!("home(id: \"{}\")", home_id)
        } else {
            debug!("No home ID specified, using first home");
            "homes".to_string()
        };

        let query = format!(
            r#"{{ viewer {{ {} {{ currentSubscription {{ priceInfo {{ today {{ total startsAt }} tomorrow {{ total startsAt }} }} }} }} }} }}"#,
            home_selector
        );

        let response = self.execute_tibber_query(&query)?;

        let data = response.data.unwrap();

        let home = match data.viewer.home {
            Some(home) => home,
            None => {
                debug!("No specific home found, using first home from list");
                data.viewer.homes.unwrap().first().unwrap().to_owned()
            }
        };

        let current_subscription = home.current_subscription.unwrap();
        let price_info = current_subscription.price_info;

        debug!(
            "Successfully retrieved price info with {} price points for today and {} for tomorrow",
            price_info.today.len(),
            price_info.tomorrow.len()
        );
        Ok(price_info)
    }

    /// Attempts to fetch price info with exponential backoff retry
    pub fn fetch_price_info(&self) -> Result<PriceInfo> {
        info!("Fetching price info");
        let mut attempt = 0;
        let mut delay = self.initial_delay_ms;

        loop {
            attempt += 1;
            debug!("Attempt {} of {}", attempt, self.max_retries);

            match self.fetch_price_info_no_retry() {
                Ok(price_info) => {
                    return Ok(price_info);
                }
                Err(e) => {
                    warn!("Failed to fetch price: {}", e);
                    if attempt > self.max_retries {
                        let error_message = format!(
                            "Failed to fetch price info after {} attempts: {}",
                            self.max_retries, e
                        );
                        return Err(anyhow::anyhow!(error_message));
                    }
                }
            }

            let wait_duration = Duration::from_millis(delay);
            warn!(
                "Waiting {} before next attempt",
                utils::format_std_duration(wait_duration)
            );
            thread::sleep(wait_duration);

            // Exponential backoff with max delay
            delay = (delay * 2).min(self.max_delay_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Server, ServerGuard};

    fn setup_mock_server() -> (ServerGuard, TibberClient) {
        let mock_server = Server::new();

        let mut client = TibberClient::try_new(
            ConnectMode::Auto,
            Some("test-api-key"),
            None,
            3,
            1,  // 1ms: Make sure the tests run fast
            10, // 10ms: Make sure the tests run fast
        )
        .unwrap();

        client.set_api_url(mock_server.url());

        (mock_server, client)
    }

    #[test]
    fn test_get_home_ids() {
        let (mut mock_server, client) = setup_mock_server();

        // Mock the response for home IDs
        let mock_response = r#"{
            "data": {
                "viewer": {
                    "homes": [
                        {
                            "id": "home1",
                            "appNickname": "Home 1"
                        },
                        {
                            "id": "home2",
                            "appNickname": "Home 2"
                        }
                    ]
                }
            }
        }"#;

        let _m = mock_server
            .mock("POST", "/")
            .match_header("Authorization", "Bearer test-api-key")
            .with_status(200)
            .with_body(mock_response)
            .create();

        let homes = client.fetch_home_ids();
        assert_eq!(homes.len(), 2);
        assert_eq!(homes[0].id.as_ref().unwrap(), "home1");
        assert_eq!(homes[0].app_nickname.as_ref().unwrap(), "Home 1");
        assert_eq!(homes[1].id.as_ref().unwrap(), "home2");
        assert_eq!(homes[1].app_nickname.as_ref().unwrap(), "Home 2");
    }

    #[test]
    fn test_get_price_info() {
        let (mut mock_server, client) = setup_mock_server();

        // Mock the response for price info
        let mock_response = r#"{
            "data": {
                "viewer": {
                    "homes": [
                        {
                            "currentSubscription": {
                                "priceInfo": {
                                    "today": [
                                        {
                                            "total": 1.23,
                                            "startsAt": "2024-03-20T10:00:00Z"
                                        }
                                    ],
                                    "tomorrow": [
                                        {
                                            "total": 1.45,
                                            "startsAt": "2024-03-21T10:00:00Z"
                                        }
                                    ]
                                }
                            }
                        }
                    ]
                }
            }
        }"#;

        let _m = mock_server
            .mock("POST", "/")
            .match_header("Authorization", "Bearer test-api-key")
            .with_status(200)
            .with_body(mock_response)
            .create();

        let price_info = client.fetch_price_info().unwrap();
        assert_eq!(price_info.today.len(), 1);
        assert_eq!(price_info.today[0].total, 1.23);
        assert_eq!(price_info.tomorrow.len(), 1);
        assert_eq!(price_info.tomorrow[0].total, 1.45);
    }

    #[test]
    fn test_get_price_info_with_retry() {
        let (mut mock_server, client) = setup_mock_server();

        // Mock a sequence of responses: first two failures, then success
        let _m1 = mock_server
            .mock("POST", "/")
            .match_header("Authorization", "Bearer test-api-key")
            .with_status(500)
            .with_body("Internal Server Error")
            .expect(2)
            .create();

        let mock_response = r#"{
            "data": {
                "viewer": {
                    "homes": [
                        {
                            "currentSubscription": {
                                "priceInfo": {
                                    "today": [
                                        {
                                            "total": 1.23,
                                            "startsAt": "2024-03-20T10:00:00Z"
                                        }
                                    ],
                                    "tomorrow": []
                                }
                            }
                        }
                    ]
                }
            }
        }"#;

        let _m2 = mock_server
            .mock("POST", "/")
            .match_header("Authorization", "Bearer test-api-key")
            .with_status(200)
            .with_body(mock_response)
            .create();

        let price_info = client.fetch_price_info().unwrap();
        assert_eq!(price_info.today.len(), 1);
        assert_eq!(price_info.today[0].total, 1.23);
        assert!(price_info.tomorrow.is_empty());
    }

    #[test]
    fn test_get_price_info_with_retry_max_attempts() {
        let (mut mock_server, client) = setup_mock_server();

        // Mock a sequence of failures
        let _m = mock_server
            .mock("POST", "/")
            .match_header("Authorization", "Bearer test-api-key")
            .with_status(500)
            .with_body("Internal Server Error")
            .expect(3)
            .create();

        let result = client.fetch_price_info();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to fetch price info after 3 attempts")
        );
    }
}
