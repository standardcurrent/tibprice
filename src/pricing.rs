use crate::tibberapi::{PricePoint, TibberClient};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Local, NaiveTime, Utc};
use clap::ValueEnum;
use log::{debug, info, trace};
use serde::{Deserialize, Serialize};
use std::fs::{File, rename};
use std::path::Path;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone)]
pub struct PricePoints(Vec<PricePoint>);

#[derive(serde::Serialize)]
pub struct ActivePrice {
    pub price: Option<f64>,
    pub starts_at: Option<DateTime<Local>>,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum OutputFormat {
    None,
    Json,
    JsonPretty,
    Csv,
    Plain,
}

impl Default for ActivePrice {
    fn default() -> Self {
        ActivePrice::new()
    }
}

impl ActivePrice {
    pub fn new() -> Self {
        Self {
            price: None,
            starts_at: None,
        }
    }
    pub fn new_from_price_point(price_point: &PricePoint) -> Self {
        Self {
            price: Some(price_point.total),
            starts_at: Some(price_point.starts_at.with_timezone(&Local)),
        }
    }

    /// Returns the active price as a string.
    /// If there is no active price, it returns an empty string.
    pub fn to_string_pretty(&self, format: &OutputFormat) -> String {
        match format {
            // Compact JSON format (single line without whitespace)
            OutputFormat::Json => serde_json::to_string(&self).expect("Unable to create json"),
            // Pretty-printed JSON format (with indentation and newlines)
            OutputFormat::JsonPretty => {
                serde_json::to_string_pretty(&self).expect("Unable to create json")
            }
            // CSV format (price,starts_at)
            // Missing values are represented as empty strings
            OutputFormat::Csv => {
                let price_str = match self.price {
                    Some(price) => price.to_string(),
                    None => "".to_string(),
                };
                let time_str = match self.starts_at {
                    Some(time) => time.to_string(),
                    None => "".to_string(),
                };
                format!("{},{}", price_str, time_str)
            }
            // Plain text format (price)
            // Missing values are represented as "unavailable"
            OutputFormat::Plain => match self.price {
                Some(price) => price.to_string(),
                None => "unavailable".to_string(),
            },
            _ => String::new(),
        }
    }
}
impl PricePoints {
    const DEFAULT_UPDATE_HOUR: u32 = 13;
    const DEFAULT_UPDATE_MINUTE: u32 = 0;

    // Parse a time string in format "HH:MM" and return a NaiveTime
    pub fn parse_update_time(time_str: &str) -> Result<NaiveTime> {
        // If the time string is empty, use the default values
        if time_str.is_empty() {
            return Ok(NaiveTime::from_hms_opt(
                Self::DEFAULT_UPDATE_HOUR,
                Self::DEFAULT_UPDATE_MINUTE,
                0,
            )
            .unwrap());
        }

        // Split the time string by ":"
        let parts: Vec<&str> = time_str.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow!(
                "Invalid time format. Expected HH:MM, got: {}",
                time_str
            ));
        }

        // Parse hours and minutes
        let hours: u32 = parts[0]
            .parse()
            .map_err(|_| anyhow!("Invalid hour value: {}", parts[0]))?;
        let minutes: u32 = parts[1]
            .parse()
            .map_err(|_| anyhow!("Invalid minute value: {}", parts[1]))?;

        // Validate hours and minutes
        if hours >= 24 {
            return Err(anyhow!(
                "Hour value must be between 0 and 23, got: {}",
                hours
            ));
        }
        if minutes >= 60 {
            return Err(anyhow!(
                "Minute value must be between 0 and 59, got: {}",
                minutes
            ));
        }

        // Create a NaiveTime
        NaiveTime::from_hms_opt(hours, minutes, 0)
            .ok_or_else(|| anyhow!("Failed to create time from {}:{}", hours, minutes))
    }

    pub fn new() -> Self {
        debug!("Creating new empty PricePoints");
        Self(Vec::new())
    }

    #[cfg(test)]
    pub fn from_prices(prices: Vec<PricePoint>) -> Self {
        debug!("Creating PricePoints from {} price points", prices.len());
        Self(prices)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, PricePoint> {
        self.0.iter()
    }

    pub fn get(&self, index: usize) -> Option<&PricePoint> {
        self.0.get(index)
    }

    pub fn last(&self) -> Option<&PricePoint> {
        self.0.last()
    }

    pub fn should_fetch_prices(&self, update_time: &NaiveTime) -> bool {
        trace!("Checking if prices should be fetched");
        // If we are missing today's prices, we can assume that new prices are available.
        if !self.has_today_prices() {
            debug!("Missing today's prices, should fetch new prices");
            return true;
        }

        // If we are missing tomorrow's prices, we can assume that new prices are
        // available if it's after the configured update time.
        if !self.has_tomorrows_prices() {
            let now_local = Local::now();
            let now_time = now_local.time();
            if now_time >= *update_time {
                debug!(
                    "Missing tomorrow's prices and it's after {}, should fetch new prices",
                    update_time.format("%H:%M")
                );
                return true;
            }
        }

        trace!("No need to fetch prices");
        false
    }
    pub fn get_active_price(&self) -> ActivePrice {
        trace!("Getting active price");
        let now_utc = Utc::now();

        if self.is_empty() {
            debug!("Price points is empty, returning empty active price");
            return ActivePrice::default();
        }

        // Find the price point that has starts_at <= now utc < ends_at
        for i in 0..self.len() - 1 {
            let current_price_point = self.get(i).unwrap();
            let ends_at = self.get(i + 1).unwrap().starts_at;

            if current_price_point.starts_at <= now_utc && now_utc < ends_at {
                debug!(
                    "Found active price: {} starting at {}",
                    current_price_point.total, current_price_point.starts_at
                );
                return ActivePrice::new_from_price_point(current_price_point);
            }
        }

        debug!("No active price found");
        ActivePrice::default()
    }

    /// Returns the duration to the next active price.
    /// The duration is guaranteed to be atleast long enough to wait for the next price to be active.
    /// If there is no next active price, it returns None.
    pub fn duration_to_next_active_price(&self) -> Option<Duration> {
        if self.is_empty() {
            return None;
        }

        let now_utc = Utc::now();

        // Find the first price point that starts after now_utc
        for price_point in self.iter() {
            if price_point.starts_at > now_utc {
                return Some(Duration::from_millis(
                    // Add 1ms to round up any fractional milliseconds
                    (price_point.starts_at - now_utc).num_milliseconds() as u64 + 1,
                ));
            }
        }

        None
    }

    /// Returns the duration to the next price list.
    /// If the prices should be fetched immediately, it returns 0.
    pub fn duration_to_new_price_list(&self, update_time: &NaiveTime) -> Duration {
        if !self.has_today_prices() {
            // We don't have today's prices, we can fetch them immediately.
            debug!("Missing today's prices, can fetch immediately");
            return Duration::from_millis(0);
        }

        // Determine some dates and times
        let now_local = Local::now();
        let date_today = now_local.date_naive();
        let date_tomorrow = (now_local + chrono::Duration::days(1)).date_naive();
        let today_update_local = date_today
            .and_time(*update_time)
            .and_local_timezone(Local)
            .unwrap();
        let tomorrow_update_local = date_tomorrow
            .and_time(*update_time)
            .and_local_timezone(Local)
            .unwrap();

        // If we already have tomorrow's prices, we have to wait until
        // the configured update time tomorrow.
        if self.has_tomorrows_prices() {
            let chrono_duration = tomorrow_update_local.signed_duration_since(now_local);
            debug!(
                "Tomorrow's prices are already available, should wait until {} local time tomorrow",
                update_time.format("%H:%M")
            );
            return Duration::from_millis(chrono_duration.num_milliseconds() as u64);
        }

        // At this point we know that we have today's prices, but not tomorrow's.

        // Is it passed the configured update time?
        if now_local > today_update_local {
            // Yes, we should fetch new prices immediately.
            debug!(
                "It's past {} local time today, can fetch immediately",
                update_time.format("%H:%M")
            );
            return Duration::from_millis(0);
        }

        // We have to wait until the configured update time today.
        let chrono_duration = today_update_local.signed_duration_since(now_local);
        debug!(
            "Should wait until {} local time today, duration: {:?}",
            update_time.format("%H:%M"),
            chrono_duration
        );
        Duration::from_millis(chrono_duration.num_milliseconds() as u64)
    }

    pub fn latest_price_date(&self) -> Option<DateTime<Utc>> {
        self.last().map(|p| p.starts_at)
    }

    pub fn has_more_recent_prices(&self, other: &Self) -> bool {
        // Compare dates
        other.latest_price_date() < self.latest_price_date()
    }

    pub fn has_prices_for_date(&self, date: &DateTime<Utc>) -> bool {
        let prices_before_date = self.iter().any(|point| point.starts_at < *date);
        let prices_after_date = self.iter().any(|point| point.starts_at > *date);
        prices_before_date && prices_after_date
    }

    pub fn has_tomorrows_prices(&self) -> bool {
        let tomorrow_local = Local::now() + chrono::Duration::days(1);
        let tomorrow_utc = DateTime::<Utc>::from(tomorrow_local);
        self.has_prices_for_date(&tomorrow_utc)
    }

    pub fn has_today_prices(&self) -> bool {
        let now_utc = Utc::now();
        self.has_prices_for_date(&now_utc)
    }

    /// Writes the price points to a JSON file (atomically).
    pub fn to_file(&self, filepath: &str) -> Result<()> {
        debug!("Writing {} price points to file: {}", self.len(), filepath);
        // Important: the temp file must be on the same mount as the target file,
        // otherwise the rename will not be atomic.
        let temp_path = format!("{}.tmp", filepath);

        // Write to temporary file
        {
            let file = File::create(&temp_path)?;
            serde_json::to_writer_pretty(file, self)?;
        }

        // Atomically rename the temporary file to the target file
        rename(&temp_path, filepath)?;

        info!("Successfully wrote price points to {}", filepath);
        Ok(())
    }

    /// Creates a new PricePoints instance from a JSON file
    /// Returns an empty PricePoints if the file is not found
    pub fn from_file(filepath: &str) -> Result<Self> {
        debug!("Loading price points from file: {}", filepath);
        if !Path::new(filepath).exists() {
            debug!(
                "File {} does not exist, returning empty price points",
                filepath
            );
            return Ok(Self::new());
        }

        let file = File::open(filepath)?;
        let mut loaded_price_points: Vec<PricePoint> = serde_json::from_reader(file)?;
        // Sort price points chronologically by starts_at
        loaded_price_points.sort_by(|a, b| a.starts_at.cmp(&b.starts_at));

        info!(
            "Successfully loaded {} price points from {}",
            loaded_price_points.len(),
            filepath
        );
        Ok(Self(loaded_price_points))
    }

    /// Creates a new PricePoints instance by fetching prices from the Tibber API.
    /// Returns prices in chronological order.
    pub fn fetch_from_tibber(tibber: &TibberClient) -> Result<Self> {
        let price_info = tibber.fetch_price_info()?;
        let mut all_prices = Vec::new();
        // Add today's and tomorrow's prices in chronological order
        all_prices.extend(price_info.today);
        all_prices.extend(price_info.tomorrow);
        // Sort price points chronologically by starts_at
        all_prices.sort_by(|a, b| a.starts_at.cmp(&b.starts_at));

        Ok(Self(all_prices))
    }

    pub fn try_update(
        &mut self,
        client: &TibberClient,
        prices_file: &str,
        update_time: &NaiveTime,
    ) -> Result<bool> {
        if !self.should_fetch_prices(update_time) {
            debug!("Decided not to contact Tibber API at this moment, using existing prices.");
            return Ok(false);
        }

        // Fetch new prices
        debug!("Fetching new prices from Tibber API");
        let new_prices = Self::fetch_from_tibber(client)?;

        // Check if we got any new prices
        if new_prices.is_empty() {
            debug!("No new prices received from Tibber API");
            return Ok(false);
        }

        // Check if the new prices are more recent than the current ones
        if !new_prices.has_more_recent_prices(self) {
            debug!("New prices are not more recent than current ones");
            return Ok(false);
        }

        // Update the prices
        debug!("Updating prices with {} new price points", new_prices.len());
        *self = new_prices;

        // Save the prices to file
        info!("Saving updated prices to file");
        self.to_file(prices_file)?;

        info!("Prices successfully updated");
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Timelike, Utc};

    #[test]
    fn test_price_points_new() {
        let price_points = PricePoints::new();
        assert!(price_points.is_empty());
    }

    #[test]
    fn test_price_points_has_today_prices() {
        let now = Utc::now();

        // Add a price point for yesterday
        let yesterday = now - Duration::days(1);
        let yesterday_price = PricePoint {
            total: 1.0,
            starts_at: yesterday,
        };

        // Add a price point for tomorrow
        let tomorrow = now + Duration::days(1);
        let tomorrow_price = PricePoint {
            total: 2.0,
            starts_at: tomorrow,
        };

        // Create a new PricePoints with these prices
        let prices = vec![yesterday_price, tomorrow_price];
        let price_points = PricePoints::from_prices(prices);

        assert!(price_points.has_today_prices());
    }

    #[test]
    fn test_price_points_has_tomorrows_prices() {
        let now = Utc::now();

        // Add a price point for today
        let today = now;
        let today_price = PricePoint {
            total: 1.0,
            starts_at: today,
        };

        // Add a price point for day after tomorrow
        let day_after_tomorrow = now + Duration::days(2);
        let day_after_tomorrow_price = PricePoint {
            total: 2.0,
            starts_at: day_after_tomorrow,
        };

        // Create a new PricePoints with these prices
        let prices = vec![today_price, day_after_tomorrow_price];
        let price_points = PricePoints::from_prices(prices);

        assert!(price_points.has_tomorrows_prices());
    }

    #[test]
    fn test_price_points_get_current_price() {
        let now = Utc::now();

        // Add a price point for current hour
        let current_price = PricePoint {
            total: 1.0,
            starts_at: now,
        };

        // Add a price point for next hour
        let next_hour = now + Duration::hours(1);
        let next_price = PricePoint {
            total: 2.0,
            starts_at: next_hour,
        };

        // Create a new PricePoints with these prices
        let prices = vec![current_price.clone(), next_price];
        let price_points = PricePoints::from_prices(prices);

        let current = price_points.get_active_price();
        assert_eq!(current.price, Some(current_price.total));
        assert_eq!(
            current.starts_at,
            Some(current_price.starts_at.with_timezone(&Local))
        );
    }

    #[test]
    fn test_parse_update_time_valid() {
        let time = PricePoints::parse_update_time("13:00").unwrap();
        assert_eq!(time.hour(), 13);
        assert_eq!(time.minute(), 0);
    }

    #[test]
    fn test_parse_update_time_invalid_format() {
        let result = PricePoints::parse_update_time("13");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_update_time_invalid_hour() {
        let result = PricePoints::parse_update_time("25:00");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_update_time_invalid_minute() {
        let result = PricePoints::parse_update_time("13:60");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_update_time_empty() {
        let time = PricePoints::parse_update_time("").unwrap();
        assert_eq!(time.hour(), PricePoints::DEFAULT_UPDATE_HOUR);
        assert_eq!(time.minute(), PricePoints::DEFAULT_UPDATE_MINUTE);
    }
}
