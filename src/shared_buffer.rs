use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::{DateTime, Utc};
use log::{debug, error, info, trace};
use rand::Rng;

use crate::pricing::PricePoints;
use crate::tibberapi::TibberClient;
use crate::utils;

/// Represents the shared state between the background worker and the main thread
pub struct SharedPricePoints {
    /// The current price points data
    price_points: Mutex<PricePoints>,
    /// Condition variable to signal when new prices are available
    has_new_prices_flag: Condvar,
}

impl SharedPricePoints {
    /// Creates a new `PriceData` instance with the given initial price points
    pub fn new(initial_prices: PricePoints) -> Self {
        debug!("Creating new SharedPricePoints with initial price data");
        Self {
            price_points: Mutex::new(initial_prices),
            has_new_prices_flag: Condvar::new(),
        }
    }

    /// Gets a copy of the current price points
    pub fn clone_prices(&self) -> PricePoints {
        trace!("Copying current prices from shared buffer");
        let prices = self
            .price_points
            .lock()
            .expect("Failed to acquire price_points lock");
        prices.clone()
    }

    /// Waits for new prices to become available, with a timeout.
    /// Returns true if new prices are available, false if the timeout was reached.
    /// Returns an error if the lock cannot be acquired.
    pub fn wait_for_new_prices(&self, after: DateTime<Utc>, timeout: Duration) -> bool {
        // Acquire the lock on price_points - this is required before we can wait on the condition variable
        let guard = self
            .price_points
            .lock()
            .expect("Failed to acquire price_points lock");

        // Check if the current price points are newer than the given timestamp
        if guard.latest_price_date() > Some(after) {
            debug!("Shared buffer has new prices");
            return true;
        }
        debug!(
            "Waiting for new price lists (more recent than {}) or proceed after {}",
            after,
            utils::format_std_duration(timeout)
        );
        // Wait on the condition variable with the specified timeout.
        // - The lock guard is passed to wait_timeout and will be unlocked while waiting
        // - Other threads can notify this thread via has_new_prices.notify_all()
        // - When we're notified or the timeout expires, the lock is reacquired automatically
        // - wait_timeout returns the guard and a timeout status
        let (guard, timeout_result) = self
            .has_new_prices_flag
            .wait_timeout(guard, timeout)
            .expect("Failed waiting on condition variable");

        if timeout_result.timed_out() {
            debug!(
                "Wait timed out after {}",
                utils::format_std_duration(timeout)
            );
        } else {
            debug!("Received notification about potential new prices");
        }

        // Check if the current price points are newer than the given timestamp
        let has_new = guard.latest_price_date() > Some(after);
        if has_new {
            debug!("Confirmed new prices are available");
        } else {
            debug!("No new prices available after wait");
        }
        has_new
    }

    /// Updates the price points data and notifies waiting threads if new prices are available
    fn set_new_prices(&self, new_prices: PricePoints) -> bool {
        debug!("Attempting to update price points");
        let mut guard = self
            .price_points
            .lock()
            .expect("Failed to acquire price_points lock");

        // Check if the new prices are more recent than the current ones
        if new_prices.has_more_recent_prices(&guard) {
            debug!("New prices are more recent, updating and notifying waiting threads");
            *guard = new_prices;
            // Notify all waiting threads that new prices are available
            self.has_new_prices_flag.notify_all();
            true
        } else {
            debug!("New prices are not more recent, no update needed");
            false
        }
    }
}

/// Starts a background worker that periodically updates price data
pub fn start_background_worker(
    shared_data: Arc<SharedPricePoints>,
    client: TibberClient,
    prices_file: String,
    update_time: chrono::NaiveTime,
) -> JoinHandle<()> {
    thread::spawn(move || {
        info!("Background worker thread started");
        let mut price_list = shared_data.clone_prices();

        // Get current prices from the shared data
        loop {
            debug!("Background worker attempting to update prices");
            // Update prices using the cache_updater function
            match price_list.try_update(&client, &prices_file, &update_time) {
                Ok(false) => {
                    debug!("No new prices updated");
                    // No new prices, no error. Continue.
                }
                Ok(true) => {
                    info!("New prices received");
                    // Update the shared data if prices are newer
                    shared_data.set_new_prices(price_list.clone());
                }
                Err(e) => {
                    error!("Error updating price cache: {}", e);
                    // Prices might be updated anyway
                    // because the error was related to the file system.
                    shared_data.set_new_prices(price_list.clone());

                    debug!("Sleeping for 60 seconds to avoid spamming the API");
                    // Sleep for 60 seconds to avoid spamming the API
                    thread::sleep(Duration::from_secs(60));
                }
            };

            let wait_time_new_list = price_list
                .duration_to_new_price_list(&update_time)
                .unwrap_or(Duration::from_secs(0));

            // Add random jitter to the wait time. Between 0 and 60 seconds.
            let jitter_millis = rand::rng().random_range(0..=60000);
            let wait_time_with_jitter = wait_time_new_list + Duration::from_millis(jitter_millis);

            info!(
                "Background worker sleeping for {} until next price list (jitter: {} milliseconds)",
                utils::format_std_duration(wait_time_with_jitter),
                jitter_millis
            );
            thread::sleep(wait_time_with_jitter);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tibberapi::PricePoint;
    use chrono::{Duration as ChronoDuration, Utc};

    // Helper function to create test price points
    fn create_test_prices(hours_offset: i64) -> PricePoints {
        let now = Utc::now() + ChronoDuration::hours(hours_offset);
        let price_point = PricePoint {
            total: 1.0,
            starts_at: now,
        };
        let prices = vec![price_point];

        #[allow(deprecated)]
        PricePoints::from_prices(prices)
    }

    #[test]
    fn test_price_data_update() {
        // Create initial prices
        let initial_prices = create_test_prices(0);

        // Create price data with initial prices
        let price_data = SharedPricePoints::new(initial_prices);

        // Create newer prices
        let newer_prices = create_test_prices(1);

        // Update prices
        let updated = price_data.set_new_prices(newer_prices.clone());
        assert!(updated, "Prices should have been updated");

        // Get updated prices
        let current_prices = price_data.clone_prices();
        assert_eq!(
            current_prices.last().unwrap().starts_at,
            newer_prices.last().unwrap().starts_at,
            "Price timestamps should match"
        );
    }

    #[test]
    fn test_price_data_no_update_with_older_prices() {
        // Create initial prices (newer)
        let initial_prices = create_test_prices(1);

        // Create price data with initial prices
        let price_data = SharedPricePoints::new(initial_prices.clone());

        // Create older prices
        let older_prices = create_test_prices(0);

        // Try to update with older prices
        let updated = price_data.set_new_prices(older_prices);
        assert!(!updated, "Prices should not have been updated");

        // Get current prices
        let current_prices = price_data.clone_prices();
        assert_eq!(
            current_prices.last().unwrap().starts_at,
            initial_prices.last().unwrap().starts_at,
            "Price timestamps should still match initial prices"
        );
    }
}
