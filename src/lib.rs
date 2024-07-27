#[macro_use]
extern crate lazy_static;

#[macro_use]
mod utils;

use clap::Parser;
use std::collections::HashMap;
use std::io::{BufRead, BufReader}; // Add this import
use std::env;
#[cfg(target_os = "windows")]
use winapi::shared::ifmib::MIB_IFTABLE;
#[cfg(target_os = "windows")]
use winapi::um::iphlpapi::GetIfTable;
#[cfg(target_os = "windows")]
use std::mem;
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "linux")]
use std::fs::File;

#[cfg(target_os = "windows")]
use winapi::shared::ifmib::MIB_IFTABLE;
#[cfg(target_os = "windows")]
use winapi::um::iphlpapi::GetIfTable;
#[cfg(target_os = "windows")]
use std::mem;

const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");
const LICENSE: &str = env!("CARGO_PKG_LICENSE");

#[derive(Parser)]
#[clap(version = VERSION, author = AUTHOR, long_version = LONG_VERSION.as_str())]
pub struct Opts {
    /// Interfaces to monitor, separated by commas (e.g., "eth0,lo")
    #[clap(short, long)]
    pub interfaces: Option<String>,

    /// Enables monitoring of all interfaces found for which statistics are available.
    #[clap(short = 'a')]
    pub monitor_all: bool,

    /// Enables monitoring of loopback interfaces for which statistics are available.
    #[clap(short = 'l')]
    pub monitor_loopback: bool,

    /// Hides interfaces with zero counters.
    #[clap(short = 'z')]
    pub hide_zero_counters: bool,

    /// Delay between updates in seconds (default is 1 second)
    #[clap(default_value = "1")]
    pub delay: f64,

    /// Delay before the first measurement in seconds (default is same as --delay)
    #[clap(long)]
    pub first_measurement: Option<f64>,

    /// Number of updates before stopping (default is unlimited)
    pub count: Option<u64>,
}

lazy_static! {
    static ref LONG_VERSION: String = {
        let commit_hash = option_env!("VERGEN_GIT_SHA").unwrap_or("unknown");
        let build_timestamp = option_env!("VERGEN_BUILD_TIMESTAMP").unwrap_or("unknown");
        let rust_version = option_env!("VERGEN_RUSTC_SEMVER").unwrap_or("unknown");
        let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());

        format!(
            "A tool to report network interface statistics.\n\n\
            Author: {}\n\
            License: {}\n\
            Build info:\n\
            Commit: {}\n\
            Build Timestamp: {}\n\
            Rust Version: {}\n\
            Compilation Target: {}\n\
            Repo: {}",
            AUTHOR, LICENSE, commit_hash, build_timestamp, rust_version, target, REPO_URL
        )
    };
}

fn filter_zero_counters(
    stats: &HashMap<String, (u64, u64)>,
    interfaces: &[String],
) -> Vec<String> {
    interfaces
        .iter()
        .filter(|iface| {
            if let Some(&(rx, tx)) = stats.get(*iface) {
                rx != 0 || tx != 0
            } else {
                false
            }
        })
        .cloned()
        .collect()
}

#[cfg(target_os = "linux")]
pub fn get_net_dev_stats() -> Result<HashMap<String, (u64, u64)>, std::io::Error> {
    let file = File::open("/proc/net/dev")?;
    let reader = BufReader::new(file);
    parse_net_dev_stats(reader)
}

#[cfg(target_os = "macos")]
pub fn get_net_dev_stats() -> Result<HashMap<String, (u64, u64)>, std::io::Error> {
    let output = Command::new("netstat")
        .arg("-b")
        .output()
        .expect("Failed to execute netstat command");
    let reader = BufReader::new(output.stdout.as_slice());
    parse_net_dev_stats(reader)
}

#[cfg(target_os = "windows")]
pub fn get_net_dev_stats() -> Result<HashMap<String, (u64, u64)>, std::io::Error> {
    let mut table: MIB_IFTABLE = unsafe { mem::zeroed() };
    let mut size = mem::size_of_val(&table) as u32;

    unsafe {
        if GetIfTable(&mut table, &mut size, 0) == 0 {
            let mut stats = HashMap::new();
            for i in 0..table.dwNumEntries {
                let row = table.table[i as usize];
                let name = format!("Interface {}", i);
                stats.insert(name, (row.dwInOctets as u64, row.dwOutOctets as u64));
            }
            Ok(stats)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to retrieve network interface table",
            ))
        }
    }
}

pub fn parse_net_dev_stats<R: BufRead>(reader: R) -> Result<HashMap<String, (u64, u64)>, std::io::Error> {
    let mut stats = HashMap::new();
    let lines: Vec<_> = reader.lines().collect::<Result<_, _>>()?;
    test_debug!("Parsing {} lines", lines.len());

    for (_index, line) in lines.into_iter().enumerate().skip(2) {
        test_debug!("Parsing line: {}", line);
        if let Some((iface, rest)) = line.split_once(':') {
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if fields.len() >= 9 {
                let rx_bytes: u64 = fields[0].parse().map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid RX bytes"))?;
                let tx_bytes: u64 = fields[8].parse().map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid TX bytes"))?;
                stats.insert(iface.trim().to_string(), (rx_bytes, tx_bytes));
            } else {
                test_debug!("Invalid line format: '{}' ({} fields: {:?})", line, fields.len(), fields);
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Invalid line format: {} fields", fields.len())));
            }
        } else {
            test_debug!("Invalid line format: '{}' (no colon found)", line);
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid line format (no colon found)"));
        }
    }
    Ok(stats)
}

pub fn print_headers(
    interfaces: &[String],
    writer: &mut dyn std::io::Write,
    hide_zero_counters: bool,
    stats: &HashMap<String, (u64, u64)>,
) -> std::io::Result<()> {
    let interfaces = if hide_zero_counters {
        filter_zero_counters(stats, interfaces)
    } else {
        interfaces.to_vec()
    };

    if interfaces.is_empty() {
        return Ok(());
    }

    let width = 18; // width for each interface field including space for in/out
    for (i, interface) in interfaces.iter().enumerate() {
        let padded_name = format!("{:^width$}", interface, width = width);
        write!(writer, "{}", padded_name)?;
        if i < interfaces.len() - 1 {
            write!(writer, "  ")?; // additional spaces between columns
        }
    }
    writeln!(writer)?;

    for (i, _) in interfaces.iter().enumerate() {
        write!(writer, "{:>8}  {:>8}", "KB/s in", "KB/s out")?;
        if i < interfaces.len() - 1 {
            write!(writer, "  ")?; // additional spaces between columns
        }
    }
    writeln!(writer)?;

    Ok(())
}

pub fn print_stats(
    previous: &HashMap<String, (u64, u64)>,
    current: &HashMap<String, (u64, u64)>,
    interfaces: &[String],
    writer: &mut dyn std::io::Write,
    hide_zero_counters: bool,
) -> std::io::Result<()> {
    let interfaces = if hide_zero_counters {
        filter_zero_counters(current, interfaces)
    } else {
        interfaces.to_vec()
    };

    for (i, interface) in interfaces.iter().enumerate() {
        if let (Some(&(prev_rx, prev_tx)), Some(&(cur_rx, cur_tx))) =
            (previous.get(interface), current.get(interface))
        {
            let rx_kbps = (cur_rx.saturating_sub(prev_rx)) as f64 / 1024.0;
            let tx_kbps = (cur_tx.saturating_sub(prev_tx)) as f64 / 1024.0;
            write!(writer, "{:>8.2}  {:>8.2}", rx_kbps, tx_kbps)?;
            if i < interfaces.len() - 1 {
                write!(writer, "  ")?; // additional spaces between columns
            }
        }
    }
    writeln!(writer)?;

    Ok(())
}
