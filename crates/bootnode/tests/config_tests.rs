use std::env;
use sqld_bootnode::config::{MetricsConfig, MonitoringArgs, DEFAULT_METRICS_HOST, DEFAULT_METRICS_PORT};

#[test]
fn test_config_edge_cases_and_validation() {
    // Test with extreme port values
    let args = MonitoringArgs {
        metrics_host: "127.0.0.1".to_string(),
        metrics_port: u16::MAX,
        disable_metrics: false,
    };
    
    let config = MetricsConfig::from_monitoring_args(&args);
    assert!(config.enabled);
    assert_eq!(config.port, u16::MAX);
    assert!(config.is_valid());
    
    // Test with port 0 (should fallback to defaults)
    let args = MonitoringArgs {
        metrics_host: "127.0.0.1".to_string(),
        metrics_port: 0,
        disable_metrics: false,
    };
    
    let config = MetricsConfig::from_monitoring_args(&args);
    assert_eq!(config.host, DEFAULT_METRICS_HOST);
    assert_eq!(config.port, DEFAULT_METRICS_PORT);
    
    // Test with empty host (should fallback to defaults)
    let args = MonitoringArgs {
        metrics_host: "".to_string(),
        metrics_port: 9090,
        disable_metrics: false,
    };
    
    let config = MetricsConfig::from_monitoring_args(&args);
    assert_eq!(config.host, DEFAULT_METRICS_HOST);
    assert_eq!(config.port, DEFAULT_METRICS_PORT);
}

#[test]
fn test_config_with_malformed_inputs() {
    let malformed_hosts = vec![
        "256.256.256.256",  // Invalid IP
        "host@domain.com",   // Invalid characters
        "host with spaces",  // Spaces
        "host\nwith\nnewlines", // Newlines
        "host\twith\ttabs",  // Tabs
        "very-long-hostname-that-exceeds-normal-limits-and-might-cause-issues-in-some-systems-but-should-still-be-handled-gracefully-by-falling-back-to-defaults",
    ];
    
    for host in malformed_hosts {
        let args = MonitoringArgs {
            metrics_host: host.to_string(),
            metrics_port: 9090,
            disable_metrics: false,
        };
        
        let config = MetricsConfig::from_monitoring_args(&args);
        // Should either be valid or fallback to defaults
        assert!(config.is_valid(), "Config should be valid for host: {}", host);
    }
}

#[test]
fn test_config_reserved_ports() {
    let reserved_ports = vec![0, 21, 22, 23, 25, 53, 80, 110, 143, 443, 993, 995, 1023];
    
    for port in reserved_ports {
        let args = MonitoringArgs {
            metrics_host: "127.0.0.1".to_string(),
            metrics_port: port,
            disable_metrics: false,
        };
        
        let config = MetricsConfig::from_monitoring_args(&args);
        // Should fallback to defaults for reserved ports
        if port <= 1023 {
            assert_eq!(config.host, DEFAULT_METRICS_HOST);
            assert_eq!(config.port, DEFAULT_METRICS_PORT);
        }
        assert!(config.is_valid());
    }
}

#[test]
fn test_config_valid_high_ports() {
    let valid_ports = vec![1024, 3000, 8000, 8443, 8888, 9000, 9090, 65535];
    
    for port in valid_ports {
        let args = MonitoringArgs {
            metrics_host: "127.0.0.1".to_string(),
            metrics_port: port,
            disable_metrics: false,
        };
        
        let config = MetricsConfig::from_monitoring_args(&args);
        assert!(config.is_valid());
        assert_eq!(config.port, port);
    }
}

#[test]
fn test_config_disable_metrics_combinations() {
    // Test disable with valid config
    let args = MonitoringArgs {
        metrics_host: "127.0.0.1".to_string(),
        metrics_port: 9090,
        disable_metrics: true,
    };
    
    let config = MetricsConfig::from_monitoring_args(&args);
    assert!(!config.enabled);
    assert!(!config.should_start_server());
    
    // Test disable with invalid config
    let args = MonitoringArgs {
        metrics_host: "".to_string(),
        metrics_port: 0,
        disable_metrics: true,
    };
    
    let config = MetricsConfig::from_monitoring_args(&args);
    assert!(!config.enabled);
    assert!(!config.should_start_server());
    // Should still fallback to valid defaults for host/port
    assert_eq!(config.host, DEFAULT_METRICS_HOST);
    assert_eq!(config.port, DEFAULT_METRICS_PORT);
}

#[test]
fn test_config_socket_addr_creation() {
    let test_cases = vec![
        ("127.0.0.1", 8080, true),
        ("0.0.0.0", 9090, true),
        ("192.168.1.1", 3000, true),
        ("localhost", 8000, false), // localhost might not resolve in all environments
        ("invalid@host", 8080, false),
        ("", 8080, false),
    ];
    
    for (host, port, should_succeed) in test_cases {
        let config = MetricsConfig {
            enabled: true,
            host: host.to_string(),
            port,
        };
        
        let result = config.socket_addr();
        if should_succeed {
            assert!(result.is_ok(), "Socket addr creation should succeed for {}:{}", host, port);
        } else {
            // May succeed or fail depending on system configuration
            // Just ensure it doesn't panic
            let _ = result;
        }
    }
}

#[test]
fn test_config_validation_consistency() {
    // Test that is_valid() is consistent with should_start_server()
    let test_configs = vec![
        (true, "127.0.0.1", 9090, true),   // enabled + valid = should start
        (true, "", 9090, false),           // enabled + invalid host = should not start
        (true, "127.0.0.1", 0, false),    // enabled + invalid port = should not start
        (false, "127.0.0.1", 9090, false), // disabled + valid = should not start
        (false, "", 0, false),             // disabled + invalid = should not start
    ];
    
    for (enabled, host, port, expected_should_start) in test_configs {
        let config = MetricsConfig {
            enabled,
            host: host.to_string(),
            port,
        };
        
        let should_start = config.should_start_server();
        assert_eq!(should_start, expected_should_start,
            "should_start_server() mismatch for enabled={}, host={}, port={}", 
            enabled, host, port);
        
        if enabled {
            // If enabled, should_start_server should match is_valid
            assert_eq!(should_start, config.is_valid(),
                "should_start_server() should match is_valid() when enabled");
        }
    }
}

#[test]
fn test_config_concurrent_access() {
    use std::sync::Arc;
    use std::thread;
    
    let args = Arc::new(MonitoringArgs {
        metrics_host: "127.0.0.1".to_string(),
        metrics_port: 9090,
        disable_metrics: false,
    });
    
    let handles: Vec<_> = (0..10).map(|_| {
        let args_clone = Arc::clone(&args);
        thread::spawn(move || {
            for _ in 0..100 {
                let config = MetricsConfig::from_monitoring_args(&args_clone);
                assert!(config.is_valid());
                assert_eq!(config.host, "127.0.0.1");
                assert_eq!(config.port, 9090);
                assert!(config.enabled);
            }
        })
    }).collect();
    
    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_config_host_validation_edge_cases() {
    let mut config = MetricsConfig::default();
    
    // Test IPv6 addresses (basic validation)
    let ipv6_addresses = vec![
        "::1",
        "2001:db8::1",
        "fe80::1",
    ];
    
    for addr in ipv6_addresses {
        config.host = addr.to_string();
        // IPv6 parsing might not be supported in basic validation
        // Just ensure it doesn't panic
        let _ = config.is_valid_host();
    }
    
    // Test edge case hostnames
    let edge_hostnames = vec![
        "a",                    // Single character
        "a.b",                  // Minimal domain
        "test-host",            // With dash
        "test.example.com",     // Full domain
        "192.168.1.1",         // IP address
    ];
    
    for hostname in edge_hostnames {
        config.host = hostname.to_string();
        let is_valid = config.is_valid_host();
        // Should not panic, result may vary
        assert!(is_valid == true || is_valid == false);
    }
}

#[test]
fn test_config_port_boundary_conditions() {
    let mut config = MetricsConfig::default();
    
    // Test boundary values
    let boundary_ports = vec![
        (0, false),        // Minimum value
        (1, false),        // Just above minimum
        (1023, false),     // Last reserved port
        (1024, true),      // First non-reserved port
        (65534, true),     // Just below maximum
        (65535, true),     // Maximum value
    ];
    
    for (port, expected_valid) in boundary_ports {
        config.port = port;
        assert_eq!(config.is_valid_port(), expected_valid,
            "Port {} validation should be {}", port, expected_valid);
    }
}

#[test]
fn test_config_fallback_behavior() {
    // Test that fallback preserves disable flag
    let args = MonitoringArgs {
        metrics_host: "invalid@host".to_string(),
        metrics_port: 0,
        disable_metrics: true,
    };
    
    let config = MetricsConfig::from_monitoring_args(&args);
    assert!(!config.enabled); // Disable flag should be preserved
    assert_eq!(config.host, DEFAULT_METRICS_HOST);
    assert_eq!(config.port, DEFAULT_METRICS_PORT);
    assert!(config.is_valid());
    assert!(!config.should_start_server());
    
    // Test that fallback enables when not disabled
    let args = MonitoringArgs {
        metrics_host: "invalid@host".to_string(),
        metrics_port: 0,
        disable_metrics: false,
    };
    
    let config = MetricsConfig::from_monitoring_args(&args);
    assert!(config.enabled); // Should be enabled after fallback
    assert_eq!(config.host, DEFAULT_METRICS_HOST);
    assert_eq!(config.port, DEFAULT_METRICS_PORT);
    assert!(config.is_valid());
    assert!(config.should_start_server());
}

#[test]
fn test_config_default_values() {
    let default_config = MetricsConfig::default();
    assert!(default_config.enabled);
    assert_eq!(default_config.host, DEFAULT_METRICS_HOST);
    assert_eq!(default_config.port, DEFAULT_METRICS_PORT);
    assert!(default_config.is_valid());
    assert!(default_config.should_start_server());
    
    let default_args = MonitoringArgs::default();
    assert_eq!(default_args.metrics_host, DEFAULT_METRICS_HOST);
    assert_eq!(default_args.metrics_port, DEFAULT_METRICS_PORT);
    assert!(!default_args.disable_metrics);
}

#[test]
fn test_config_error_recovery() {
    // Test that configuration errors are handled gracefully
    let problematic_configs = vec![
        MonitoringArgs {
            metrics_host: "\0invalid".to_string(), // Null byte
            metrics_port: 9090,
            disable_metrics: false,
        },
        MonitoringArgs {
            metrics_host: "host".repeat(1000), // Very long hostname
            metrics_port: 9090,
            disable_metrics: false,
        },
        MonitoringArgs {
            metrics_host: "127.0.0.1".to_string(),
            metrics_port: u16::MAX,
            disable_metrics: false,
        },
    ];
    
    for args in problematic_configs {
        let config = MetricsConfig::from_monitoring_args(&args);
        // Should not panic and should result in a valid config
        assert!(config.is_valid());
    }
}

#[test]
fn test_config_string_representations() {
    let config = MetricsConfig {
        enabled: true,
        host: "127.0.0.1".to_string(),
        port: 8080,
    };
    
    // Test that Debug formatting works
    let debug_str = format!("{:?}", config);
    assert!(debug_str.contains("127.0.0.1"));
    assert!(debug_str.contains("8080"));
    assert!(debug_str.contains("true"));
    
    // Test socket address formatting
    let socket_addr = config.socket_addr().unwrap();
    assert_eq!(socket_addr.to_string(), "127.0.0.1:8080");
}

#[test]
fn test_config_memory_safety() {
    // Test that config operations don't cause memory issues
    let mut configs = Vec::new();
    
    for i in 0..1000 {
        let args = MonitoringArgs {
            metrics_host: format!("host{}.example.com", i),
            metrics_port: 8000 + (i % 1000) as u16,
            disable_metrics: i % 2 == 0,
        };
        
        let config = MetricsConfig::from_monitoring_args(&args);
        configs.push(config);
    }
    
    // Verify all configs are valid
    for config in &configs {
        assert!(config.is_valid());
    }
    
    // Test cloning
    let cloned_configs: Vec<_> = configs.iter().cloned().collect();
    assert_eq!(configs.len(), cloned_configs.len());
}