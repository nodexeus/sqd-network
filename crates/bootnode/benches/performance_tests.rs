use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::sync::Arc;

use tokio::runtime::Runtime;
use prometheus_client::registry::Registry;

use sqld_bootnode::{
    config::{MetricsConfig, MonitoringArgs},
    http_server::MetricsServer,
    state::BootnodeState,
    metrics,
};

fn bench_metrics_collection_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("metrics_collection");
    
    // Benchmark individual metric operations
    group.bench_function("update_connected_peers", |b| {
        b.iter(|| {
            metrics::update_connected_peers(black_box(42));
        })
    });
    
    group.bench_function("increment_connection_attempts", |b| {
        b.iter(|| {
            metrics::increment_connection_attempts();
        })
    });
    
    group.bench_function("record_network_event", |b| {
        b.iter(|| {
            metrics::record_network_event(black_box("test_event"));
        })
    });
    
    group.bench_function("update_uptime", |b| {
        let start_time = std::time::Instant::now();
        b.iter(|| {
            metrics::update_uptime(black_box(start_time));
        })
    });
    
    group.finish();
}

fn bench_state_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_operations");
    
    let state = Arc::new(BootnodeState::new());
    
    group.bench_function("peer_connected", |b| {
        let state = state.clone();
        b.iter(|| {
            state.peer_connected();
        })
    });
    
    group.bench_function("peer_disconnected", |b| {
        let state = state.clone();
        b.iter(|| {
            state.peer_disconnected();
        })
    });
    
    group.bench_function("set_dht_ready", |b| {
        let state = state.clone();
        b.iter(|| {
            state.set_dht_ready(black_box(true));
        })
    });
    
    group.bench_function("get_peer_count", |b| {
        let state = state.clone();
        b.iter(|| {
            black_box(state.get_peer_count());
        })
    });
    
    group.bench_function("is_dht_ready", |b| {
        let state = state.clone();
        b.iter(|| {
            black_box(state.is_dht_ready());
        })
    });
    
    group.bench_function("uptime", |b| {
        let state = state.clone();
        b.iter(|| {
            black_box(state.uptime());
        })
    });
    
    group.finish();
}

fn bench_concurrent_state_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_state");
    
    for thread_count in [1, 2, 4, 8].iter() {
        group.bench_with_input(
            BenchmarkId::new("peer_operations", thread_count),
            thread_count,
            |b, &thread_count| {
                let state = Arc::new(BootnodeState::new());
                
                b.iter(|| {
                    let rt = Runtime::new().unwrap();
                    let state = state.clone();
                    rt.block_on(async move {
                        let mut handles = vec![];
                        
                        for _ in 0..thread_count {
                            let state_clone = state.clone();
                            let handle = tokio::spawn(async move {
                                for _ in 0..10 { // Reduced iterations for benchmark
                                    state_clone.peer_connected();
                                    state_clone.peer_disconnected();
                                    state_clone.set_dht_ready(true);
                                    black_box(state_clone.get_peer_count());
                                }
                            });
                            handles.push(handle);
                        }
                        
                        for handle in handles {
                            handle.await.unwrap();
                        }
                    })
                });
            },
        );
    }
    
    group.finish();
}

fn bench_metrics_registry_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("metrics_registry");
    
    group.bench_function("register_metrics", |b| {
        b.iter(|| {
            let mut registry = Registry::default();
            metrics::register_metrics(&mut registry);
            black_box(registry);
        })
    });
    
    group.bench_function("encode_metrics", |b| {
        let mut registry = Registry::default();
        metrics::register_metrics(&mut registry);
        
        // Add some data to metrics
        metrics::update_connected_peers(10);
        metrics::increment_connection_attempts();
        metrics::record_network_event("test_event");
        
        b.iter(|| {
            let mut buffer = String::new();
            prometheus_client::encoding::text::encode(&mut buffer, &registry).unwrap();
            black_box(buffer);
        })
    });
    
    group.finish();
}

fn bench_config_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("config_operations");
    
    let args = MonitoringArgs {
        metrics_host: "127.0.0.1".to_string(),
        metrics_port: 9090,
        disable_metrics: false,
    };
    
    group.bench_function("config_from_args", |b| {
        b.iter(|| {
            let config = MetricsConfig::from_monitoring_args(black_box(&args));
            black_box(config);
        })
    });
    
    group.bench_function("config_validation", |b| {
        let config = MetricsConfig::from_monitoring_args(&args);
        b.iter(|| {
            black_box(config.is_valid());
        })
    });
    
    group.bench_function("socket_addr_creation", |b| {
        let config = MetricsConfig::from_monitoring_args(&args);
        b.iter(|| {
            let addr = config.socket_addr().unwrap();
            black_box(addr);
        })
    });
    
    group.finish();
}

fn bench_http_server_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("http_server");
    
    group.bench_function("server_creation", |b| {
        let registry = Arc::new(Registry::default());
        let bootnode_state = Arc::new(BootnodeState::new());
        
        b.iter(|| {
            let server = MetricsServer::new(
                black_box(registry.clone()),
                black_box(bootnode_state.clone())
            );
            black_box(server);
        })
    });
    
    group.finish();
}

fn bench_high_volume_metrics(c: &mut Criterion) {
    let mut group = c.benchmark_group("high_volume_metrics");
    
    // Benchmark handling high volumes of metric updates
    for event_count in [100, 1000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::new("bulk_metric_updates", event_count),
            event_count,
            |b, &event_count| {
                b.iter(|| {
                    for i in 0..event_count {
                        metrics::update_connected_peers(i as i64);
                        metrics::increment_connection_attempts();
                        metrics::record_network_event("bulk_test_event");
                        
                        if i % 100 == 0 {
                            metrics::increment_successful_connections();
                        }
                        if i % 200 == 0 {
                            metrics::increment_failed_connections();
                        }
                    }
                })
            },
        );
    }
    
    group.finish();
}

fn bench_memory_usage_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_usage");
    
    group.bench_function("state_creation_destruction", |b| {
        b.iter(|| {
            let states: Vec<_> = (0..1000).map(|_| {
                Arc::new(BootnodeState::new())
            }).collect();
            
            for state in &states {
                state.peer_connected();
                state.set_dht_ready(true);
            }
            
            black_box(states);
        })
    });
    
    group.bench_function("config_creation_destruction", |b| {
        b.iter(|| {
            let configs: Vec<_> = (0..1000).map(|i| {
                let args = MonitoringArgs {
                    metrics_host: format!("host{}.example.com", i),
                    metrics_port: 8000 + (i % 1000) as u16,
                    disable_metrics: i % 2 == 0,
                };
                MetricsConfig::from_monitoring_args(&args)
            }).collect();
            
            for config in &configs {
                black_box(config.is_valid());
            }
            
            black_box(configs);
        })
    });
    
    group.finish();
}

fn bench_error_handling_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_handling");
    
    let state = Arc::new(BootnodeState::new());
    
    group.bench_function("state_operations_with_error_handling", |b| {
        let state = state.clone();
        b.iter(|| {
            // These operations include panic protection
            state.peer_connected();
            state.peer_disconnected();
            state.set_dht_ready(black_box(true));
        })
    });
    
    group.bench_function("metrics_operations_with_error_handling", |b| {
        b.iter(|| {
            // These operations now include error handling
            metrics::update_connected_peers(black_box(42));
            metrics::increment_connection_attempts();
            metrics::record_network_event(black_box("test_event"));
        })
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_metrics_collection_overhead,
    bench_state_operations,
    bench_concurrent_state_operations,
    bench_metrics_registry_operations,
    bench_config_operations,
    bench_http_server_creation,
    bench_high_volume_metrics,
    bench_memory_usage_patterns,
    bench_error_handling_overhead
);

criterion_main!(benches);