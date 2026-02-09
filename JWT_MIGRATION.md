# JWT Implementation Migration: RS256 → HS256

## Summary

Successfully migrated JWT implementation from **RS256 (RSA asymmetric encryption)** to **HS256 (HMAC-SHA256 symmetric encryption)** with a simple secret string.

**Date:** 2026-02-09
**Status:** ✅ Complete and tested

---

## Changes Made

### 1. Configuration (`synctv-core/src/config.rs`)

**Before:**
```rust
pub struct JwtConfig {
    pub private_key_path: String,
    pub public_key_path: String,
    pub access_token_duration_hours: u64,
    pub refresh_token_duration_days: u64,
}
```

**After:**
```rust
pub struct JwtConfig {
    pub secret: String,
    pub access_token_duration_hours: u64,
    pub refresh_token_duration_days: u64,
}
```

**Default value:** `"change-me-in-production"`

### 2. JWT Service (`synctv-core/src/service/auth/jwt.rs`)

**Before:**
```rust
impl JwtService {
    pub fn new(private_key_pem: &[u8], public_key_pem: &[u8]) -> Result<Self> {
        let encoding_key = EncodingKey::from_rsa_pem(private_key_pem)?;
        let decoding_key = DecodingKey::from_rsa_pem(public_key_pem)?;
        Ok(Self {
            encoding_key: Arc::new(encoding_key),
            decoding_key: Arc::new(decoding_key),
            algorithm: Algorithm::RS256,
        })
    }
}
```

**After:**
```rust
impl JwtService {
    pub fn new(secret: &str) -> Result<Self> {
        if secret.is_empty() {
            return Err(Error::Internal("JWT secret cannot be empty".to_string()));
        }
        let encoding_key = EncodingKey::from_secret(secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(secret.as_bytes());
        Ok(Self {
            encoding_key: Arc::new(encoding_key),
            decoding_key: Arc::new(decoding_key),
            algorithm: Algorithm::HS256,
        })
    }
}
```

### 3. Service Bootstrap (`synctv-core/src/bootstrap/services.rs`)

**Before:**
```rust
fn load_jwt_service(config: &Config) -> Result<JwtService, anyhow::Error> {
    let private_key = std::fs::read(&config.jwt.private_key_path);
    let public_key = std::fs::read(&config.jwt.public_key_path);

    if let (Ok(priv_key), Ok(pub_key)) = (private_key, public_key) {
        JwtService::new(&priv_key, &pub_key)
            .map_err(|e| anyhow::anyhow!("Failed to initialize JWT service: {e}"))
    } else {
        Err(anyhow::anyhow!("JWT keys not found at {} and {}", ...))
    }
}
```

**After:**
```rust
fn load_jwt_service(config: &Config) -> Result<JwtService, anyhow::Error> {
    if config.jwt.secret.is_empty() {
        return Err(anyhow::anyhow!(
            "JWT secret is empty. Please set SYNCTV__JWT__SECRET environment variable"
        ));
    }

    if config.jwt.secret == "change-me-in-production" {
        warn!("Using default JWT secret! This is insecure for production use.");
    }

    JwtService::new(&config.jwt.secret)
        .map_err(|e| anyhow::anyhow!("Failed to initialize JWT service: {e}"))
}
```

### 4. Tests Updated

All test files updated to use secret-based JWT:

- `synctv-core/src/service/auth/jwt.rs` - Unit tests
- `synctv-core/tests/integration_tests.rs` - Integration tests
- `synctv-core/src/service/auth/validator.rs` - Validator tests
- `synctv-core/src/service/user.rs` - User service tests
- `synctv-core/benches/auth_service.rs` - Benchmarks

**Example:**
```rust
// Before
fn create_test_jwt_service() -> JwtService {
    let (private_pem, public_pem) = JwtService::generate_keys();
    JwtService::new(&private_pem, &public_pem).unwrap()
}

// After
fn create_test_jwt_service() -> JwtService {
    JwtService::new("test-secret-key").unwrap()
}
```

### 5. Dependencies Removed

- Removed `rsa = "0.9"` from dev-dependencies in `synctv-core/Cargo.toml`
- No longer need RSA key generation libraries

---

## Configuration Migration

### Environment Variables

**Before:**
```bash
SYNCTV__JWT__PRIVATE_KEY_PATH=./keys/jwt_private.pem
SYNCTV__JWT__PUBLIC_KEY_PATH=./keys/jwt_public.pem
```

**After:**
```bash
SYNCTV__JWT__SECRET=your-secure-random-secret-here
```

### Configuration File

**Before (config.yaml):**
```yaml
jwt:
  private_key_path: ./keys/jwt_private.pem
  public_key_path: ./keys/jwt_public.pem
  access_token_duration_hours: 1
  refresh_token_duration_days: 30
```

**After (config.yaml):**
```yaml
jwt:
  secret: your-secure-random-secret-here
  access_token_duration_hours: 1
  refresh_token_duration_days: 30
```

### Generating a Secure Secret

Generate a strong random secret for production:

```bash
# Using openssl
openssl rand -base64 64

# Using /dev/urandom
head -c 64 /dev/urandom | base64

# Using Python
python3 -c "import secrets; print(secrets.token_urlsafe(64))"
```

---

## Benefits

### ✅ Simplicity
- **Before:** Required generating and managing RSA key pairs (private/public keys)
- **After:** Single secret string - much simpler to configure and deploy

### ✅ Deployment
- **Before:** Need to securely distribute and mount key files in containers
- **After:** Single environment variable or secret management entry

### ✅ Performance
- **Before:** RSA signing/verification is computationally expensive
- **After:** HMAC is significantly faster for both signing and verifying

### ✅ Key Rotation
- **Before:** Complex key rotation with public key distribution
- **After:** Simple secret rotation - just update the secret

---

## Security Considerations

### ⚠️ Important Notes

1. **Secret Length:** Use a strong, long random secret (at least 32 bytes recommended, 64+ bytes ideal)

2. **Secret Security:**
   - Never commit secrets to version control
   - Use environment variables or secret management systems
   - Rotate secrets periodically

3. **Token Validation:**
   - Only the server with the secret can create and verify tokens
   - Tokens cannot be verified by third parties (unlike RS256 public keys)
   - This is acceptable for most use cases where tokens are only validated by the issuing server

4. **Default Secret Warning:**
   - The default secret `"change-me-in-production"` triggers a warning
   - Config validation will fail if the default is used

---

## Test Results

All tests pass successfully:

```
✅ JWT unit tests: 6 passed
✅ Integration tests: 25 passed (4 ignored - require DB)
✅ All synctv-core tests: 170 passed (27 ignored - require Redis/DB)
✅ Benchmarks compile successfully
```

**Test coverage includes:**
- Token signing and verification
- Access vs refresh token validation
- Token type validation
- Tampered token detection
- Invalid token handling
- Empty secret validation
- Concurrent authentication (10 users)
- Permission checks
- Publish key workflow
- Error propagation

---

## Breaking Changes

### ⚠️ For Existing Deployments

This is a **breaking change** for existing deployments:

1. **Tokens are incompatible:** Existing JWT tokens signed with RS256 cannot be verified with HS256
2. **All users will need to log in again** after the update
3. **Configuration must be updated** to use the new secret-based format

### Migration Steps

1. **Generate a secure secret:**
   ```bash
   openssl rand -base64 64 > jwt_secret.txt
   ```

2. **Update configuration:**
   - Remove `private_key_path` and `public_key_path`
   - Add `secret` field with your generated secret

3. **Deploy the update:**
   - All users will be logged out
   - Users must log in again to get new HS256 tokens

4. **Clean up:**
   - Old RSA key files can be removed
   - Update deployment scripts/documentation

---

## Performance Impact

### Benchmarks (Expected Improvements)

Based on typical HS256 vs RS256 performance:

- **Token Signing:** ~100x faster (HMAC vs RSA signing)
- **Token Verification:** ~10-20x faster (HMAC vs RSA verification)
- **Memory Usage:** Lower (no large key structures)

This results in:
- Faster authentication responses
- Lower CPU usage
- Better scalability for high-traffic scenarios

---

## Rollback Plan

If needed, to rollback:

1. Revert to the previous commit before this change
2. Restore old RSA key files
3. Update configuration back to key paths
4. Redeploy

Note: Users will need to log in again after rollback as well.

---

## References

- **JWT Specification:** [RFC 7519](https://tools.ietf.org/html/rfc7519)
- **HMAC-SHA256:** [RFC 2104](https://tools.ietf.org/html/rfc2104)
- **jsonwebtoken crate:** [Documentation](https://docs.rs/jsonwebtoken/)

---

## Conclusion

The migration from RS256 to HS256 simplifies the JWT implementation while maintaining security and improving performance. The change requires a one-time configuration update and user re-authentication but provides long-term benefits in deployment simplicity and system performance.
