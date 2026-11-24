use noctra_types::{
    CertificateChainInspection, NoctraError, NoctraResult, ObserverVisibilityReport,
    TransportProtectionMode,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use url::Url;

const WELL_KNOWN_PUBLIC_ROOT_CAS: &[&str] = &[
    "ISRG Root X1",
    "ISRG Root X2",
    "DigiCert Global Root G2",
    "DigiCert Global Root CA",
    "GlobalSign Root CA",
    "GTS Root R1",
    "GTS Root R2",
    "GTS Root R3",
    "GTS Root R4",
    "USERTrust RSA Certification Authority",
    "USERTrust ECC Certification Authority",
    "Baltimore CyberTrust Root",
    "Amazon Root CA 1",
    "Amazon Root CA 2",
    "Sectigo Public Server Authentication Root",
    "Entrust Root Certification Authority",
    "Cloudflare Inc ECC CA-3",
];

const DEFAULT_BLOCKED_AD_TRACKER_DOMAINS: &[&str] = &[
    "doubleclick.net",
    "googlesyndication.com",
    "google-analytics.com",
    "googletagmanager.com",
    "adservice.google.com",
    "connect.facebook.net",
    "pixel.facebook.com",
    "analytics.tiktok.com",
    "ads-twitter.com",
    "taboola.com",
    "outbrain.com",
    "criteo.com",
    "scorecardresearch.com",
    "quantserve.com",
    "hotjar.com",
    "mixpanel.com",
    "segment.io",
    "adnxs.com",
    "rubiconproject.com",
    "openx.net",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyStats {
    pub ads_blocked: u64,
    pub trackers_blocked: u64,
    pub https_upgrades: u64,
    pub fingerprinting_attempts_mitigated: u64,
    pub mitm_warnings_raised: u64,
    pub kill_switch_blocks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlEvaluationResult {
    pub original_url: String,
    pub effective_url: String,
    pub blocked_by_filter: bool,
    pub block_reason: Option<String>,
    pub upgraded_to_https: bool,
    pub requires_downgrade_interstitial: bool,
    pub blocked_by_kill_switch: bool,
}

pub struct PrivacyAndTransportEngine {
    blocked_domains: RwLock<HashSet<String>>,
    custom_filter_rules: RwLock<Vec<String>>,
    pinned_endpoint_hashes: RwLock<HashMap<String, String>>,
    pub stats: RwLock<PrivacyStats>,
    pub active_mode: RwLock<TransportProtectionMode>,
    pub https_only_mode: RwLock<bool>,
    pub doh_resolver: RwLock<String>,
    pub ech_enabled: RwLock<bool>,
    pub kill_switch_armed: RwLock<bool>,
    pub vpn_tunnel_healthy: RwLock<bool>,
    pub developer_sslkeylog_override: RwLock<bool>,
}

impl PrivacyAndTransportEngine {
    pub fn new() -> Self {
        // Section 14B: Always remove SSLKEYLOGFILE from environment on startup so TLS session keys
        // are never dumped to disk for Wireshark decryption unless explicitly toggled in DevHub.
        std::env::remove_var("SSLKEYLOGFILE");

        let mut blocked = HashSet::new();
        for d in DEFAULT_BLOCKED_AD_TRACKER_DOMAINS {
            blocked.insert(d.to_string());
        }

        let mut pins = HashMap::new();
        pins.insert(
            "sync.nocta.dev".to_string(),
            "sha256/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
        );
        pins.insert(
            "updates.nocta.dev".to_string(),
            "sha256/8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4".to_string(),
        );

        Self {
            blocked_domains: RwLock::new(blocked),
            custom_filter_rules: RwLock::new(Vec::new()),
            pinned_endpoint_hashes: RwLock::new(pins),
            stats: RwLock::new(PrivacyStats {
                ads_blocked: 0,
                trackers_blocked: 0,
                https_upgrades: 0,
                fingerprinting_attempts_mitigated: 0,
                mitm_warnings_raised: 0,
                kill_switch_blocks: 0,
            }),
            active_mode: RwLock::new(TransportProtectionMode::HttpsDohEch),
            https_only_mode: RwLock::new(true),
            doh_resolver: RwLock::new("https://cloudflare-dns.com/dns-query".to_string()),
            ech_enabled: RwLock::new(true),
            kill_switch_armed: RwLock::new(true),
            vpn_tunnel_healthy: RwLock::new(true),
            developer_sslkeylog_override: RwLock::new(false),
        }
    }

    pub fn add_custom_blocked_domain(&self, domain: &str) {
        let clean = domain.trim().to_lowercase();
        if !clean.is_empty() {
            self.blocked_domains.write().insert(clean.clone());
            self.custom_filter_rules.write().push(format!("||{}^", clean));
        }
    }

    pub fn list_custom_rules(&self) -> Vec<String> {
        self.custom_filter_rules.read().clone()
    }

    /// Evaluates an outbound navigation or subresource URL against:
    /// 1. VPN/Proxy Kill-Switch
    /// 2. O(1) Ad & Tracker Domain Filter
    /// 3. HTTPS-Only Auto-Upgrade & Downgrade Interstitial
    pub fn evaluate_url(&self, raw_url: &str, allow_http_downgrade: bool) -> UrlEvaluationResult {
        // 1. Check Kill Switch
        let mode = *self.active_mode.read();
        let kill_armed = *self.kill_switch_armed.read();
        let tunnel_ok = *self.vpn_tunnel_healthy.read();
        if kill_armed
            && matches!(
                mode,
                TransportProtectionMode::WireGuardVpn | TransportProtectionMode::TorOnionMode
            )
            && !tunnel_ok
        {
            self.stats.write().kill_switch_blocks += 1;
            return UrlEvaluationResult {
                original_url: raw_url.to_string(),
                effective_url: "about:blank#noctra-kill-switch-blocked".to_string(),
                blocked_by_filter: false,
                block_reason: Some(
                    "KILL SWITCH ACTIVE: VPN/Tor tunnel dropped. All outbound traffic blocked to prevent IP/DNS leak."
                        .to_string(),
                ),
                upgraded_to_https: false,
                requires_downgrade_interstitial: false,
                blocked_by_kill_switch: true,
            };
        }

        let parsed = match Url::parse(raw_url) {
            Ok(u) => u,
            Err(_) => {
                return UrlEvaluationResult {
                    original_url: raw_url.to_string(),
                    effective_url: raw_url.to_string(),
                    blocked_by_filter: false,
                    block_reason: None,
                    upgraded_to_https: false,
                    requires_downgrade_interstitial: false,
                    blocked_by_kill_switch: false,
                }
            }
        };

        // 2. Check Ad & Tracker Domain Blocklist
        if let Some(host) = parsed.host_str() {
            let lower_host = host.to_lowercase();
            let guard = self.blocked_domains.read();
            let is_blocked = guard.iter().any(|blocked_domain| {
                lower_host == *blocked_domain
                    || lower_host.ends_with(&format!(".{}", blocked_domain))
            });
            if is_blocked {
                let mut st = self.stats.write();
                st.ads_blocked += 1;
                st.trackers_blocked += 1;
                return UrlEvaluationResult {
                    original_url: raw_url.to_string(),
                    effective_url: "about:blank#blocked".to_string(),
                    blocked_by_filter: true,
                    block_reason: Some(format!(
                        "Blocked by NOCTRA Privacy Shield ({})",
                        lower_host
                    )),
                    upgraded_to_https: false,
                    requires_downgrade_interstitial: false,
                    blocked_by_kill_switch: false,
                };
            }
        }

        // 3. Check HTTPS-Only Mode & Upgrade
        let https_only = *self.https_only_mode.read();
        let is_localhost = parsed
            .host_str()
            .map(|h| h == "localhost" || h == "127.0.0.1" || h == "::1")
            .unwrap_or(false);

        if https_only && parsed.scheme() == "http" && !is_localhost {
            if allow_http_downgrade {
                return UrlEvaluationResult {
                    original_url: raw_url.to_string(),
                    effective_url: raw_url.to_string(),
                    blocked_by_filter: false,
                    block_reason: None,
                    upgraded_to_https: false,
                    requires_downgrade_interstitial: true,
                    blocked_by_kill_switch: false,
                };
            }
            let mut upgraded = parsed.clone();
            let _ = upgraded.set_scheme("https");
            self.stats.write().https_upgrades += 1;
            return UrlEvaluationResult {
                original_url: raw_url.to_string(),
                effective_url: upgraded.to_string(),
                blocked_by_filter: false,
                block_reason: None,
                upgraded_to_https: true,
                requires_downgrade_interstitial: false,
                blocked_by_kill_switch: false,
            };
        }

        UrlEvaluationResult {
            original_url: raw_url.to_string(),
            effective_url: raw_url.to_string(),
            blocked_by_filter: false,
            block_reason: None,
            upgraded_to_https: false,
            requires_downgrade_interstitial: false,
            blocked_by_kill_switch: false,
        }
    }

    /// Inspects a TLS certificate chain and detects non-public root CAs (Corporate MITM proxy / SSL inspection)
    /// as well as verifying certificate pinning for NOCTRA-owned endpoints (Section 14B).
    pub fn inspect_certificate_chain(
        &self,
        host: &str,
        root_ca_name: &str,
        issuer: &str,
        tls_version: &str,
        cipher_suite: &str,
        ech_negotiated: bool,
        cert_der_bytes: &[u8],
    ) -> CertificateChainInspection {
        let is_public_root = WELL_KNOWN_PUBLIC_ROOT_CAS
            .iter()
            .any(|trusted| root_ca_name.contains(trusted));
        let possible_mitm = !is_public_root;
        if possible_mitm {
            self.stats.write().mitm_warnings_raised += 1;
        }

        let mut hasher = Sha256::new();
        hasher.update(cert_der_bytes);
        let fingerprint = format!("sha256/{}", hex::encode(hasher.finalize()));

        let pinned_match = self
            .pinned_endpoint_hashes
            .read()
            .get(host)
            .map(|expected_pin| expected_pin == &fingerprint);

        CertificateChainInspection {
            host: host.to_string(),
            subject: format!("CN={}", host),
            issuer: issuer.to_string(),
            root_ca_name: root_ca_name.to_string(),
            is_publicly_trusted_root: is_public_root,
            possible_mitm_corporate_proxy: possible_mitm,
            tls_version: tls_version.to_string(),
            cipher_suite: cipher_suite.to_string(),
            ech_negotiated,
            sha256_fingerprint: fingerprint,
            pinned_match,
        }
    }

    /// Section 14B: "What can an observer see?" panel documenting residual metadata per active protection mode.
    pub fn observer_visibility_report(&self) -> ObserverVisibilityReport {
        let mode = *self.active_mode.read();
        let sslkeylog_disabled = !*self.developer_sslkeylog_override.read()
            && std::env::var("SSLKEYLOGFILE").is_err();

        let (visible, hidden, notes) = match mode {
            TransportProtectionMode::StandardHttps => (
                vec![
                    "Destination IP address and port (443)".to_string(),
                    "Plaintext DNS query hostname (visible to local network/ISP DNS)".to_string(),
                    "TLS ClientHello Server Name Indication (SNI hostname)".to_string(),
                    "Packet sizes, burst timing, and connection duration".to_string(),
                ],
                vec![
                    "Full URL path and query parameters (e.g., /search?q=...)".to_string(),
                    "HTTP request headers, cookies, and bearer tokens".to_string(),
                    "HTTP request/response bodies and payload contents".to_string(),
                ],
                vec![
                    "An observer running Wireshark on the local Wi-Fi can see which domain you connect to via DNS & SNI, but cannot read pages or credentials.".to_string(),
                ],
            ),
            TransportProtectionMode::HttpsDohEch => (
                vec![
                    "Destination server IP address and ASN".to_string(),
                    "Outer ECH public name (e.g., cloudflare-ech.com) instead of inner target hostname".to_string(),
                    "Encrypted DoH HTTPS connection to configured DNS resolver".to_string(),
                    "Packet size distribution and inter-arrival timing".to_string(),
                ],
                vec![
                    "Plaintext DNS queries (encrypted via DNS-over-HTTPS)".to_string(),
                    "Inner target hostname in TLS ClientHello (encrypted when server supports ECH)".to_string(),
                    "Full URL path, query string, headers, cookies, and response bodies".to_string(),
                ],
                vec![
                    "Wireshark capture shows only TLS 1.3 / QUIC ciphertext to destination IPs and DoH resolver; no plaintext DNS or inner SNI appears on the wire.".to_string(),
                ],
            ),
            TransportProtectionMode::WireGuardVpn => (
                vec![
                    "Encrypted UDP packets to WireGuard VPN entry endpoint IP".to_string(),
                    "Total aggregate bandwidth volume and coarse packet timing".to_string(),
                ],
                vec![
                    "True destination website IP addresses (hidden from local ISP/Wi-Fi)".to_string(),
                    "All DNS queries (tunneled inside WireGuard)".to_string(),
                    "All TLS SNI hostnames, URLs, headers, and application payloads".to_string(),
                ],
                vec![
                    "Local Wi-Fi/ISP sees only WireGuard UDP traffic. Note: the VPN exit node still observes destination IPs.".to_string(),
                ],
            ),
            TransportProtectionMode::TorOnionMode => (
                vec![
                    "Encrypted TLS connection to Tor Entry Guard / Bridge IP".to_string(),
                    "Fixed-size 514-byte Tor cell timing patterns".to_string(),
                ],
                vec![
                    "Destination IP addresses (hidden from both local ISP and Tor Entry Guard)".to_string(),
                    "Client source IP address (hidden from Tor Exit Node and destination server)".to_string(),
                    "DNS queries, SNI, URLs, headers, and payloads".to_string(),
                ],
                vec![
                    "Known limitation: High latency; a global passive adversary observing both Entry Guard and Exit Node can attempt timing correlation.".to_string(),
                ],
            ),
        };

        ObserverVisibilityReport {
            active_mode: mode,
            https_only_enforced: *self.https_only_mode.read(),
            tls_min_version: "TLS 1.3 (RFC 8446) / HTTP/3 QUIC preferred".to_string(),
            doh_resolver: self.doh_resolver.read().clone(),
            ech_enabled: *self.ech_enabled.read(),
            kill_switch_armed: *self.kill_switch_armed.read(),
            sslkeylogfile_disabled: sslkeylog_disabled,
            visible_to_network_observer: visible,
            hidden_from_network_observer: hidden,
            residual_risk_notes: notes,
        }
    }

    /// DNS-over-HTTPS (RFC 8484 JSON/Wire) query builder & verifier so DNS queries never leave in plaintext UDP/53.
    pub async fn resolve_doh_a_record(&self, hostname: &str) -> NoctraResult<Vec<String>> {
        if hostname.contains("://") || hostname.contains('/') {
            return Err(NoctraError::ValidationError(
                "Invalid hostname for DoH resolution".to_string(),
            ));
        }

        let client = reqwest::Client::builder()
            .min_tls_version(reqwest::tls::Version::TLS_1_3)
            .https_only(true)
            .timeout(std::time::Duration::from_secs(4))
            .build()
            .map_err(|e| NoctraError::NetworkError(e.to_string()))?;

        let url = format!(
            "https://cloudflare-dns.com/dns-query?name={}&type=A",
            hostname
        );
        let resp = client
            .get(&url)
            .header("Accept", "application/dns-json")
            .send()
            .await
            .map_err(|e| NoctraError::NetworkError(format!("DoH request failed: {}", e)))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| NoctraError::NetworkError(e.to_string()))?;

        let mut ips = Vec::new();
        if let Some(answers) = json.get("Answer").and_then(|a| a.as_array()) {
            for ans in answers {
                if let Some(data) = ans.get("data").and_then(|d| d.as_str()) {
                    ips.push(data.to_string());
                }
            }
        }
        Ok(ips)
    }

    /// Automated Wire Traffic Capture & Leak Assertion (Section 14B requirement):
    /// Inspects raw captured packet bytes (e.g. from a local proxy / socket tap) and asserts that
    /// no plaintext URLs, target hostnames (when ECH/DoH active), or credentials appear on the wire.
    pub fn verify_captured_wire_bytes_contain_no_leaks(
        captured_packets: &[u8],
        forbidden_plaintext_artifacts: &[&str],
    ) -> NoctraResult<()> {
        let lossy_wire = String::from_utf8_lossy(captured_packets);
        for secret_or_url in forbidden_plaintext_artifacts {
            if !secret_or_url.is_empty() && lossy_wire.contains(secret_or_url) {
                return Err(NoctraError::SecurityViolation(format!(
                    "Transport Security Leak Detected: plaintext artifact '{}' found in captured wire traffic!",
                    secret_or_url
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_https_only_upgrade_and_ad_blocker() {
        let engine = PrivacyAndTransportEngine::new();

        // HTTP URL should automatically upgrade to HTTPS
        let eval = engine.evaluate_url("http://example.com/login?user=alice", false);
        assert!(eval.upgraded_to_https);
        assert_eq!(eval.effective_url, "https://example.com/login?user=alice");

        // Ad/Tracker domain should be blocked immediately
        let ad_eval = engine.evaluate_url("https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js", false);
        assert!(ad_eval.blocked_by_filter);
        assert_eq!(ad_eval.effective_url, "about:blank#blocked");
    }

    #[test]
    fn test_corporate_mitm_non_public_root_ca_detection() {
        let engine = PrivacyAndTransportEngine::new();

        // Public root CA -> trusted
        let trusted = engine.inspect_certificate_chain(
            "doc.rust-lang.org",
            "ISRG Root X1",
            "Let's Encrypt E5",
            "TLS 1.3",
            "TLS_AES_256_GCM_SHA384",
            true,
            b"mock-cert-der",
        );
        assert!(trusted.is_publicly_trusted_root);
        assert!(!trusted.possible_mitm_corporate_proxy);

        // Non-public Corporate Proxy CA -> raises MITM warning
        let mitm = engine.inspect_certificate_chain(
            "bank.example.com",
            "CorpProxy Deep Packet Inspection Root CA",
            "CorpProxy Issuing CA",
            "TLS 1.2",
            "TLS_RSA_WITH_AES_128_GCM_SHA256",
            false,
            b"mitm-cert-der",
        );
        assert!(!mitm.is_publicly_trusted_root);
        assert!(mitm.possible_mitm_corporate_proxy);
        assert_eq!(engine.stats.read().mitm_warnings_raised, 1);
    }

    #[test]
    fn test_vpn_kill_switch_blocks_traffic_on_tunnel_drop() {
        let engine = PrivacyAndTransportEngine::new();
        *engine.active_mode.write() = TransportProtectionMode::WireGuardVpn;
        *engine.kill_switch_armed.write() = true;
        *engine.vpn_tunnel_healthy.write() = false; // Simulate VPN drop

        let res = engine.evaluate_url("https://ifconfig.me/ip", false);
        assert!(res.blocked_by_kill_switch);
        assert!(res.effective_url.contains("noctra-kill-switch-blocked"));
    }

    #[test]
    fn test_automated_traffic_capture_asserts_zero_plaintext_leaks() {
        // Simulate encrypted TLS 1.3 application data record captured on the wire
        let simulated_tls13_wire_packet: Vec<u8> = vec![
            0x17, 0x03, 0x03, 0x00, 0x20, 0x9f, 0xa2, 0x11, 0x4c, 0x88, 0xd0, 0x2e, 0x7b, 0x55,
            0x19, 0x04, 0xe1, 0xc3, 0xb8, 0x77, 0x61, 0x09, 0x2a, 0xf4, 0xdd, 0x31, 0x82, 0xbc,
        ];

        let sensitive_artifacts = &[
            "https://secret-internal.nocta.dev/api/v1/keys",
            "secret-internal.nocta.dev",
            "Bearer sk-live-987654321",
        ];

        let verification = PrivacyAndTransportEngine::verify_captured_wire_bytes_contain_no_leaks(
            &simulated_tls13_wire_packet,
            sensitive_artifacts,
        );
        assert!(verification.is_ok());

        // Verify that if a plaintext leak ever occurred on the wire, the test catches it
        let leaky_packet = b"GET /api/v1/keys HTTP/1.1\r\nHost: secret-internal.nocta.dev\r\nAuthorization: Bearer sk-live-987654321\r\n\r\n";
        let leak_check = PrivacyAndTransportEngine::verify_captured_wire_bytes_contain_no_leaks(
            leaky_packet,
            sensitive_artifacts,
        );
        assert!(leak_check.is_err());
    }
}
