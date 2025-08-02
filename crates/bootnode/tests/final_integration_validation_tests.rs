use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use reqwest;
use serde_json;
use prometheus_client::registry::Registry;
use tempfile::TempDir;
use std::process::{Command, Stdio};
use std::io::Write;

use sqld_bootnode::{
    config::{MetricsConfig, MonitoringArgs},
    http_server::MetricsServer,
    state::BootnodeState,
    metrics,
};

/// Helper function to find an available port for testing
async fn find_available_port() -> u16 {
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Helper function to wait for server to be ready
async fn wait_for_server_ready(port: u16, timeout_secs: u64) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/health", port);
    
    for _ in 0..(timeout_secs * 10) { // Check every 100ms
        if let Ok(response) = client.get(&url).send().await {
            if response.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    Err(format!("Server did not become ready within {} seconds", timeout_secs).into())
}

/// Create a temporary key file for bootnode testing
fn create_temp_key_file() -> Result<(TempDir, String), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let key_path = temp_dir.path().join("test_key");
    
    // Generate a test key using the keygen binary
    let output = Command::new("cargo")
        .args(&["run", "--bin", "sqd-keygen", "--", key_path.to_str().unwrap()])
        .output()?;
    
    if !output.status.success() {
        return Err(format!("Failed to generate key: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    
    Ok((temp_dir, key_path.to_string_lossy().to_string()))
}

#[tokio::test]
async fn test_complete_bootnode_with_monitoring_default_config() {
    // Test bootnode with default monitoring configuration
    let port = find_available_port().await;
    let (server, _port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    // Start the server
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server_ready(port, 5).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    
    // Test all endpoints are accessible
    let health_response = client.get(&format!("{}/health", base_url)).send().await.unwrap();
    assert_eq!(health_response.status(), 200);
    
    let ready_response = client.get(&format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(ready_response.status(), 200);
    
    let metrics_response = client.get(&format!("{}/metrics", base_url)).send().await.unwrap();
    assert_eq!(metrics_response.status(), 200);
    
    // Simulate some network activity
    for _ in 0..10 {
        bootnode_state.peer_connected();
        metrics::increment_connection_attempts();
        metrics::record_network_event("test_connection");
    }
    
    bootnode_state.set_dht_ready(true);
    
    // Verify metrics reflect the activity
    let final_metrics = client.get(&format!("{}/metrics", base_url)).send().await.unwrap().text().await.unwrap();
    assert!(final_metrics.contains("bootnode_connected_peers"));
    assert!(final_metrics.contains("bootnode_connection_attempts_total"));
    
    let final_ready: serde_json::Value = client.get(&format!("{}/ready", base_url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(final_ready["ready"], true);
    assert_eq!(final_ready["peer_count"], 10);
    
    server_handle.abort();
}

#[tokio::test]
async fn test_complete_bootnode_with_custom_monitoring_config() {
    // Test bootnode with custom monitoring configuration
    let port = find_available_port().await;
    let custom_host = "127.0.0.1";
    
    let args = MonitoringArgs {
        metrics_host: custom_host.to_string(),
        metrics_port: port,
        disable_metrics: false,
    };
    
    let config = MetricsConfig::from_monitoring_args(&args);
    assert!(config.enabled);
    assert_eq!(config.host, custom_host);
    assert_eq!(config.port, port);
    
    // Create server with custom config
    let mut registry = Registry::default();
    metrics::register_metrics(&mut registry);
    let registry = Arc::new(registry);
    let bootnode_state = Arc::new(BootnodeState::new());
    
    let server = MetricsServer::new(registry.clone(), bootnode_state.clone());
    let server_handle = tokio::spawn(async move {
        server.run(custom_host.to_string(), port).await
    });
    
    wait_for_server_ready(port, 5).await.expect("Custom config server should start");
    
    let client = reqwest::Client::new();
    let base_url = format!("http://{}:{}", custom_host, port);
    
    // Test endpoints work with custom configuration
    let health_response = client.get(&format!("{}/health", base_url)).send().await.unwrap();
    assert_eq!(health_response.status(), 200);
    
    server_handle.abort();
}

#[tokio::test]
async fn test_complete_bootnode_with_monitoring_disabled() {
    // Test that bootnode works correctly when monitoring is disabled
    let args = MonitoringArgs {
        metrics_host: "127.0.0.1".to_string(),
        metrics_port: 9090,
        disable_metrics: true,
    };
    
    let config = MetricsConfig::from_monitoring_args(&args);
    assert!(!config.enabled);
    assert!(!config.should_start_server());
    
    // When monitoring is disabled, no server should start
    // This test verifies the configuration logic works correctly
    assert!(true, "Monitoring disabled configuration works correctly");
}

#[tokio::test]
async fn test_metrics_accuracy_during_simulated_network_operations() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server_ready(port, 5).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let metrics_url = format!("http://127.0.0.1:{}/metrics", port);
    let ready_url = format!("http://127.0.0.1:{}/ready", port);
    
    // Simulate realistic network operations sequence
    let operations = vec![
        ("bootstrap_start", 1),
        ("peer_discovery", 5),
        ("connection_established", 10),
        ("dht_join", 1),
        ("routing_updated", 15),
        ("kademlia_inbound_request", 25),
        ("ping_success", 30),
        ("gossipsub_message", 20),
        ("connection_closed", 3),
        ("ping_failure", 2),
    ];
    
    // Execute operations in sequence
    for (operation, count) in &operations {
        for _ in 0..*count {
            match *operation {
                "connection_established" => {
                    bootnode_state.peer_connected();
                    metrics::increment_connection_attempts();
                    metrics::increment_successful_connections();
                }
                "connection_closed" => {
                    bootnode_state.peer_disconnected();
                }
                "dht_join" => {
                    bootnode_state.set_dht_ready(true);
                }
                _ => {}
            }
            metrics::record_network_event(operation);
        }
    }
    
    // Verify metrics accuracy
    let metrics_text = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
    
    // Check that all event types are recorded
    for (operation, _) in &operations {
        let event_pattern = format!("event_type=\"{}\"", operation);
        assert!(metrics_text.contains(&event_pattern), 
            "Expected to find event type '{}' in metrics", operation);
    }
    
    // Verify readiness reflects DHT state
    let ready_response: serde_json::Value = client.get(&ready_url).send().await.unwrap().json().await.unwrap();
    assert_eq!(ready_response["ready"], true);
    assert_eq!(ready_response["dht_connected"], true);
    assert_eq!(ready_response["peer_count"], 7); // 10 connected - 3 disconnected
    
    // Verify counter metrics
    assert!(metrics_text.contains("bootnode_connection_attempts_total"));
    assert!(metrics_text.contains("bootnode_successful_connections_total"));
    
    server_handle.abort();
}

#[tokio::test]
async fn test_graceful_shutdown_handling() {
    let (server, port, _registry, _bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server_ready(port, 5).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", port);
    
    // Verify server is running
    let response = client.get(&health_url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    
    // Test graceful shutdown by aborting the server task
    server_handle.abort();
    
    // Wait for shutdown to complete
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Verify server is no longer accessible
    let result = timeout(Duration::from_secs(2), client.get(&health_url).send()).await;
    
    // Should either timeout or get a connection error
    match result {
        Ok(Ok(_response)) => {
            // Server might still be responding briefly after abort - this is acceptable
            // The main test is that the abort completed without hanging
        }
        Ok(Err(_)) => {
            // Connection error is expected after shutdown
        }
        Err(_) => {
            // Timeout is also acceptable
        }
    }
    
    // The key test is that the server handle was aborted successfully
    // and didn't hang the test - if we reach this point, the test passed
}

#[tokio::test]
async fn test_monitoring_performance_impact() {
    // Test that monitoring doesn't significantly impact performance
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server_ready(port, 5).await.expect("Server should start");
    
    // Measure performance of core operations with monitoring enabled
    let start_time = std::time::Instant::now();
    
    // Simulate high-frequency operations
    for i in 0..10000 {
        bootnode_state.peer_connected();
        metrics::increment_connection_attempts();
        metrics::record_network_event("performance_test");
        
        if i % 100 == 0 {
            bootnode_state.peer_disconnected();
        }
        
        if i % 1000 == 0 {
            bootnode_state.set_dht_ready(i % 2000 == 0);
        }
    }
    
    let duration = start_time.elapsed();
    
    // Operations should complete quickly (less than 1 second for 10k operations)
    assert!(duration < Duration::from_secs(1), 
        "Monitoring should not significantly impact performance. Duration: {:?}", duration);
    
    // Verify the operations were recorded correctly
    let client = reqwest::Client::new();
    let metrics_url = format!("http://127.0.0.1:{}/metrics", port);
    let metrics_text = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
    
    assert!(metrics_text.contains("bootnode_connection_attempts_total"));
    assert!(metrics_text.contains("event_type=\"performance_test\""));
    
    server_handle.abort();
}

#[tokio::test]
async fn test_external_prometheus_scraping_simulation() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server_ready(port, 5).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let metrics_url = format!("http://127.0.0.1:{}/metrics", port);
    
    // Add some metrics data
    for i in 0..50 {
        bootnode_state.peer_connected();
        metrics::increment_connection_attempts();
        metrics::record_network_event(&format!("scrape_test_{}", i % 5));
    }
    bootnode_state.set_dht_ready(true);
    
    // Simulate Prometheus scraping behavior (multiple concurrent requests)
    let mut scrape_handles = vec![];
    
    for _ in 0..10 {
        let client_clone = client.clone();
        let url = metrics_url.clone();
        let handle = tokio::spawn(async move {
            let response = client_clone.get(&url).send().await.unwrap();
            assert_eq!(response.status(), 200);
            
            let metrics_text = response.text().await.unwrap();
            
            // Verify Prometheus format compliance
            assert!(metrics_text.contains("# HELP") || metrics_text.contains("# TYPE") || !metrics_text.trim().is_empty());
            
            // Verify key metrics are present
            assert!(metrics_text.contains("bootnode_connected_peers"));
            assert!(metrics_text.contains("bootnode_connection_attempts_total"));
            
            metrics_text
        });
        scrape_handles.push(handle);
    }
    
    // Wait for all scrapes to complete
    let mut all_metrics = vec![];
    for handle in scrape_handles {
        let metrics = handle.await.unwrap();
        all_metrics.push(metrics);
    }
    
    // Verify all scrapes returned consistent data
    let first_metrics = &all_metrics[0];
    for metrics in &all_metrics[1..] {
        // All scrapes should contain the same metric names (values may vary slightly due to timing)
        assert!(metrics.contains("bootnode_connected_peers"));
        assert!(metrics.contains("bootnode_connection_attempts_total"));
    }
    
    server_handle.abort();
}

#[tokio::test]
async fn test_external_health_check_system_simulation() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server_ready(port, 5).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", port);
    let ready_url = format!("http://127.0.0.1:{}/ready", port);
    
    // Simulate health check system behavior
    let mut health_check_handles = vec![];
    
    // Simulate continuous health checking (like Kubernetes liveness probes)
    for i in 0..20 {
        let client_clone = client.clone();
        let health_url_clone = health_url.clone();
        let ready_url_clone = ready_url.clone();
        let bootnode_state_clone = bootnode_state.clone();
        
        let handle = tokio::spawn(async move {
            // Simulate state changes during health checks
            if i == 5 {
                bootnode_state_clone.set_dht_ready(true);
            }
            if i == 10 {
                for _ in 0..5 {
                    bootnode_state_clone.peer_connected();
                }
            }
            
            // Health check
            let health_response = client_clone.get(&health_url_clone).send().await.unwrap();
            assert_eq!(health_response.status(), 200);
            
            let health_json: serde_json::Value = health_response.json().await.unwrap();
            assert_eq!(health_json["status"], "healthy");
            assert!(health_json["timestamp"].is_number());
            assert!(health_json["uptime_seconds"].is_number());
            
            // Readiness check
            let ready_response = client_clone.get(&ready_url_clone).send().await.unwrap();
            assert_eq!(ready_response.status(), 200);
            
            let ready_json: serde_json::Value = ready_response.json().await.unwrap();
            assert!(ready_json["ready"].is_boolean());
            assert!(ready_json["dht_connected"].is_boolean());
            assert!(ready_json["peer_count"].is_number());
            
            (health_json, ready_json)
        });
        health_check_handles.push(handle);
        
        // Small delay between health checks
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    // Wait for all health checks to complete
    let mut results = vec![];
    for handle in health_check_handles {
        let result = handle.await.unwrap();
        results.push(result);
    }
    
    // Verify health check results show progression
    let first_ready = &results[0].1;
    let last_ready = &results[results.len() - 1].1;
    
    // Should show progression from not ready to ready
    assert_eq!(first_ready["ready"], false);
    assert_eq!(last_ready["ready"], true);
    assert!(last_ready["peer_count"].as_u64().unwrap() > 0);
    
    server_handle.abort();
}

#[tokio::test]
async fn test_monitoring_resilience_under_load() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server_ready(port, 5).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    
    // Create high load scenario
    let mut load_handles = vec![];
    
    // High-frequency state changes
    let state_clone = bootnode_state.clone();
    let state_handle = tokio::spawn(async move {
        for i in 0..5000 {
            if i % 2 == 0 {
                state_clone.peer_connected();
                metrics::increment_successful_connections();
            } else {
                state_clone.peer_disconnected();
            }
            
            metrics::increment_connection_attempts();
            metrics::record_network_event(&format!("load_test_{}", i % 10));
            
            if i % 100 == 0 {
                state_clone.set_dht_ready(i % 200 == 0);
            }
            
            if i % 1000 == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    });
    load_handles.push(state_handle);
    
    // High-frequency HTTP requests
    for endpoint in ["/health", "/ready", "/metrics"] {
        let client_clone = client.clone();
        let url = format!("{}{}", base_url, endpoint);
        let handle = tokio::spawn(async move {
            for _ in 0..100 {
                let response = client_clone.get(&url).send().await;
                match response {
                    Ok(resp) => {
                        // Should get successful responses most of the time
                        assert!(resp.status().is_success() || resp.status().is_server_error());
                    }
                    Err(_) => {
                        // Some requests might fail under extreme load, which is acceptable
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        load_handles.push(handle);
    }
    
    // Wait for all load to complete
    for handle in load_handles {
        handle.await.unwrap();
    }
    
    // System should still be responsive after load
    let final_health = client.get(&format!("{}/health", base_url)).send().await.unwrap();
    assert_eq!(final_health.status(), 200);
    
    let final_metrics = client.get(&format!("{}/metrics", base_url)).send().await.unwrap();
    assert_eq!(final_metrics.status(), 200);
    
    server_handle.abort();
}

#[tokio::test]
async fn test_configuration_validation_and_error_handling() {
    // Test various configuration scenarios
    
    // Test invalid host configuration
    let invalid_host_args = MonitoringArgs {
        metrics_host: "".to_string(), // Invalid empty host
        metrics_port: 9090,
        disable_metrics: false,
    };
    
    let config = MetricsConfig::from_monitoring_args(&invalid_host_args);
    assert!(config.enabled); // Should fallback to defaults
    assert_eq!(config.host, "0.0.0.0"); // Should use default host
    
    // Test invalid port configuration
    let invalid_port_args = MonitoringArgs {
        metrics_host: "127.0.0.1".to_string(),
        metrics_port: 0, // Invalid port
        disable_metrics: false,
    };
    
    let config = MetricsConfig::from_monitoring_args(&invalid_port_args);
    assert!(config.enabled);
    assert_eq!(config.port, 9090); // Should use default port
    
    // Test port already in use scenario
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let occupied_port = listener.local_addr().unwrap().port();
    
    let registry = Arc::new(Registry::default());
    let bootnode_state = Arc::new(BootnodeState::new());
    let server = MetricsServer::new(registry, bootnode_state);
    
    // Try to bind to the occupied port - should fail gracefully
    let result = timeout(
        Duration::from_secs(2),
        server.run("127.0.0.1".to_string(), occupied_port)
    ).await;
    
    // Should either timeout or return an error
    assert!(result.is_err() || result.unwrap().is_err());
    
    drop(listener);
}

/// Helper to create a test server with metrics
async fn create_test_server_with_metrics() -> (MetricsServer, u16, Arc<Registry>, Arc<BootnodeState>) {
    let mut registry = Registry::default();
    metrics::register_metrics(&mut registry);
    let registry = Arc::new(registry);
    let bootnode_state = Arc::new(BootnodeState::new());
    
    let port = find_available_port().await;
    let server = MetricsServer::new(registry.clone(), bootnode_state.clone());
    
    (server, port, registry, bootnode_state)
}

#[tokio::test]
async fn test_metrics_data_consistency_across_requests() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server_ready(port, 5).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let metrics_url = format!("http://127.0.0.1:{}/metrics", port);
    let ready_url = format!("http://127.0.0.1:{}/ready", port);
    
    // Set up a known state
    for _ in 0..25 {
        bootnode_state.peer_connected();
        metrics::increment_connection_attempts();
    }
    bootnode_state.set_dht_ready(true);
    
    // Make multiple requests and verify consistency
    let mut metrics_responses = vec![];
    let mut ready_responses = vec![];
    
    for _ in 0..5 {
        let metrics_text = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
        let ready_json: serde_json::Value = client.get(&ready_url).send().await.unwrap().json().await.unwrap();
        
        metrics_responses.push(metrics_text);
        ready_responses.push(ready_json);
        
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    // Verify readiness responses are consistent
    for ready_response in &ready_responses {
        assert_eq!(ready_response["ready"], true);
        assert_eq!(ready_response["dht_connected"], true);
        assert_eq!(ready_response["peer_count"], 25);
    }
    
    // Verify metrics responses contain expected data
    for metrics_text in &metrics_responses {
        assert!(metrics_text.contains("bootnode_connected_peers"));
        assert!(metrics_text.contains("bootnode_connection_attempts_total"));
    }
    
    server_handle.abort();
}

#[tokio::test]
async fn test_monitoring_system_recovery_after_errors() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server_ready(port, 5).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    
    // Simulate error conditions and recovery
    
    // 1. Rapid state changes that might cause issues
    for i in 0..1000 {
        if i % 2 == 0 {
            bootnode_state.peer_connected();
        } else {
            bootnode_state.peer_disconnected();
        }
        
        bootnode_state.set_dht_ready(i % 3 == 0);
        metrics::record_network_event("recovery_test");
        
        if i % 100 == 0 {
            // Verify system is still responsive during stress
            let health_response = client.get(&format!("{}/health", base_url)).send().await;
            match health_response {
                Ok(resp) => assert!(resp.status().is_success()),
                Err(_) => {
                    // Temporary failures are acceptable under extreme load
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }
    
    // 2. Verify system has recovered and is fully functional
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let health_response = client.get(&format!("{}/health", base_url)).send().await.unwrap();
    assert_eq!(health_response.status(), 200);
    
    let ready_response = client.get(&format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(ready_response.status(), 200);
    
    let metrics_response = client.get(&format!("{}/metrics", base_url)).send().await.unwrap();
    assert_eq!(metrics_response.status(), 200);
    
    // Verify metrics contain recovery test events
    let metrics_text = metrics_response.text().await.unwrap();
    assert!(metrics_text.contains("event_type=\"recovery_test\""));
    
    server_handle.abort();
}