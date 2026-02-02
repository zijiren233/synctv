# Performance Benchmarks

This directory contains performance benchmarks for the SyncTV Rust implementation.

## Running Benchmarks

Run all benchmarks:
```bash
cargo bench
```

Run specific benchmark:
```bash
cargo bench --bench user_cache
cargo bench --bench room_queries
cargo bench --bench auth_service
```

## Benchmark Structure

```
benches/
├── database/
│   ├── user_queries.rs     # Database query performance (users)
│   ├── room_queries.rs     # Database query performance (rooms)
│   └── chat_queries.rs     # Database query performance (chat)
├── cache/
│   ├── user_cache.rs       # User cache performance
│   └── room_cache.rs       # Room cache performance
├── service/
│   ├── auth_service.rs     # Authentication service performance
│   ├── room_service.rs     # Room service performance
│   └── chat_service.rs     # Chat service performance
└── README.md
```

## Understanding Results

Benchmark results are saved to `target/criterion/`. Open `target/criterion/report/index.html` in a web browser to view detailed results.

## Key Metrics

Target performance metrics:
- **API response time P99**: < 200ms
- **Database query time P99**: < 50ms
- **Cache hit rate**: > 80%
- **WebSocket message latency**: < 100ms
- **Concurrent connections**: > 10,000

## Adding New Benchmarks

When adding new benchmarks:

1. Follow the existing structure
2. Use Criterion's benchmark groups for related benchmarks
3. Add appropriate measurement times
4. Include both cold and warm cache scenarios
5. Test with varying data sizes
6. Include concurrent access patterns

Example:
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_my_function(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("my_function", |b| {
        b.to_async(&rt).iter(|| {
            async {
                // Your benchmark code here
                my_function().await
            }
        })
    });
}

criterion_group!(benches, bench_my_function);
criterion_main!(benches);
```

## Performance Tips

Based on benchmark results, focus optimization efforts on:

1. **Hot paths**: Functions called frequently (e.g., message routing, cache lookups)
2. **I/O bottlenecks**: Database queries, network calls
3. **Memory allocations**: Reduce allocations in hot paths
4. **Lock contention**: Use concurrent data structures where appropriate
5. **Batch operations**: Aggregate operations when possible
