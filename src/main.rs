use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use log::{LevelFilter, debug, error, info};
use pricing::{OutputFormat, PricePoints};
use serde_json::json;
use std::time::Duration;
use std::{env, sync::Arc};
use tibberapi::TibberClient;

pub mod pricing;
pub mod shared_buffer;
pub mod tibberapi;
pub mod utils;

#[derive(Parser)]
#[command(
    name = "tibprice",
    version,
    about = "Get the active energy price from Tibber. (Use --help for full details)",
    long_about = r#"Tibber price tool provides real-time energy pricing from Tibber's API.

Configuration options:
- Environment variables can be set in a .env file in the current directory
- Arguments can be loaded from files using @filename syntax, where each line in the file is treated as a separate argument

Examples:
  tibprice --token YOUR_API_TOKEN price
  tibprice @config.args price

The tool is very fast because it caches prices locally and only connects to Tibber when necessary based on the configured update time."#
)]
struct Cli {
    /// Tibber API access token
    #[arg(short, long, env = "TIBBER_TOKEN", hide_env_values = true)]
    token: String,

    /// Optional ID of the home to fetch prices for
    #[arg(short = 'i', long, env = "TIBBER_HOME_ID")]
    home_id: Option<String>,

    /// Path used to store the price data fetched from Tibber.
    #[arg(short, long, default_value = "prices.json")]
    prices_file: String,

    /// Maximum number of retries for Tibber API requests
    #[arg(short = 'r', long, default_value = "3")]
    max_retries: u32,

    /// Initial delay for Tibber API requests (in seconds)
    #[arg(short = 'd', long, default_value = "1")]
    initial_delay: u64,

    /// Maximum delay for Tibber API requests (in seconds)
    #[arg(short = 'D', long, default_value = "60")]
    max_delay: u64,

    /// Time of day when new prices are expected to be available (24-hour format, HH:MM)
    #[arg(short = 'u', long, default_value = "13:00")]
    price_update_time: String,

    /// Output style of the active price. Use "none" to not display the price.
    #[arg(short, long, default_value = "json")]
    output_format: OutputFormat,

    /// Set the log level.
    #[arg(short, long, default_value = "warn")]
    log_level: CliLevelFilter,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all homes that can be used with the supplied access token.
    Homes,

    /// Output the active price.
    Price,

    /// Run in daemon mode to continuously fetch and output active prices.
    Daemon,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, ValueEnum)]
enum CliLevelFilter {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<CliLevelFilter> for LevelFilter {
    fn from(value: CliLevelFilter) -> Self {
        match value {
            CliLevelFilter::Off => LevelFilter::Off,
            CliLevelFilter::Error => LevelFilter::Error,
            CliLevelFilter::Warn => LevelFilter::Warn,
            CliLevelFilter::Info => LevelFilter::Info,
            CliLevelFilter::Debug => LevelFilter::Debug,
            CliLevelFilter::Trace => LevelFilter::Trace,
        }
    }
}

fn print_homes(client: &TibberClient) {
    debug!("Fetching home IDs from Tibber API");
    let home_ids = client.fetch_home_ids();
    let homes = home_ids
        .into_iter()
        .map(|home| {
            json!({
                "id": home.id.unwrap(),
                "nickname": home.app_nickname.unwrap()
            })
        })
        .collect::<Vec<_>>();

    debug!("Found {} homes", homes.len());
    println!(
        "{}",
        serde_json::to_string_pretty(&homes).expect("Unable to create json")
    );
}

fn print_active_price(cli: &Cli, client: &TibberClient) {
    debug!("Loading cached prices from {}", cli.prices_file);
    let mut cached_prices = match PricePoints::from_file(&cli.prices_file) {
        Ok(prices_from_file) => prices_from_file,
        Err(e) => {
            error!("Error loading price file: {}", e);
            std::process::exit(1);
        }
    };

    // Parse the update time from the command line
    let update_time = match PricePoints::parse_update_time(&cli.price_update_time) {
        Ok(time) => time,
        Err(e) => {
            error!("Error parsing price update time: {}", e);
            std::process::exit(1);
        }
    };

    debug!("Attempting to update prices");
    match cached_prices.try_update(client, &cli.prices_file, &update_time) {
        Ok(_) => {
            let output = cached_prices
                .get_active_price()
                .to_string_pretty(&cli.output_format);
            println!("{}", output);
        }
        Err(e) => {
            error!("Error updating prices: {}", e);
            std::process::exit(1);
        }
    }
}

fn start_daemon(cli: &Cli, client: &TibberClient) {
    info!("Starting daemon mode");

    // Parse the update time from the command line
    let update_time = match PricePoints::parse_update_time(&cli.price_update_time) {
        Ok(time) => {
            info!(
                "Expecting a new price list every day at {}",
                time.format("%H:%M")
            );
            time
        }
        Err(e) => {
            error!("Error parsing price update time: {}", e);
            std::process::exit(1);
        }
    };

    let one_second = 1000;
    let one_minute = 60 * one_second;
    let one_hour = 60 * one_minute;
    let background_client = client.adjusted_clone(9999, one_second, one_hour);

    // Load the initial prices from file
    debug!("Loading cached prices from {}", cli.prices_file);

    let prices_from_file = match PricePoints::from_file(&cli.prices_file) {
        Ok(prices_from_file) => prices_from_file,
        Err(e) => {
            error!("Error loading price file: {}", e);
            std::process::exit(1);
        }
    };

    let price_list_is_empty = prices_from_file.is_empty();

    // Create a shared price data object
    debug!("Creating shared price data object");
    let shared_prices = Arc::new(shared_buffer::SharedPricePoints::new(prices_from_file));

    // Start the background worker with an hourly update interval
    info!("Starting background worker");
    shared_buffer::start_background_worker(
        Arc::clone(&shared_prices),
        background_client,
        cli.prices_file.clone(),
        update_time,
    );

    // Check if we need to wait for the first price to arrive.
    // This ensures we don't show an empty active price while waiting for the first price.
    if price_list_is_empty {
        // Wait up to 60 seconds for the first price to arrive.
        info!("Waiting for first price from background worker");
        while !shared_prices.wait_for_new_prices(Utc::now(), Duration::from_secs(15 * 60)) {
            info!("Still waiting for first price.")
        }
    }

    // Simple loop - check for new prices and display them
    info!("Entering main loop");

    // Get the initial prices from the shared price buffer
    // This might have been updated by the background worker already.
    let mut prices = shared_prices.clone_prices();
    loop {
        let output = prices
            .get_active_price()
            .to_string_pretty(&cli.output_format);
        println!("{}", output);

        let latest_price_date = prices.latest_price_date().unwrap_or(Utc::now());
        let wait_time = prices
            .duration_to_next_active_price()
            .unwrap_or(Duration::from_secs(60));

        info!(
            "Sleeping for {} until next active price",
            utils::format_std_duration(wait_time)
        );
        // Wait for new prices, or timeout after 60 seconds
        if shared_prices.wait_for_new_prices(latest_price_date, wait_time) {
            // Update with new prices
            debug!("New prices available, updating");
            prices = shared_prices.clone_prices();
        }
    }
}

fn main() -> Result<()> {
    // Load .env file from current directory
    dotenvy::from_filename(".env").ok();

    // Expand arguments from files
    let args = argfile::expand_args_from(env::args_os(), argfile::parse_fromfile, argfile::PREFIX)
        .expect("Failed to expand arguments");

    // Parse command line
    let cli = Cli::parse_from(args);

    // Initialize the logger with appropriate verbosity
    env_logger::Builder::new()
        .filter_level(cli.log_level.into())
        .init();

    info!("Starting Tibber price tool");

    let tibber_client = TibberClient::try_new(
        Some(&cli.token),
        cli.home_id.as_deref(),
        cli.max_retries,
        cli.initial_delay * 1000,
        cli.max_delay * 1000,
    )?;

    match cli.command {
        Commands::Price => {
            debug!("Executing Price command");
            print_active_price(&cli, &tibber_client)
        }
        Commands::Homes => {
            debug!("Executing Homes command");
            print_homes(&tibber_client)
        }
        Commands::Daemon => {
            debug!("Executing Daemon command");
            start_daemon(&cli, &tibber_client)
        }
    }

    info!("Tibber price tool completed");
    Ok(())
}

#[test]
fn verify_cli() {
    use clap::CommandFactory;
    Cli::command().debug_assert();
}
