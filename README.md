# Tibber Price Tool
[![ci](https://github.com/standardcurrent/tibprice/actions/workflows/ci.yml/badge.svg)](https://github.com/standardcurrent/tibprice/actions/workflows/ci.yml)

Easily access your Tibber electricity prices directly from your command line! The Tibber Price Tool provides a quick and efficient way to check current and upcoming energy costs. It smartly caches data to give you instant results while minimizing calls to the Tibber API.

## Disclaimer

This tool is an independent project and is not affiliated with, created by, or endorsed by Tibber. It is a third-party application that utilizes the Tibber API to access electricity price data. All Tibber trademarks, logos, and brand identifiers are the property of Tibber.

## Usage

### Features

- Fetch current and future electricity prices from Tibber
- List available homes in your Tibber account
- Get current price information in multiple formats (JSON, plain text, CSV)
- Automatic caching of price data to avoid unnecessary API calls
- Smart price downloading that respects Tibber's price update schedule
- Daemon mode to continuously monitor and output electricity prices

### Prerequisites

A Tibber API access token is required. You can get your token by loging in with your Tibber account at:
https://developer.tibber.com/settings/access-token


### Installation

1. Download the latest release from the [GitHub Releases page](https://github.com/standardcurrent/tibprice/releases).
    *   **Windows**: Look for a file named similar to `tibprice-...-x86_64-pc-windows-msvc.zip`
    *   **macOS (Apple Silicon, M1/M2/M3/M4)**: Look for `tibprice-...-aarch64-apple-darwin.tar.gz`
    *   **macOS (Intel)**: Look for `tibprice-...-x86_64-apple-darwin.tar.gz`
    *   **Linux (most common desktops/servers)**: Look for `tibprice-...-x86_64-unknown-linux-gnu.tar.gz`
    *   **CerboGX (and similar ARMv7 devices)**: Look for `tibprice-...-armv7-unknown-linux-gnueabihf.tar.gz`
    *   For other systems, choose the file that best matches your architecture.
2. Extract the zip file (for Windows) or tar.gz file (for macOS/Linux) to a location of your choice.
3. Run the executable from the command line.

### Configuration

The tool requires a Tibber API token. You can provide it in three ways:

1. Command-line argument:
```bash
tibprice --token your-token-here [...]
```

2. Environment variable:
```bash
export TIBBER_TOKEN="your-token-here"
```

3. A ".env" file in the current folder that contains the relevant environment variables:
```bash
TIBBER_TOKEN="your-token-here"
```

Optionally, you can specify a home ID. This is only relevant if you have multiple homes associated with your Tibber account. This can be supplied on the command-line or as an environment variable (TIBBER_HOME_ID). The environment variable can also be in the ".env" file.

```bash
tibprice --token your-token-here --home-id your-home-id-here [...]
```

### Commands

#### List Homes

List all home IDs in your Tibber account:
```bash
tibprice --token YOUR_TOKEN homes
```

#### Get Current Price

Get the current electricity price in different formats. This command will fetch prices from Tibber if necessary based on the connection mode:

JSON format (default):
```bash
tibprice --token YOUR_TOKEN price
```

Plain text format:
```bash
tibprice --token YOUR_TOKEN price --output-format plain
```

CSV format:
```bash
tibprice --token YOUR_TOKEN price --output-format csv
```

Force download even if prices are already cached:
```bash
tibprice --token YOUR_TOKEN price --connect-mode always
```

#### Daemon Mode

Run in daemon mode to continuously fetch and output active prices:
```bash
tibprice --token YOUR_TOKEN daemon
```

### Command-line Options

- `--token`, `-t`: Tibber API access token (required)
- `--home-id`, `-i`: Optional ID of the home to fetch prices for
- `--prices-file`, `-p`: Path to save the price data (default: prices.json)
- `--max-retries`, `-r`: Maximum number of retries for Tibber API requests (default: 3)
- `--initial-delay`, `-d`: Initial delay for Tibber API requests in seconds (default: 1)
- `--max-delay`, `-D`: Maximum delay for Tibber API requests in seconds (default: 60)
- `--price-update-time`, `-u`: Time of day when new prices are expected to be available (24-hour format, HH:MM) (default: 13:00)
- `--connect-mode`, `-c`: Connection mode for the Tibber API. Options: `auto` (only connect if new prices are expected), `never`, `always` (default: auto)
- `--output-format`, `-o`: Output style of the active price. Options: `json`, `jsonpretty`, `plain`, `csv`, `none` (default: json)
- `--log-level`, `-l`: Set the log level. Options: `off`, `error`, `warn`, `info`, `debug`, `trace` (default: warn)

### License

This project is licensed under the terms of the included LICENSE file.

## For Developers

### Prerequisites

- Rust 2024 edition or later

### Building from Source

1. Clone the repository:
```bash
git clone https://github.com/standardcurrent/tibprice.git
cd tibprice
```

2. Build the project:
```bash
cargo build
```

### Architecture

The Tibber Price Tool is a command-line application built in Rust. Its architecture revolves around the following key components:

*   **Command-Line Interface (CLI)**: Powered by the `clap` crate, it parses user input, arguments, and subcommands (`homes`, `price`, `daemon`).
*   **Tibber API Client (`TibberClient`)**: This module is responsible for all interactions with the Tibber API. It handles API token authentication, constructs GraphQL queries, and retrieves data such as home information and electricity prices.
*   **Price Data Management (`PricePoints`)**: This component manages the electricity price information. It includes logic for:
    *   Fetching new price data from the Tibber API via `TibberClient`.
    *   Caching price data locally (typically in `prices.json`) to minimize API calls.
    *   Determining when new data should be fetched based on Tibber's price update schedule.
    *   Providing the current active price based on the cached data.
*   **Command Handlers**: Dedicated functions orchestrate the actions for each subcommand, utilizing the `TibberClient` and `PricePoints` components as needed.
*   **Daemon Mode**: A specialized component that enables the tool to run continuously in the background, periodically updating and providing price information.
*   **Configuration**: The tool reads configuration like the API token and home ID from command-line arguments, environment variables, or a `.env` file.

The typical flow involves parsing the command, fetching or loading price data (respecting the cache and connection mode), and then outputting the requested information in the specified format.

### Contributing

Contributions are welcome! Please feel free to submit a Pull Request.