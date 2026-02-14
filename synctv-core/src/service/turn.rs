//! TURN Server Integration
//!
//! Provides integration with external TURN (Traversal Using Relays around NAT) servers
//! for WebRTC connectivity in challenging network environments.
//!
//! ## TURN Overview
//! - Used when P2P connection fails (Symmetric NAT scenarios)
//! - Server acts as relay, forwarding media between peers
//! - Required for ~25-30% of connections
//! - Higher cost than STUN (relays all media traffic)
//!
//! ## Coturn Integration
//! This module is designed to work with coturn (<https://github.com/coturn/coturn>),
//! the most widely deployed open-source TURN server.
//!
//! ## Credential Generation
//! - Uses RFC 5389 long-term credentials
//! - HMAC-SHA1 based on shared secret
//! - Time-limited credentials (default 24 hours)
//! - Compatible with coturn's `static-auth-secret` mode

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::time::Duration;
use base64::Engine;

/// TURN server configuration
#[derive(Debug, Clone)]
pub struct TurnConfig {
    /// TURN server URL (e.g., "turn:turn.example.com:3478")
    pub server_url: String,

    /// Static auth secret (must match coturn's configuration)
    pub static_secret: String,

    /// Credential time-to-live (default: 24 hours)
    pub credential_ttl: Duration,

    /// Whether to use TLS/DTLS (turns: or turn: with ?transport=tcp)
    pub use_tls: bool,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            static_secret: String::new(),
            credential_ttl: Duration::from_hours(24), // 24 hours
            use_tls: false,
        }
    }
}

/// TURN credentials (username and password)
#[derive(Debug, Clone)]
pub struct TurnCredential {
    /// Username in format: "<timestamp>:<`user_identifier`>"
    pub username: String,

    /// HMAC-SHA1 based password
    pub password: String,

    /// Credential expiry time
    pub expires_at: DateTime<Utc>,
}

/// TURN credential generation service
#[derive(Clone)]
pub struct TurnCredentialService {
    config: TurnConfig,
}

impl TurnCredentialService {
    /// Create a new TURN credential service
    #[must_use] 
    pub const fn new(config: TurnConfig) -> Self {
        Self { config }
    }

    /// Generate time-limited TURN credentials for a user
    ///
    /// Credentials format (RFC 5389 long-term credentials):
    /// - Username: `<expiry_timestamp>:<user_id>`
    /// - Password: base64(HMAC-SHA1(secret, username))
    ///
    /// This format is compatible with coturn's `static-auth-secret` mode.
    pub fn generate_credential(&self, user_id: &str) -> anyhow::Result<TurnCredential> {
        // Calculate expiry timestamp
        let now = Utc::now();
        let expires_at = now + chrono::Duration::from_std(self.config.credential_ttl)?;
        let expiry_timestamp = expires_at.timestamp();

        // Format: "<timestamp>:<user_id>"
        let username = format!("{expiry_timestamp}:{user_id}");

        // Generate HMAC-SHA1 password
        let password = self.compute_hmac(&username)?;

        Ok(TurnCredential {
            username,
            password,
            expires_at,
        })
    }

    /// Compute HMAC-SHA1 for credential generation
    fn compute_hmac(&self, username: &str) -> anyhow::Result<String> {
        let mut mac = Hmac::<Sha1>::new_from_slice(self.config.static_secret.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to create HMAC: {e}"))?;

        mac.update(username.as_bytes());
        let result = mac.finalize();
        let credential = base64::engine::general_purpose::STANDARD.encode(result.into_bytes());

        Ok(credential)
    }

    /// Verify if a credential is still valid
    #[must_use] 
    pub fn is_credential_valid(&self, credential: &TurnCredential) -> bool {
        Utc::now() < credential.expires_at
    }

    /// Get TURN server URLs
    #[must_use] 
    pub fn get_urls(&self) -> Vec<String> {
        let mut urls = vec![self.config.server_url.clone()];

        // Add TLS variant if enabled
        if self.config.use_tls {
            let tls_url = self.config.server_url.replace("turn:", "turns:");
            if tls_url != self.config.server_url {
                urls.push(tls_url);
            }
        }

        urls
    }

    /// Validate TURN configuration
    pub fn validate_config(&self) -> anyhow::Result<()> {
        if self.config.server_url.is_empty() {
            return Err(anyhow::anyhow!("TURN server URL is empty"));
        }

        if !self.config.server_url.starts_with("turn:") && !self.config.server_url.starts_with("turns:") {
            return Err(anyhow::anyhow!("TURN server URL must start with 'turn:' or 'turns:'"));
        }

        if self.config.static_secret.is_empty() {
            return Err(anyhow::anyhow!("TURN static secret is empty"));
        }

        if self.config.static_secret.len() < 32 {
            return Err(anyhow::anyhow!("TURN static secret should be at least 32 characters"));
        }

        if self.config.credential_ttl.as_secs() < 60 {
            return Err(anyhow::anyhow!("TURN credential TTL should be at least 60 seconds"));
        }

        if self.config.credential_ttl.as_secs() > 86400 * 7 {
            return Err(anyhow::anyhow!("TURN credential TTL should not exceed 7 days"));
        }

        Ok(())
    }
}

/// TURN server deployment guide
pub const COTURN_DEPLOYMENT_GUIDE: &str = r#"
# Coturn Deployment Guide for SyncTV

## Installation

### Ubuntu/Debian:
```bash
sudo apt-get update
sudo apt-get install coturn
```

### CentOS/RHEL:
```bash
sudo yum install coturn
```

### Docker:
```bash
docker pull coturn/coturn
```

## Configuration

Edit `/etc/turnserver.conf`:

```conf
# Listening IP (use 0.0.0.0 for all interfaces)
listening-ip=0.0.0.0

# External IP (your server's public IP)
external-ip=YOUR_PUBLIC_IP

# Listening ports
listening-port=3478
tls-listening-port=5349

# Relay IP range
min-port=49152
max-port=65535

# Authentication
use-auth-secret
static-auth-secret=YOUR_SECRET_HERE  # Must match WebRTCConfig.turn_static_secret

# Realm (can be your domain)
realm=turn.example.com

# Logging
log-file=/var/log/coturn/turnserver.log
verbose

# Security
no-multicast-peers
no-loopback-peers

# Performance
total-quota=100
bps-capacity=0

# TLS/DTLS (optional, for turns: protocol)
cert=/etc/letsencrypt/live/turn.example.com/cert.pem
pkey=/etc/letsencrypt/live/turn.example.com/privkey.pem
```

## Start Service

```bash
# Enable on boot
sudo systemctl enable coturn

# Start service
sudo systemctl start coturn

# Check status
sudo systemctl status coturn
```

## Firewall Rules

```bash
# UDP/TCP for TURN
sudo ufw allow 3478/tcp
sudo ufw allow 3478/udp

# TLS/DTLS for TURNS
sudo ufw allow 5349/tcp
sudo ufw allow 5349/udp

# Media relay ports
sudo ufw allow 49152:65535/tcp
sudo ufw allow 49152:65535/udp
```

## SyncTV Configuration

In `config.yaml`:

```yaml
webrtc:
  mode: peer_to_peer  # or hybrid
  enable_turn: true
  turn_server_url: "turn:turn.example.com:3478"
  turn_static_secret: "YOUR_SECRET_HERE"  # Must match coturn config
  turn_credential_ttl: 86400  # 24 hours
```

## Testing

Test with Trickle ICE:
https://webrtc.github.io/samples/src/content/peerconnection/trickle-ice/

Enter your TURN server URL and credentials to verify connectivity.

## Monitoring

```bash
# View logs
sudo tail -f /var/log/coturn/turnserver.log

# Check connections
sudo turnutils_uclient -v turn.example.com

# Monitor with prometheus
# Coturn supports prometheus metrics on port 9641
```

## Cost Estimation

- Small deployment (< 100 users): ~$20-50/month
- Medium deployment (100-1000 users): ~$100-300/month
- Large deployment (1000+ users): ~$500+/month

Most traffic will still use P2P (STUN), TURN is fallback only (~25-30% of connections).
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_credential() {
        let config = TurnConfig {
            server_url: "turn:turn.example.com:3478".to_string(),
            static_secret: "test_secret_key_1234567890abcdefgh".to_string(),
            credential_ttl: Duration::from_secs(3600),
            use_tls: false,
        };

        let service = TurnCredentialService::new(config);
        let credential = service.generate_credential("user123").unwrap();

        // Username should be in format: "<timestamp>:<user_id>"
        assert!(credential.username.contains(":user123"));

        // Password should be base64 encoded
        assert!(!credential.password.is_empty());
        assert!(base64::engine::general_purpose::STANDARD.decode(&credential.password).is_ok());

        // Expiry should be in the future
        assert!(credential.expires_at > Utc::now());
    }

    #[test]
    fn test_credential_validation() {
        let config = TurnConfig {
            server_url: "turn:turn.example.com:3478".to_string(),
            static_secret: "test_secret_key_1234567890abcdefgh".to_string(),
            credential_ttl: Duration::from_secs(3600),
            use_tls: false,
        };

        let service = TurnCredentialService::new(config);
        let credential = service.generate_credential("user123").unwrap();

        // Fresh credential should be valid
        assert!(service.is_credential_valid(&credential));

        // Expired credential should be invalid
        let expired_credential = TurnCredential {
            username: credential.username.clone(),
            password: credential.password.clone(),
            expires_at: Utc::now() - chrono::Duration::hours(1),
        };
        assert!(!service.is_credential_valid(&expired_credential));
    }

    #[test]
    fn test_get_urls() {
        let config = TurnConfig {
            server_url: "turn:turn.example.com:3478".to_string(),
            static_secret: "test_secret".to_string(),
            credential_ttl: Duration::from_secs(3600),
            use_tls: true,
        };

        let service = TurnCredentialService::new(config);
        let urls = service.get_urls();

        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"turn:turn.example.com:3478".to_string()));
        assert!(urls.contains(&"turns:turn.example.com:3478".to_string()));
    }

    #[test]
    fn test_validate_config() {
        // Valid config
        let config = TurnConfig {
            server_url: "turn:turn.example.com:3478".to_string(),
            static_secret: "test_secret_key_1234567890abcdefgh".to_string(),
            credential_ttl: Duration::from_secs(3600),
            use_tls: false,
        };
        let service = TurnCredentialService::new(config);
        assert!(service.validate_config().is_ok());

        // Invalid: empty URL
        let config = TurnConfig {
            server_url: String::new(),
            static_secret: "test_secret_key_1234567890abcdefgh".to_string(),
            credential_ttl: Duration::from_secs(3600),
            use_tls: false,
        };
        let service = TurnCredentialService::new(config);
        assert!(service.validate_config().is_err());

        // Invalid: short secret
        let config = TurnConfig {
            server_url: "turn:turn.example.com:3478".to_string(),
            static_secret: "short".to_string(),
            credential_ttl: Duration::from_secs(3600),
            use_tls: false,
        };
        let service = TurnCredentialService::new(config);
        assert!(service.validate_config().is_err());

        // Invalid: TTL too short
        let config = TurnConfig {
            server_url: "turn:turn.example.com:3478".to_string(),
            static_secret: "test_secret_key_1234567890abcdefgh".to_string(),
            credential_ttl: Duration::from_secs(30),
            use_tls: false,
        };
        let service = TurnCredentialService::new(config);
        assert!(service.validate_config().is_err());
    }

    #[test]
    fn test_hmac_deterministic() {
        let config = TurnConfig {
            server_url: "turn:turn.example.com:3478".to_string(),
            static_secret: "test_secret_key_1234567890abcdefgh".to_string(),
            credential_ttl: Duration::from_secs(3600),
            use_tls: false,
        };

        let service = TurnCredentialService::new(config);

        let username = "12345:user123";
        let hmac1 = service.compute_hmac(username).unwrap();
        let hmac2 = service.compute_hmac(username).unwrap();

        // HMAC should be deterministic
        assert_eq!(hmac1, hmac2);
    }
}
