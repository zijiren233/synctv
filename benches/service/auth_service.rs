//! Service benchmarks for authentication operations
//!
//! Run with: cargo bench --bench auth_service

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use synctv_core::service::auth::{JwtService, TokenType};
use synctv_core::models::UserId;
use std::time::Duration;

/// Benchmark: JWT token generation
fn bench_jwt_sign(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        // Note: JwtService::generate_keys is #[cfg(test)], so we need keys
        // In production benchmarks, load actual keys from files
        // For now, we'll create a placeholder benchmark

        c.bench_function("jwt_sign_access_token", |b| {
            b.to_async(&rt).iter(|| {
                // TODO: Create actual JwtService with test keys
                // let jwt_service = JwtService::new(&private_key, &public_key).unwrap();
                // let user_id = UserId::new();
                // async { jwt_service.sign_token(&user_id, 0, TokenType::Access) }
                async { std::future::ready::<(), ()>(()).await }
            })
        });
    });
}

/// Benchmark: JWT token verification
fn bench_jwt_verify(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        c.bench_function("jwt_verify_access_token", |b| {
            b.to_async(&rt).iter(|| {
                // TODO: Create actual JwtService with test keys
                // let jwt_service = JwtService::new(&private_key, &public_key).unwrap();
                // let token = "valid.jwt.token";
                // async { jwt_service.verify_access_token(token) }
                async { std::future::ready::<(), ()>(()).await }
            })
        });
    });
}

/// Benchmark: Password hashing (argon2)
fn bench_password_hash(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        c.bench_function("password_hash", |b| {
            b.to_async(&rt).iter(|| {
                async {
                    // TODO: Use actual password hashing function
                    // let password = "test_password_123";
                    // hash_password(password).await
                    std::future::ready::<(), ()>(()).await
                }
            })
        });
    });
}

/// Benchmark: Password verification
fn bench_password_verify(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        c.bench_function("password_verify", |b| {
            b.to_async(&rt).iter(|| {
                async {
                    // TODO: Use actual password verification function
                    // let password = "test_password_123";
                    // let hash = "$argon2id$v=19$m=4096,t=3,p=1...";
                    // verify_password(password, hash).await
                    std::future::ready::<(), ()>(()).await
                }
            })
        });
    });
}

/// Benchmark: Concurrent token generation
fn bench_concurrent_token_generation(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        let mut group = c.benchmark_group("concurrent_token_generation");
        group.measurement_time(Duration::from_secs(5));

        for num_concurrent in [10, 50, 100].iter() {
            group.bench_with_input(BenchmarkId::from_parameter(num_concurrent), num_concurrent, |b, &num_concurrent| {
                b.to_async(&rt).iter(|| {
                    async {
                        let mut tasks = Vec::new();
                        for _ in 0..num_concurrent {
                            tasks.push(tokio::spawn(async {
                                // TODO: Create actual token generation
                                // jwt_service.sign_token(&user_id, 0, TokenType::Access)
                                std::future::ready::<(), ()>(()).await
                            }));
                        }
                        for task in tasks {
                            task.await.unwrap();
                        }
                    }
                })
            });
        }

        group.finish();
    });
}

criterion_group!(
    benches,
    bench_jwt_sign,
    bench_jwt_verify,
    bench_password_hash,
    bench_password_verify,
    bench_concurrent_token_generation
);
criterion_main!(benches);
