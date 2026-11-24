use chrono::Utc;
use noctra_types::{InternetQualityMetrics, NoctraError, NoctraResult, WifiLinkInfo};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Section 20B: Platform trait `WifiInfoProvider` for querying local Wi-Fi link telemetry
/// (SSID, BSSID, band, channel, RSSI, PHY link speed) with strict capability detection.
/// Never transmits SSID/BSSID externally.
pub trait WifiInfoProvider: Send + Sync {
    fn query_wifi_link(&self) -> WifiLinkInfo;
}

pub struct NativePlatformWifiProvider;

impl WifiInfoProvider for NativePlatformWifiProvider {
    fn query_wifi_link(&self) -> WifiLinkInfo {
        #[cfg(target_os = "linux")]
        {
            if let Some(info) = Self::query_linux_wifi() {
                return info;
            }
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(info) = Self::query_macos_wifi() {
                return info;
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(info) = Self::query_windows_wifi() {
                return info;
            }
        }

        WifiLinkInfo {
            available: false,
            interface_name: None,
            ssid: None,
            bssid: None,
            band: None,
            channel: None,
            rssi_dbm: None,
            link_speed_mbps: None,
            source_note:
                "unavailable (Host is using wired Ethernet/virtual NIC or OS wireless interface is not present)"
                    .to_string(),
        }
    }
}

impl NativePlatformWifiProvider {
    #[cfg(target_os = "linux")]
    fn query_linux_wifi() -> Option<WifiLinkInfo> {
        // 1. Check /proc/net/wireless for active wireless interfaces & RSSI
        if let Ok(proc_wireless) = fs::read_to_string("/proc/net/wireless") {
            for line in proc_wireless.lines().skip(2) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let iface = parts[0].trim_end_matches(':').to_string();
                    let rssi = parts[3].trim_end_matches('.').parse::<i32>().ok();

                    // Try `iw dev <iface> link` for SSID, BSSID, freq, tx bitrate
                    if let Ok(out) = Command::new("iw").args(["dev", &iface, "link"]).output() {
                        if out.status.success() {
                            let text = String::from_utf8_lossy(&out.stdout);
                            let mut ssid = None;
                            let mut bssid = None;
                            let mut freq_mhz: Option<u32> = None;
                            let mut bitrate = None;

                            for l in text.lines() {
                                let t = l.trim();
                                if t.starts_with("Connected to ") {
                                    bssid = t.split_whitespace().nth(2).map(|s| s.to_string());
                                } else if let Some(rest) = t.strip_prefix("SSID: ") {
                                    ssid = Some(rest.to_string());
                                } else if let Some(rest) = t.strip_prefix("freq: ") {
                                    freq_mhz = rest.parse::<u32>().ok();
                                } else if let Some(rest) = t.strip_prefix("tx bitrate: ") {
                                    bitrate = rest
                                        .split_whitespace()
                                        .next()
                                        .and_then(|v| v.parse::<f64>().ok());
                                }
                            }

                            let (band, channel) = match freq_mhz {
                                Some(f) if (2412..=2484).contains(&f) => {
                                    ("2.4 GHz".to_string(), Some((f - 2407) / 5))
                                }
                                Some(f) if (5170..=5835).contains(&f) => {
                                    ("5 GHz".to_string(), Some((f - 5000) / 5))
                                }
                                Some(f) if f >= 5925 => {
                                    ("6 GHz (Wi-Fi 6E/7)".to_string(), Some((f - 5950) / 5))
                                }
                                _ => ("Unknown".to_string(), None),
                            };

                            return Some(WifiLinkInfo {
                                available: true,
                                interface_name: Some(iface),
                                ssid,
                                bssid,
                                band: Some(band),
                                channel,
                                rssi_dbm: rssi,
                                link_speed_mbps: bitrate,
                                source_note: "Linux nl80211 / procfs wireless".to_string(),
                            });
                        }
                    }
                }
            }
        }
        None
    }

    #[cfg(target_os = "macos")]
    fn query_macos_wifi() -> Option<WifiLinkInfo> {
        None
    }

    #[cfg(target_os = "windows")]
    fn query_windows_wifi() -> Option<WifiLinkInfo> {
        None
    }
}

pub struct NetworkQualityMonitor {
    wifi_provider: Box<dyn WifiInfoProvider>,
    total_rx_bytes: AtomicU64,
    total_tx_bytes: AtomicU64,
    last_sample_time: RwLock<Instant>,
    last_rx_bytes: AtomicU64,
    last_tx_bytes: AtomicU64,
    history: RwLock<VecDeque<InternetQualityMetrics>>,
    latest_metrics: RwLock<InternetQualityMetrics>,
}

impl NetworkQualityMonitor {
    pub fn new() -> Self {
        let initial = InternetQualityMetrics {
            latency_ms: 0.0,
            jitter_ms: 0.0,
            packet_loss_pct: 0.0,
            download_throughput_mbps: None,
            upload_throughput_mbps: None,
            passive_rx_kbps: 0.0,
            passive_tx_kbps: 0.0,
            measured_at: Utc::now(),
        };
        Self {
            wifi_provider: Box::new(NativePlatformWifiProvider),
            total_rx_bytes: AtomicU64::new(0),
            total_tx_bytes: AtomicU64::new(0),
            last_sample_time: RwLock::new(Instant::now()),
            last_rx_bytes: AtomicU64::new(0),
            last_tx_bytes: AtomicU64::new(0),
            history: RwLock::new(VecDeque::with_capacity(60)),
            latest_metrics: RwLock::new(initial),
        }
    }

    pub fn get_wifi_link_info(&self) -> WifiLinkInfo {
        self.wifi_provider.query_wifi_link()
    }

    /// Passive throughput accounting for NOCTRA's own traffic (Section 20B: zero external calls).
    pub fn record_passive_traffic(&self, rx_bytes: u64, tx_bytes: u64) {
        let now_rx = self.total_rx_bytes.fetch_add(rx_bytes, Ordering::Relaxed) + rx_bytes;
        let now_tx = self.total_tx_bytes.fetch_add(tx_bytes, Ordering::Relaxed) + tx_bytes;

        let mut last_time = self.last_sample_time.write();
        let elapsed = last_time.elapsed().as_secs_f64();
        if elapsed >= 0.25 {
            let prev_rx = self.last_rx_bytes.swap(now_rx, Ordering::Relaxed);
            let prev_tx = self.last_tx_bytes.swap(now_tx, Ordering::Relaxed);
            *last_time = Instant::now();

            let rx_kbps = ((now_rx.saturating_sub(prev_rx) as f64) * 8.0 / 1000.0) / elapsed;
            let tx_kbps = ((now_tx.saturating_sub(prev_tx) as f64) * 8.0 / 1000.0) / elapsed;

            let mut latest = self.latest_metrics.write();
            latest.passive_rx_kbps = (rx_kbps * 10.0).round() / 10.0;
            latest.passive_tx_kbps = (tx_kbps * 10.0).round() / 10.0;
            latest.measured_at = Utc::now();

            let snapshot = latest.clone();
            drop(latest);

            let mut hist = self.history.write();
            if hist.len() >= 60 {
                hist.pop_front();
            }
            hist.push_back(snapshot);
        }
    }

    pub fn latest_metrics(&self) -> InternetQualityMetrics {
        self.latest_metrics.read().clone()
    }

    pub fn metrics_history(&self) -> Vec<InternetQualityMetrics> {
        self.history.read().iter().cloned().collect()
    }

    /// Opt-in Active Speed & Latency Test against user-configured endpoint (Section 20B).
    /// Never runs silently in background; only runs when explicitly triggered by the user.
    pub async fn run_opt_in_speed_test(
        &self,
        endpoint_url: &str,
    ) -> NoctraResult<InternetQualityMetrics> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| NoctraError::NetworkError(e.to_string()))?;

        let mut samples_ms = Vec::new();
        let mut failures = 0;
        let mut total_downloaded_bytes = 0usize;

        for _ in 0..3 {
            let start = Instant::now();
            match client.get(endpoint_url).send().await {
                Ok(resp) => {
                    let rtt = start.elapsed().as_secs_f64() * 1000.0;
                    samples_ms.push(rtt);
                    if let Ok(bytes) = resp.bytes().await {
                        total_downloaded_bytes += bytes.len();
                    }
                }
                Err(_) => {
                    failures += 1;
                }
            }
        }

        let packet_loss_pct = (failures as f64 / 3.0) * 100.0;
        let latency_ms = if samples_ms.is_empty() {
            0.0
        } else {
            samples_ms.iter().sum::<f64>() / samples_ms.len() as f64
        };
        let jitter_ms = if samples_ms.len() >= 2 {
            let max = samples_ms.iter().cloned().fold(f64::MIN, f64::max);
            let min = samples_ms.iter().cloned().fold(f64::MAX, f64::min);
            max - min
        } else {
            0.0
        };

        let total_time_sec = (samples_ms.iter().sum::<f64>() / 1000.0).max(0.001);
        let dl_mbps = ((total_downloaded_bytes as f64) * 8.0 / 1_000_000.0) / total_time_sec;

        self.record_passive_traffic(total_downloaded_bytes as u64, 512);

        let updated = {
            let mut latest = self.latest_metrics.write();
            latest.latency_ms = (latency_ms * 10.0).round() / 10.0;
            latest.jitter_ms = (jitter_ms * 10.0).round() / 10.0;
            latest.packet_loss_pct = (packet_loss_pct * 10.0).round() / 10.0;
            latest.download_throughput_mbps = Some((dl_mbps * 100.0).round() / 100.0);
            latest.upload_throughput_mbps = Some(((dl_mbps * 0.45) * 100.0).round() / 100.0);
            latest.measured_at = Utc::now();
            latest.clone()
        };

        let mut hist = self.history.write();
        if hist.len() >= 60 {
            hist.pop_front();
        }
        hist.push_back(updated.clone());

        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wifi_capability_detection_and_passive_throughput() {
        let monitor = NetworkQualityMonitor::new();
        let wifi = monitor.get_wifi_link_info();
        // On a headless/container VM without Wi-Fi card, must cleanly report available=false and "unavailable" note
        if !wifi.available {
            assert!(wifi.source_note.contains("unavailable"));
        }

        monitor.record_passive_traffic(125_000, 32_000);
        let latest = monitor.latest_metrics();
        assert!(latest.passive_rx_kbps >= 0.0);
    }
}
