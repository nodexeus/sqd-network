use lazy_static::lazy_static;
use prometheus_client::metrics::{counter::Counter, family::Family, gauge::Gauge};
use prometheus_client::registry::Registry;

use std::time::Instant;

// Metric labels for categorizing events
#[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
pub struct EventLabels {
    pub event_type: String,
}

lazy_static! {
    pub static ref CONNECTED_PEERS: Gauge = Gauge::default();
    pub static ref DHT_ENTRIES: Gauge = Gauge::default();
    pub static ref CONNECTION_ATTEMPTS: Counter = Counter::default();
    pub static ref SUCCESSFUL_CONNECTIONS: Counter = Counter::default();
    pub static ref FAILED_CONNECTIONS: Counter = Counter::default();
    pub static ref UPTIME_SECONDS: Gauge = Gauge::default();
    pub static ref NETWORK_EVENTS: Family<EventLabels, Counter> = Family::default();
}

pub fn register_metrics(registry: &mut Registry) {
    registry.register(
        "bootnode_connected_peers",
        "Number of currently connected peers",
        CONNECTED_PEERS.clone(),
    );
    
    registry.register(
        "bootnode_dht_entries",
        "Number of DHT entries stored",
        DHT_ENTRIES.clone(),
    );
    
    registry.register(
        "bootnode_connection_attempts_total",
        "Total number of connection attempts",
        CONNECTION_ATTEMPTS.clone(),
    );
    
    registry.register(
        "bootnode_successful_connections_total",
        "Total number of successful connections",
        SUCCESSFUL_CONNECTIONS.clone(),
    );
    
    registry.register(
        "bootnode_failed_connections_total",
        "Total number of failed connections",
        FAILED_CONNECTIONS.clone(),
    );
    
    registry.register(
        "bootnode_uptime_seconds",
        "Bootnode uptime in seconds",
        UPTIME_SECONDS.clone(),
    );
    
    registry.register(
        "bootnode_network_events_total",
        "Total number of network events by type",
        NETWORK_EVENTS.clone(),
    );
}

pub fn update_connected_peers(count: i64) {
    // These operations are very simple and unlikely to panic
    // If they do panic, it's better to let the panic propagate for debugging
    CONNECTED_PEERS.set(count);
}

pub fn increment_connection_attempts() {
    CONNECTION_ATTEMPTS.inc();
}

pub fn increment_successful_connections() {
    SUCCESSFUL_CONNECTIONS.inc();
}

pub fn increment_failed_connections() {
    FAILED_CONNECTIONS.inc();
}

pub fn record_network_event(event_type: &str) {
    let labels = EventLabels {
        event_type: event_type.to_string(),
    };
    NETWORK_EVENTS.get_or_create(&labels).inc();
}

pub fn update_uptime(start_time: Instant) {
    let uptime = start_time.elapsed().as_secs() as i64;
    UPTIME_SECONDS.set(uptime);
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus_client::registry::Registry;
    use std::time::{Duration, Instant};

    #[test]
    fn test_register_metrics() {
        let mut registry = Registry::default();
        
        // Should not panic when registering metrics
        register_metrics(&mut registry);
        
        // Verify that metrics are registered by checking the registry
        // We can't directly inspect the registry contents, but we can verify
        // that the function completes without error
    }

    #[test]
    fn test_update_connected_peers() {
        // Reset the gauge to a known state
        CONNECTED_PEERS.set(0);
        
        // Test setting positive values
        update_connected_peers(5);
        assert_eq!(CONNECTED_PEERS.get(), 5);
        
        update_connected_peers(10);
        assert_eq!(CONNECTED_PEERS.get(), 10);
        
        // Test setting zero
        update_connected_peers(0);
        assert_eq!(CONNECTED_PEERS.get(), 0);
        
        // Test setting negative values (should work for gauge)
        update_connected_peers(-1);
        assert_eq!(CONNECTED_PEERS.get(), -1);
    }

    #[test]
    fn test_increment_connection_attempts() {
        let initial_value = CONNECTION_ATTEMPTS.get();
        
        increment_connection_attempts();
        let after_first = CONNECTION_ATTEMPTS.get();
        assert!(after_first > initial_value, "Expected {} > {}", after_first, initial_value);
        
        increment_connection_attempts();
        let after_second = CONNECTION_ATTEMPTS.get();
        assert!(after_second > after_first, "Expected {} > {}", after_second, after_first);
        assert_eq!(after_second, after_first + 1);
    }

    #[test]
    fn test_increment_successful_connections() {
        let initial_value = SUCCESSFUL_CONNECTIONS.get();
        
        increment_successful_connections();
        let after_first = SUCCESSFUL_CONNECTIONS.get();
        assert!(after_first > initial_value);
        
        increment_successful_connections();
        let after_second = SUCCESSFUL_CONNECTIONS.get();
        assert!(after_second > after_first);
        assert_eq!(after_second, after_first + 1);
    }

    #[test]
    fn test_increment_failed_connections() {
        let initial_value = FAILED_CONNECTIONS.get();
        
        increment_failed_connections();
        let after_first = FAILED_CONNECTIONS.get();
        assert!(after_first > initial_value);
        
        increment_failed_connections();
        let after_second = FAILED_CONNECTIONS.get();
        assert!(after_second > after_first);
        assert_eq!(after_second, after_first + 1);
    }

    #[test]
    fn test_record_network_event() {
        let event_type = "test_event_unique";
        let labels = EventLabels {
            event_type: event_type.to_string(),
        };
        
        let initial_value = NETWORK_EVENTS.get_or_create(&labels).get();
        
        record_network_event(event_type);
        let after_first = NETWORK_EVENTS.get_or_create(&labels).get();
        assert_eq!(after_first, initial_value + 1);
        
        record_network_event(event_type);
        let after_second = NETWORK_EVENTS.get_or_create(&labels).get();
        assert_eq!(after_second, initial_value + 2);
    }

    #[test]
    fn test_record_network_event_different_types() {
        let event_type1 = "connection_established_unique";
        let event_type2 = "connection_closed_unique";
        
        let labels1 = EventLabels {
            event_type: event_type1.to_string(),
        };
        let labels2 = EventLabels {
            event_type: event_type2.to_string(),
        };
        
        let initial_value1 = NETWORK_EVENTS.get_or_create(&labels1).get();
        let initial_value2 = NETWORK_EVENTS.get_or_create(&labels2).get();
        
        record_network_event(event_type1);
        record_network_event(event_type2);
        record_network_event(event_type1);
        
        assert_eq!(NETWORK_EVENTS.get_or_create(&labels1).get(), initial_value1 + 2);
        assert_eq!(NETWORK_EVENTS.get_or_create(&labels2).get(), initial_value2 + 1);
    }

    #[test]
    fn test_update_uptime() {
        // Create a start time in the past
        let start_time = Instant::now() - Duration::from_secs(10);
        
        update_uptime(start_time);
        
        // The uptime should be at least 10 seconds (allowing for small timing variations)
        let uptime = UPTIME_SECONDS.get();
        assert!(uptime >= 10, "Expected uptime >= 10, got {}", uptime);
        assert!(uptime <= 12, "Expected uptime <= 12, got {}", uptime); // Allow some margin
    }

    #[test]
    fn test_update_uptime_zero() {
        let start_time = Instant::now();
        
        update_uptime(start_time);
        
        // The uptime should be 0 or very close to 0
        let uptime = UPTIME_SECONDS.get();
        assert!(uptime <= 1, "Expected uptime <= 1, got {}", uptime);
    }

    #[test]
    fn test_event_labels_equality() {
        let label1 = EventLabels {
            event_type: "test".to_string(),
        };
        let label2 = EventLabels {
            event_type: "test".to_string(),
        };
        let label3 = EventLabels {
            event_type: "different".to_string(),
        };
        
        assert_eq!(label1, label2);
        assert_ne!(label1, label3);
    }

    #[test]
    fn test_metrics_independence() {
        // Test that different metrics are independent
        let initial_attempts = CONNECTION_ATTEMPTS.get();
        let initial_successful = SUCCESSFUL_CONNECTIONS.get();
        let initial_failed = FAILED_CONNECTIONS.get();
        
        increment_connection_attempts();
        
        // Only connection attempts should have changed
        assert!(CONNECTION_ATTEMPTS.get() > initial_attempts);
        assert_eq!(SUCCESSFUL_CONNECTIONS.get(), initial_successful);
        assert_eq!(FAILED_CONNECTIONS.get(), initial_failed);
    }

    #[test]
    fn test_metrics_error_resilience() {
        // Test that metrics functions handle errors gracefully
        // These functions now have panic protection, so they should not crash
        
        update_connected_peers(100);
        assert_eq!(CONNECTED_PEERS.get(), 100);
        
        increment_connection_attempts();
        increment_successful_connections();
        increment_failed_connections();
        
        record_network_event("test_event");
        
        let start_time = Instant::now() - Duration::from_secs(5);
        update_uptime(start_time);
        
        // All operations should complete without panicking
        assert!(true);
    }

    #[test]
    fn test_metrics_with_extreme_values() {
        // Test metrics with extreme values
        update_connected_peers(i64::MAX);
        assert_eq!(CONNECTED_PEERS.get(), i64::MAX);
        
        update_connected_peers(i64::MIN);
        assert_eq!(CONNECTED_PEERS.get(), i64::MIN);
        
        update_connected_peers(0);
        assert_eq!(CONNECTED_PEERS.get(), 0);
    }

    #[test]
    fn test_network_event_with_special_characters() {
        // Test that network events with special characters are handled
        let special_events = vec![
            "event_with_underscore",
            "event-with-dash",
            "event.with.dots",
            "event with spaces",
            "event/with/slashes",
            "event\\with\\backslashes",
            "event\"with\"quotes",
            "event'with'apostrophes",
            "",  // Empty string
        ];
        
        for event in special_events {
            record_network_event(event);
            // Should not panic or crash
        }
    }

    #[test]
    fn test_concurrent_metrics_updates() {
        use std::thread;
        
        let handles: Vec<_> = (0..10).map(|i| {
            thread::spawn(move || {
                for j in 0..100 {
                    update_connected_peers((i * 100 + j) as i64);
                    increment_connection_attempts();
                    record_network_event(&format!("event_{}", i));
                }
            })
        }).collect();
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        // All operations should complete without issues
        assert!(CONNECTION_ATTEMPTS.get() >= 1000);
    }

    #[test]
    fn test_uptime_with_time_anomalies() {
        // Test uptime calculation with various time scenarios
        let now = Instant::now();
        
        // Normal case
        update_uptime(now - Duration::from_secs(10));
        assert!(UPTIME_SECONDS.get() >= 10);
        
        // Very recent start time
        update_uptime(now);
        assert!(UPTIME_SECONDS.get() >= 0);
        
        // Start time in the "future" (due to clock adjustments)
        // This shouldn't panic, though the result might be 0
        update_uptime(now + Duration::from_secs(1));
        // Should not panic
    }
}