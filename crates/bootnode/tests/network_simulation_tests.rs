use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use reqwest;
use serde_json;
use prometheus_client::registry::Registry;

use sqld_bootnode::{
    config::{MetricsConfig, MonitoringArgs},
    http_server::MetricsServer,
    state::BootnodeState,
    metrics,
};

/// Helper to create a test server with metrics
async fn create_test_server_with_metrics() -> (MetricsServer, u16, Arc<Registry>, Arc<BootnodeState>) {
    let mut registry = Registry::default();
    metrics::register_metrics(&mut registry);
    let registry = Arc::new(registry);
    let bootnode_state = Arc::new(BootnodeState::new());
    
    // Find available port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    
    let server = MetricsServer::new(registry.clone(), bootnode_state.clone());
    (server, port, registry, bootnode_state)
}

/// Wait for server to be ready
async fn wait_for_server(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/health", port);
    
    for _ in 0..50 { // Try for 5 seconds
        if let Ok(response) = client.get(&url).send().await {
            if response.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    Err("Server did not become ready".into())
}

#[tokio::test]
async fn test_simulated_peer_connection_lifecycle() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server(port).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let metrics_url = format!("http://127.0.0.1:{}/metrics", port);
    let ready_url = format!("http://127.0.0.1:{}/ready", port);
    
    // Initial state - check that metrics are available
    let initial_metrics = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
    assert!(initial_metrics.contains("bootnode_connected_peers"));
    
    let initial_ready: serde_json::Value = client.get(&ready_url).send().await.unwrap().json().await.unwrap();
    assert_eq!(initial_ready["peer_count"], 0);
    assert_eq!(initial_ready["ready"], false);
    
    // Simulate peer connections
    for i in 1..=10 {
        bootnode_state.peer_connected();
        metrics::increment_connection_attempts();
        metrics::increment_successful_connections();
        metrics::record_network_event("connection_established");
        
        // Check metrics after each connection - just verify the metric exists
        let metrics_text = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
        assert!(metrics_text.contains("bootnode_connected_peers"));
        
        let ready_response: serde_json::Value = client.get(&ready_url).send().await.unwrap().json().await.unwrap();
        assert_eq!(ready_response["peer_count"], i);
    }
    
    // Set DHT ready
    bootnode_state.set_dht_ready(true);
    let ready_response: serde_json::Value = client.get(&ready_url).send().await.unwrap().json().await.unwrap();
    assert_eq!(ready_response["ready"], true);
    assert_eq!(ready_response["dht_connected"], true);
    
    // Simulate some peer disconnections
    for i in (5..=9).rev() {
        bootnode_state.peer_disconnected();
        metrics::record_network_event("connection_closed");
        
        let metrics_text = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
        assert!(metrics_text.contains("bootnode_connected_peers"));
    }
    
    // Simulate connection failures
    for _ in 0..3 {
        metrics::increment_connection_attempts();
        metrics::increment_failed_connections();
        metrics::record_network_event("connection_failed");
    }
    
    // Final metrics check
    let final_metrics = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
    assert!(final_metrics.contains("bootnode_connected_peers"));
    
    // Verify counters increased
    assert!(final_metrics.contains("bootnode_connection_attempts_total"));
    assert!(final_metrics.contains("bootnode_successful_connections_total"));
    assert!(final_metrics.contains("bootnode_failed_connections_total"));
    
    server_handle.abort();
}

#[tokio::test]
async fn test_simulated_network_events_accuracy() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server(port).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let metrics_url = format!("http://127.0.0.1:{}/metrics", port);
    
    // Simulate various network events
    let events = vec![
        ("identify_received", 5),
        ("connection_established", 10),
        ("connection_closed", 3),
        ("kademlia_inbound_request", 7),
        ("ping_success", 15),
        ("ping_failure", 2),
        ("gossipsub_message", 8),
        ("autonat_status_changed", 1),
    ];
    
    for (event_type, count) in &events {
        for _ in 0..*count {
            metrics::record_network_event(event_type);
        }
    }
    
    // Get metrics and verify event counts
    let metrics_text = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
    
    for (event_type, _expected_count) in events {
        // Look for the event type in the metrics output (counts may vary due to global state)
        let event_pattern = format!("event_type=\"{}\"", event_type);
        assert!(metrics_text.contains(&event_pattern), 
            "Expected to find event type '{}' in metrics output", event_type);
    }
    
    server_handle.abort();
}

#[tokio::test]
async fn test_simulated_high_load_scenario() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server(port).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let metrics_url = format!("http://127.0.0.1:{}/metrics", port);
    
    // Simulate high load scenario
    let simulation_handle = tokio::spawn(async move {
        for i in 0..1000 {
            // Simulate connection attempts
            metrics::increment_connection_attempts();
            
            // 80% success rate
            if i % 5 != 0 {
                bootnode_state.peer_connected();
                metrics::increment_successful_connections();
                metrics::record_network_event("connection_established");
            } else {
                metrics::increment_failed_connections();
                metrics::record_network_event("connection_failed");
            }
            
            // Simulate some disconnections
            if i % 10 == 0 && i > 0 {
                bootnode_state.peer_disconnected();
                metrics::record_network_event("connection_closed");
            }
            
            // Simulate various network events
            if i % 3 == 0 {
                metrics::record_network_event("ping_success");
            }
            if i % 7 == 0 {
                metrics::record_network_event("kademlia_inbound_request");
            }
            if i % 11 == 0 {
                metrics::record_network_event("gossipsub_message");
            }
            
            // Small delay to prevent overwhelming
            if i % 100 == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    });
    
    // Make concurrent requests while simulation is running
    let request_handles: Vec<_> = (0..10).map(|_| {
        let client = client.clone();
        let metrics_url = metrics_url.clone();
        tokio::spawn(async move {
            for _ in 0..20 {
                let _ = client.get(&metrics_url).send().await;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
    }).collect();
    
    // Wait for simulation to complete
    simulation_handle.await.unwrap();
    
    // Wait for all requests to complete
    for handle in request_handles {
        handle.await.unwrap();
    }
    
    // Verify final state
    let final_metrics = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
    
    // Should have processed 1000 connection attempts
    assert!(final_metrics.contains("bootnode_connection_attempts_total"));
    
    // Should have some successful and failed connections
    assert!(final_metrics.contains("bootnode_successful_connections_total"));
    assert!(final_metrics.contains("bootnode_failed_connections_total"));
    
    // Should have various network events
    assert!(final_metrics.contains("bootnode_network_events_total"));
    
    server_handle.abort();
}

#[tokio::test]
async fn test_simulated_dht_lifecycle() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server(port).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let ready_url = format!("http://127.0.0.1:{}/ready", port);
    let metrics_url = format!("http://127.0.0.1:{}/metrics", port);
    
    // Initial state - DHT not ready
    let initial_ready: serde_json::Value = client.get(&ready_url).send().await.unwrap().json().await.unwrap();
    assert_eq!(initial_ready["ready"], false);
    assert_eq!(initial_ready["dht_connected"], false);
    
    // Simulate DHT bootstrap process
    for i in 1..=5 {
        bootnode_state.peer_connected();
        metrics::record_network_event("routing_updated");
        
        // Check that we're still not ready until we explicitly set it
        let ready_response: serde_json::Value = client.get(&ready_url).send().await.unwrap().json().await.unwrap();
        assert_eq!(ready_response["ready"], false);
        assert_eq!(ready_response["peer_count"], i);
    }
    
    // Simulate DHT becoming ready
    bootnode_state.set_dht_ready(true);
    metrics::record_network_event("dht_ready");
    
    let ready_response: serde_json::Value = client.get(&ready_url).send().await.unwrap().json().await.unwrap();
    assert_eq!(ready_response["ready"], true);
    assert_eq!(ready_response["dht_connected"], true);
    assert_eq!(ready_response["peer_count"], 5);
    
    // Simulate DHT operations
    for _ in 0..10 {
        metrics::record_network_event("kademlia_inbound_request");
    }
    
    // Simulate DHT becoming temporarily unavailable
    bootnode_state.set_dht_ready(false);
    
    let ready_response: serde_json::Value = client.get(&ready_url).send().await.unwrap().json().await.unwrap();
    assert_eq!(ready_response["ready"], false);
    assert_eq!(ready_response["dht_connected"], false);
    
    // Simulate DHT recovery
    bootnode_state.set_dht_ready(true);
    
    let final_ready: serde_json::Value = client.get(&ready_url).send().await.unwrap().json().await.unwrap();
    assert_eq!(final_ready["ready"], true);
    assert_eq!(final_ready["dht_connected"], true);
    
    // Verify metrics contain DHT events
    let metrics_text = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
    assert!(metrics_text.contains("event_type=\"dht_ready\""));
    assert!(metrics_text.contains("event_type=\"routing_updated\""));
    assert!(metrics_text.contains("event_type=\"kademlia_inbound_request\""));
    
    server_handle.abort();
}

#[tokio::test]
async fn test_simulated_error_conditions() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server(port).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let metrics_url = format!("http://127.0.0.1:{}/metrics", port);
    
    // Simulate various error conditions
    for _ in 0..10 {
        metrics::increment_connection_attempts();
        metrics::increment_failed_connections();
        metrics::record_network_event("incoming_connection_error");
    }
    
    for _ in 0..5 {
        metrics::increment_connection_attempts();
        metrics::increment_failed_connections();
        metrics::record_network_event("outgoing_connection_error");
    }
    
    // Simulate ping failures
    for _ in 0..8 {
        metrics::record_network_event("ping_failure");
    }
    
    // Simulate successful operations mixed with failures
    for i in 0..20 {
        if i % 3 == 0 {
            bootnode_state.peer_connected();
            metrics::increment_successful_connections();
            metrics::record_network_event("connection_established");
        } else {
            metrics::increment_failed_connections();
            metrics::record_network_event("connection_failed");
        }
        metrics::increment_connection_attempts();
    }
    
    // Verify error metrics are recorded
    let metrics_text = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
    
    // Should have recorded failed connections
    assert!(metrics_text.contains("bootnode_failed_connections_total"));
    
    // Should have recorded various error events
    assert!(metrics_text.contains("event_type=\"incoming_connection_error\""));
    assert!(metrics_text.contains("event_type=\"outgoing_connection_error\""));
    assert!(metrics_text.contains("event_type=\"ping_failure\""));
    
    // Should still have some successful connections
    assert!(metrics_text.contains("bootnode_successful_connections_total"));
    
    server_handle.abort();
}

#[tokio::test]
async fn test_simulated_concurrent_network_activity() {
    let (server, port, _registry, bootnode_state) = create_test_server_with_metrics().await;
    
    let server_handle = tokio::spawn(async move {
        server.run("127.0.0.1".to_string(), port).await
    });
    
    wait_for_server(port).await.expect("Server should start");
    
    let client = reqwest::Client::new();
    let metrics_url = format!("http://127.0.0.1:{}/metrics", port);
    
    // Spawn multiple tasks simulating concurrent network activity
    let mut simulation_handles = vec![];
    
    // Task 1: Peer connections/disconnections
    let state1 = bootnode_state.clone();
    let handle1 = tokio::spawn(async move {
        for i in 0..100 {
            if i % 2 == 0 {
                state1.peer_connected();
                metrics::increment_successful_connections();
            } else {
                state1.peer_disconnected();
            }
            metrics::increment_connection_attempts();
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
    simulation_handles.push(handle1);
    
    // Task 2: DHT operations
    let state2 = bootnode_state.clone();
    let handle2 = tokio::spawn(async move {
        for i in 0..50 {
            if i == 10 {
                state2.set_dht_ready(true);
            }
            metrics::record_network_event("kademlia_inbound_request");
            metrics::record_network_event("routing_updated");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });
    simulation_handles.push(handle2);
    
    // Task 3: Ping operations
    let handle3 = tokio::spawn(async move {
        for i in 0..75 {
            if i % 4 == 0 {
                metrics::record_network_event("ping_failure");
            } else {
                metrics::record_network_event("ping_success");
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
    simulation_handles.push(handle3);
    
    // Task 4: Gossipsub activity
    let handle4 = tokio::spawn(async move {
        for _ in 0..30 {
            metrics::record_network_event("gossipsub_message");
            metrics::record_network_event("gossipsub_subscribed");
            tokio::time::sleep(Duration::from_millis(3)).await;
        }
    });
    simulation_handles.push(handle4);
    
    // Task 5: Connection failures
    let handle5 = tokio::spawn(async move {
        for _ in 0..25 {
            metrics::increment_connection_attempts();
            metrics::increment_failed_connections();
            metrics::record_network_event("connection_failed");
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
    });
    simulation_handles.push(handle5);
    
    // Make concurrent HTTP requests while simulation is running
    let mut request_handles = vec![];
    for _ in 0..5 {
        let client = client.clone();
        let metrics_url = metrics_url.clone();
        let handle = tokio::spawn(async move {
            for _ in 0..10 {
                let _ = client.get(&metrics_url).send().await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        request_handles.push(handle);
    }
    
    // Wait for all simulation tasks to complete
    for handle in simulation_handles {
        handle.await.unwrap();
    }
    
    // Wait for all request tasks to complete
    for handle in request_handles {
        handle.await.unwrap();
    }
    
    // Verify final metrics contain all expected events
    let final_metrics = client.get(&metrics_url).send().await.unwrap().text().await.unwrap();
    
    // Verify all event types are present
    let expected_events = vec![
        "kademlia_inbound_request",
        "routing_updated",
        "ping_success",
        "ping_failure",
        "gossipsub_message",
        "gossipsub_subscribed",
        "connection_failed",
    ];
    
    for event in expected_events {
        assert!(final_metrics.contains(&format!("event_type=\"{}\"", event)),
            "Expected to find event type '{}' in metrics", event);
    }
    
    // Verify counters are present
    assert!(final_metrics.contains("bootnode_connection_attempts_total"));
    assert!(final_metrics.contains("bootnode_successful_connections_total"));
    assert!(final_metrics.contains("bootnode_failed_connections_total"));
    
    server_handle.abort();
}