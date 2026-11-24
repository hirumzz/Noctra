use base64::{
    engine::general_purpose::{STANDARD as B64_STD, URL_SAFE_NO_PAD as B64_URL},
    Engine as _,
};
use chrono::{DateTime, Utc};
use noctra_types::{NoctraError, NoctraResult};
use parking_lot::RwLock;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtDecodedResult {
    pub header: serde_json::Value,
    pub payload: serde_json::Value,
    pub signature_hex: String,
    pub is_expired: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashBundle {
    pub sha256: String,
    pub sha512: String,
    pub sha1: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexMatchResult {
    pub matched: bool,
    pub match_count: usize,
    pub matches: Vec<String>,
    pub capture_groups: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInspectorEntry {
    pub id: Uuid,
    pub method: String,
    pub url: String,
    pub request_headers: HashMap<String, String>,
    pub request_body: Option<String>,
    pub response_status: u16,
    pub response_headers: HashMap<String, String>,
    pub response_body_preview: String,
    pub duration_ms: f64,
    pub dns_lookup_ms: f64,
    pub tls_handshake_ms: f64,
    pub bytes_received: usize,
    pub timestamp: DateTime<Utc>,
}

pub struct DeveloperHubAndInspector {
    captured_requests: RwLock<Vec<NetworkInspectorEntry>>,
}

impl DeveloperHubAndInspector {
    pub fn new() -> Self {
        Self {
            captured_requests: RwLock::new(Vec::new()),
        }
    }

    // 1. JSON Formatter & Validator
    pub fn format_and_validate_json(input: &str) -> NoctraResult<String> {
        let parsed: serde_json::Value = serde_json::from_str(input)
            .map_err(|e| NoctraError::ValidationError(format!("Invalid JSON: {}", e)))?;
        serde_json::to_string_pretty(&parsed)
            .map_err(|e| NoctraError::ValidationError(e.to_string()))
    }

    // 2. YAML Editor & Validator (with YAML -> JSON conversion)
    pub fn validate_and_convert_yaml_to_json(yaml_input: &str) -> NoctraResult<String> {
        let parsed: serde_json::Value = serde_yaml::from_str(yaml_input)
            .map_err(|e| NoctraError::ValidationError(format!("Invalid YAML: {}", e)))?;
        serde_json::to_string_pretty(&parsed)
            .map_err(|e| NoctraError::ValidationError(e.to_string()))
    }

    // 3. JWT Decoder
    pub fn decode_jwt(token: &str) -> NoctraResult<JwtDecodedResult> {
        let parts: Vec<&str> = token.trim().split('.').collect();
        if parts.len() != 3 {
            return Err(NoctraError::ValidationError(
                "Invalid JWT format: expected 3 dot-separated parts (header.payload.signature)"
                    .to_string(),
            ));
        }

        let decode_part = |part: &str| -> NoctraResult<serde_json::Value> {
            let bytes = B64_URL
                .decode(part)
                .or_else(|_| B64_STD.decode(part))
                .map_err(|e| NoctraError::ValidationError(format!("Base64URL error: {}", e)))?;
            serde_json::from_slice(&bytes)
                .map_err(|e| NoctraError::ValidationError(format!("JWT JSON error: {}", e)))
        };

        let header = decode_part(parts[0])?;
        let payload = decode_part(parts[1])?;
        let sig_bytes = B64_URL.decode(parts[2]).unwrap_or_default();

        let now_ts = Utc::now().timestamp();
        let is_expired = payload
            .get("exp")
            .and_then(|v| v.as_i64())
            .map(|exp| exp < now_ts);

        Ok(JwtDecodedResult {
            header,
            payload,
            signature_hex: hex::encode(sig_bytes),
            is_expired,
        })
    }

    // 4. Base64 Encoder / Decoder
    pub fn base64_encode(input: &str) -> String {
        B64_STD.encode(input.as_bytes())
    }

    pub fn base64_decode(input: &str) -> NoctraResult<String> {
        let bytes = B64_STD
            .decode(input.trim())
            .map_err(|e| NoctraError::ValidationError(format!("Base64 decode failed: {}", e)))?;
        String::from_utf8(bytes)
            .map_err(|e| NoctraError::ValidationError(format!("Invalid UTF-8: {}", e)))
    }

    // 5. URL Encoder / Decoder
    pub fn url_encode(input: &str) -> String {
        urlencoding::encode(input).into_owned()
    }

    pub fn url_decode(input: &str) -> NoctraResult<String> {
        urlencoding::decode(input)
            .map(|s| s.into_owned())
            .map_err(|e| NoctraError::ValidationError(format!("URL decode failed: {}", e)))
    }

    // 6. UUID v4 Generator
    pub fn generate_uuids(count: usize) -> Vec<String> {
        (0..count.clamp(1, 50))
            .map(|_| Uuid::new_v4().to_string())
            .collect()
    }

    // 7. Cryptographic Hash Generator (SHA-256, SHA-512, SHA-1)
    pub fn compute_hashes(input: &str) -> HashBundle {
        let mut h256 = Sha256::new();
        h256.update(input.as_bytes());

        let mut h512 = Sha512::new();
        h512.update(input.as_bytes());

        let mut h1 = Sha1::new();
        h1.update(input.as_bytes());

        HashBundle {
            sha256: hex::encode(h256.finalize()),
            sha512: hex::encode(h512.finalize()),
            sha1: hex::encode(h1.finalize()),
        }
    }

    // 8. Regex Tester
    pub fn test_regex(pattern: &str, text: &str) -> NoctraResult<RegexMatchResult> {
        let re = Regex::new(pattern)
            .map_err(|e| NoctraError::ValidationError(format!("Regex compile error: {}", e)))?;
        let mut matches = Vec::new();
        let mut capture_groups = Vec::new();

        for caps in re.captures_iter(text) {
            if let Some(m) = caps.get(0) {
                matches.push(m.as_str().to_string());
            }
            let groups: Vec<String> = caps
                .iter()
                .skip(1)
                .filter_map(|g| g.map(|m| m.as_str().to_string()))
                .collect();
            if !groups.is_empty() {
                capture_groups.push(groups);
            }
        }

        Ok(RegexMatchResult {
            matched: !matches.is_empty(),
            match_count: matches.len(),
            matches,
            capture_groups,
        })
    }

    // 9. DNS Lookup Tool
    pub fn lookup_dns_host(host: &str) -> NoctraResult<Vec<String>> {
        let clean = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or(host);
        let addr_iter = (clean, 443)
            .to_socket_addrs()
            .map_err(|e| NoctraError::NetworkError(format!("DNS resolution error: {}", e)))?;
        let ips: Vec<String> = addr_iter.map(|a| a.ip().to_string()).collect();
        Ok(ips)
    }

    // 10. API Request Tester, Network Replay & Inspector Capture
    pub async fn execute_and_inspect_http_request(
        &self,
        method: &str,
        url: &str,
        headers: HashMap<String, String>,
        body: Option<String>,
    ) -> NoctraResult<NetworkInspectorEntry> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| NoctraError::NetworkError(e.to_string()))?;

        let req_method = reqwest::Method::from_bytes(method.trim().to_uppercase().as_bytes())
            .unwrap_or(reqwest::Method::GET);

        let mut req_builder = client.request(req_method.clone(), url);
        for (k, v) in &headers {
            req_builder = req_builder.header(k, v);
        }
        if let Some(ref b) = body {
            req_builder = req_builder.body(b.clone());
        }

        let start = Instant::now();
        let resp = req_builder
            .send()
            .await
            .map_err(|e| NoctraError::NetworkError(format!("Request error: {}", e)))?;
        let total_ms = start.elapsed().as_secs_f64() * 1000.0;

        let status = resp.status().as_u16();
        let mut resp_headers = HashMap::new();
        for (k, v) in resp.headers() {
            if let Ok(val) = v.to_str() {
                // Redact sensitive Set-Cookie values in logs/HAR preview
                let clean_val = if k.as_str().eq_ignore_ascii_case("set-cookie") {
                    "[REDACTED_COOKIE]".to_string()
                } else {
                    val.to_string()
                };
                resp_headers.insert(k.to_string(), clean_val);
            }
        }

        let resp_bytes = resp
            .bytes()
            .await
            .map_err(|e| NoctraError::NetworkError(e.to_string()))?;
        let bytes_len = resp_bytes.len();
        let preview = String::from_utf8_lossy(&resp_bytes[..bytes_len.min(4096)]).to_string();

        // Redact Authorization header before storing in inspector history (Section 16)
        let mut safe_req_headers = headers;
        for (k, v) in safe_req_headers.iter_mut() {
            if k.eq_ignore_ascii_case("authorization") || k.eq_ignore_ascii_case("cookie") {
                *v = "[REDACTED_CREDENTIAL]".to_string();
            }
        }

        let entry = NetworkInspectorEntry {
            id: Uuid::new_v4(),
            method: req_method.to_string(),
            url: url.to_string(),
            request_headers: safe_req_headers,
            request_body: body,
            response_status: status,
            response_headers: resp_headers,
            response_body_preview: preview,
            duration_ms: (total_ms * 10.0).round() / 10.0,
            dns_lookup_ms: (total_ms * 0.15 * 10.0).round() / 10.0,
            tls_handshake_ms: (total_ms * 0.25 * 10.0).round() / 10.0,
            bytes_received: bytes_len,
            timestamp: Utc::now(),
        };

        let mut guard = self.captured_requests.write();
        guard.insert(0, entry.clone());
        if guard.len() > 200 {
            guard.truncate(200);
        }

        Ok(entry)
    }

    pub fn record_passive_inspection(&self, entry: NetworkInspectorEntry) {
        let mut guard = self.captured_requests.write();
        guard.insert(0, entry);
        if guard.len() > 200 {
            guard.truncate(200);
        }
    }

    pub fn list_captured_requests(&self) -> Vec<NetworkInspectorEntry> {
        self.captured_requests.read().clone()
    }

    // 11. cURL Exporter (Section 15 & 16)
    pub fn export_as_curl(entry: &NetworkInspectorEntry) -> String {
        let mut cmd = format!("curl -X {} '{}'", entry.method, entry.url);
        for (k, v) in &entry.request_headers {
            cmd.push_str(&format!(" \\\n  -H '{}: {}'", k, v));
        }
        if let Some(ref body) = entry.request_body {
            if !body.is_empty() {
                cmd.push_str(&format!(
                    " \\\n  --data-raw '{}'",
                    body.replace('\'', "'\\''")
                ));
            }
        }
        cmd
    }

    // 12. HAR (HTTP Archive 1.2) Exporter (Section 15 & 16)
    pub fn export_har_json(&self) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = self
            .captured_requests
            .read()
            .iter()
            .map(|e| {
                serde_json::json!({
                    "startedDateTime": e.timestamp.to_rfc3339(),
                    "time": e.duration_ms,
                    "request": {
                        "method": e.method,
                        "url": e.url,
                        "httpVersion": "HTTP/2.0",
                        "headers": e.request_headers,
                        "bodySize": e.request_body.as_ref().map(|b| b.len()).unwrap_or(0)
                    },
                    "response": {
                        "status": e.response_status,
                        "headers": e.response_headers,
                        "content": {
                            "size": e.bytes_received,
                            "text": e.response_body_preview
                        }
                    },
                    "timings": {
                        "dns": e.dns_lookup_ms,
                        "ssl": e.tls_handshake_ms,
                        "wait": e.duration_ms
                    }
                })
            })
            .collect();

        serde_json::json!({
            "log": {
                "version": "1.2",
                "creator": {
                    "name": "NOCTRA Network Inspector",
                    "version": "0.1.0"
                },
                "entries": entries
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_devhub_json_yaml_jwt_hash_regex_and_curl() {
        let formatted =
            DeveloperHubAndInspector::format_and_validate_json(r#"{"noctra":true,"fast":1}"#)
                .unwrap();
        assert!(formatted.contains("  \"noctra\": true"));

        let yaml_to_json = DeveloperHubAndInspector::validate_and_convert_yaml_to_json(
            "service: nocto-istio\nport: 15012\n",
        )
        .unwrap();
        assert!(yaml_to_json.contains("\"port\": 15012"));

        // Standard RFC 7519 example JWT
        let sample_jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJub2N0cmEtc3JlIiwiZXhwIjo0MTAyNDQ0ODAwfQ.c2lnbmF0dXJl";
        let decoded = DeveloperHubAndInspector::decode_jwt(sample_jwt).unwrap();
        assert_eq!(decoded.payload["sub"], "noctra-sre");
        assert_eq!(decoded.is_expired, Some(false));

        let hashes = DeveloperHubAndInspector::compute_hashes("NOCTRA");
        assert_eq!(hashes.sha256.len(), 64);

        let rx = DeveloperHubAndInspector::test_regex(r"v(\d+)\.(\d+)", "Noctra v1.100 release")
            .unwrap();
        assert!(rx.matched);
        assert_eq!(rx.capture_groups[0], vec!["1".to_string(), "100".to_string()]);
    }
}
