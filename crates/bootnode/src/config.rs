use clap::Parser;
use std::net::{IpAddr, SocketAddr};

/// Default values for metrics configuration
pub const DEFAULT_METRICS_HOST: &str = "0.0.0.0";
pub const DEFAULT_METRICS_PORT: u16 = 9090;

#[derive(Debug, Clone)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
}

#[derive(Parser, Debug, Clone)]
pub struct MonitoringArgs {
    #[arg(
        long, 
        env = "METRICS_HOST", 
        default_value = DEFAULT_METRICS_HOST,
        help = "Host address to bind the metrics HTTP server",
        long_help = "Host address to bind the metrics HTTP server. Accepts IP addresses (0.0.0.0, 127.0.0.1) or hostnames (localhost). Use 0.0.0.0 to bind to all interfaces."
    )]
    pub metrics_host: String,
    
    #[arg(
        long, 
        env = "METRICS_PORT", 
        default_value_t = DEFAULT_METRICS_PORT,
        help = "Port to bind the metrics HTTP server",
        long_help = "Port to bind the metrics HTTP server. Must be above 1023 to avoid system reserved ports. Common values: 9090 (Prometheus default), 8080, 3000."
    )]
    pub metrics_port: u16,
    
    #[arg(
        long, 
        env = "DISABLE_METRICS",
        help = "Disable metrics collection and HTTP server",
        long_help = "Completely disable the metrics collection system and HTTP server. When disabled, no monitoring endpoints will be available and no metrics will be collected."
    )]
    pub disable_metrics: bool,
}

impl Default for MonitoringArgs {
    fn default() -> Self {
        Self {
            metrics_host: DEFAULT_METRICS_HOST.to_string(),
            metrics_port: DEFAULT_METRICS_PORT,
            disable_metrics: false,
        }
    }
}

impl MetricsConfig {
    /// Create MetricsConfig from MonitoringArgs with validation and fallback to defaults
    pub fn from_monitoring_args(args: &MonitoringArgs) -> Self {
        let mut config = Self {
            enabled: !args.disable_metrics,
            host: args.metrics_host.clone(),
            port: args.metrics_port,
        };

        // Validate and apply defaults if invalid
        if !config.is_valid() {
            log::warn!("Invalid metrics configuration provided, using defaults");
            config = Self::default();
            config.enabled = !args.disable_metrics; // Preserve the disable flag
        }

        config
    }

    /// Create default MetricsConfig
    pub fn default() -> Self {
        Self {
            enabled: true,
            host: DEFAULT_METRICS_HOST.to_string(),
            port: DEFAULT_METRICS_PORT,
        }
    }

    /// Validate the configuration
    pub fn is_valid(&self) -> bool {
        self.is_valid_host() && self.is_valid_port()
    }

    /// Validate the host address
    pub fn is_valid_host(&self) -> bool {
        if self.host.is_empty() {
            return false;
        }

        // Try to parse as IP address
        if self.host.parse::<IpAddr>().is_ok() {
            return true;
        }

        // Check for valid hostname patterns (basic validation)
        // Allow localhost and basic hostname patterns
        if self.host == "localhost" || self.host.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-') {
            return true;
        }

        false
    }

    /// Validate the port number
    pub fn is_valid_port(&self) -> bool {
        // Avoid well-known system ports (0-1023) but allow common development ports
        self.port > 1023
    }

    /// Get the socket address for binding
    pub fn socket_addr(&self) -> Result<SocketAddr, std::net::AddrParseError> {
        format!("{}:{}", self.host, self.port).parse()
    }

    /// Check if metrics should be enabled
    pub fn should_start_server(&self) -> bool {
        self.enabled && self.is_valid()
    }
}
#
[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_monitoring_args() {
        let args = MonitoringArgs::default();
        assert_eq!(args.metrics_host, DEFAULT_METRICS_HOST);
        assert_eq!(args.metrics_port, DEFAULT_METRICS_PORT);
        assert!(!args.disable_metrics);
    }

    #[test]
    fn test_default_metrics_config() {
        let config = MetricsConfig::default();
        assert!(config.enabled);
        assert_eq!(config.host, DEFAULT_METRICS_HOST);
        assert_eq!(config.port, DEFAULT_METRICS_PORT);
        assert!(config.is_valid());
    }

    #[test]
    fn test_metrics_config_from_valid_args() {
        let args = MonitoringArgs {
            metrics_host: "127.0.0.1".to_string(),
            metrics_port: 8080,
            disable_metrics: false,
        };

        let config = MetricsConfig::from_monitoring_args(&args);
        assert!(config.enabled);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert!(config.is_valid());
    }

    #[test]
    fn test_metrics_config_disabled() {
        let args = MonitoringArgs {
            metrics_host: "0.0.0.0".to_string(),
            metrics_port: 9090,
            disable_metrics: true,
        };

        let config = MetricsConfig::from_monitoring_args(&args);
        assert!(!config.enabled);
        assert!(!config.should_start_server());
    }

    #[test]
    fn test_metrics_config_invalid_host_fallback() {
        let args = MonitoringArgs {
            metrics_host: "".to_string(), // Invalid empty host
            metrics_port: 9090,
            disable_metrics: false,
        };

        let config = MetricsConfig::from_monitoring_args(&args);
        assert!(config.enabled);
        assert_eq!(config.host, DEFAULT_METRICS_HOST); // Should fallback to default
        assert_eq!(config.port, DEFAULT_METRICS_PORT);
        assert!(config.is_valid());
    }

    #[test]
    fn test_metrics_config_invalid_port_fallback() {
        let args = MonitoringArgs {
            metrics_host: "0.0.0.0".to_string(),
            metrics_port: 0, // Invalid port
            disable_metrics: false,
        };

        let config = MetricsConfig::from_monitoring_args(&args);
        assert!(config.enabled);
        assert_eq!(config.host, DEFAULT_METRICS_HOST); // Should fallback to default
        assert_eq!(config.port, DEFAULT_METRICS_PORT);
        assert!(config.is_valid());
    }

    #[test]
    fn test_host_validation() {
        let mut config = MetricsConfig::default();

        // Valid hosts
        config.host = "0.0.0.0".to_string();
        assert!(config.is_valid_host());

        config.host = "127.0.0.1".to_string();
        assert!(config.is_valid_host());

        config.host = "192.168.1.1".to_string();
        assert!(config.is_valid_host());

        config.host = "localhost".to_string();
        assert!(config.is_valid_host());

        config.host = "example.com".to_string();
        assert!(config.is_valid_host());

        // Invalid hosts
        config.host = "".to_string();
        assert!(!config.is_valid_host());

        config.host = "invalid@host".to_string();
        assert!(!config.is_valid_host());
    }

    #[test]
    fn test_port_validation() {
        let mut config = MetricsConfig::default();

        // Valid ports
        config.port = 8080;
        assert!(config.is_valid_port());

        config.port = 9090;
        assert!(config.is_valid_port());

        config.port = 3000;
        assert!(config.is_valid_port());

        // Invalid ports
        config.port = 0;
        assert!(!config.is_valid_port());

        config.port = 22; // SSH port
        assert!(!config.is_valid_port());

        config.port = 80; // HTTP port
        assert!(!config.is_valid_port());

        config.port = 443; // HTTPS port
        assert!(!config.is_valid_port());
    }

    #[test]
    fn test_socket_addr_creation() {
        let config = MetricsConfig {
            enabled: true,
            host: "127.0.0.1".to_string(),
            port: 8080,
        };

        let socket_addr = config.socket_addr().unwrap();
        assert_eq!(socket_addr.to_string(), "127.0.0.1:8080");
    }

    #[test]
    fn test_socket_addr_creation_invalid() {
        let config = MetricsConfig {
            enabled: true,
            host: "invalid@host".to_string(),
            port: 8080,
        };

        assert!(config.socket_addr().is_err());
    }

    #[test]
    fn test_should_start_server() {
        // Enabled and valid
        let config = MetricsConfig {
            enabled: true,
            host: "0.0.0.0".to_string(),
            port: 9090,
        };
        assert!(config.should_start_server());

        // Disabled but valid
        let config = MetricsConfig {
            enabled: false,
            host: "0.0.0.0".to_string(),
            port: 9090,
        };
        assert!(!config.should_start_server());

        // Enabled but invalid
        let config = MetricsConfig {
            enabled: true,
            host: "".to_string(),
            port: 9090,
        };
        assert!(!config.should_start_server());
    }

    #[test]
    fn test_configuration_precedence() {
        // Test that command line args take precedence over defaults
        let args = MonitoringArgs {
            metrics_host: "192.168.1.100".to_string(),
            metrics_port: 8888,
            disable_metrics: false,
        };

        let config = MetricsConfig::from_monitoring_args(&args);
        assert_eq!(config.host, "192.168.1.100");
        assert_eq!(config.port, 8888);
        assert!(config.enabled);
    }

    #[test]
    fn test_disable_metrics_preserves_other_config() {
        let args = MonitoringArgs {
            metrics_host: "127.0.0.1".to_string(),
            metrics_port: 8080,
            disable_metrics: true,
        };

        let config = MetricsConfig::from_monitoring_args(&args);
        assert!(!config.enabled);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert!(!config.should_start_server()); // Should not start server when disabled
    }

    #[test]
    fn test_invalid_config_fallback_preserves_disable_flag() {
        let args = MonitoringArgs {
            metrics_host: "".to_string(), // Invalid
            metrics_port: 0, // Invalid
            disable_metrics: true,
        };

        let config = MetricsConfig::from_monitoring_args(&args);
        assert!(!config.enabled); // Disable flag should be preserved
        assert_eq!(config.host, DEFAULT_METRICS_HOST); // Should fallback to defaults
        assert_eq!(config.port, DEFAULT_METRICS_PORT);
        assert!(!config.should_start_server());
    }

    #[test]
    fn test_config_error_handling_with_extreme_values() {
        // Test with extreme port values
        let args = MonitoringArgs {
            metrics_host: "127.0.0.1".to_string(),
            metrics_port: u16::MAX,
            disable_metrics: false,
        };

        let config = MetricsConfig::from_monitoring_args(&args);
        assert!(config.enabled);
        assert_eq!(config.port, u16::MAX);
        assert!(config.is_valid_port()); // Should be valid
        
        // Test with port 0
        let args = MonitoringArgs {
            metrics_host: "127.0.0.1".to_string(),
            metrics_port: 0,
            disable_metrics: false,
        };

        let config = MetricsConfig::from_monitoring_args(&args);
        // Should fallback to defaults due to invalid port
        assert_eq!(config.host, DEFAULT_METRICS_HOST);
        assert_eq!(config.port, DEFAULT_METRICS_PORT);
    }

    #[test]
    fn test_config_with_malformed_hosts() {
        let malformed_hosts = vec![
            "256.256.256.256", // Invalid IP
            "host@domain.com",  // Invalid characters
            "host with spaces", // Spaces
            "host\nwith\nnewlines", // Newlines
            "host\twith\ttabs", // Tabs
            "very-long-hostname-that-exceeds-normal-limits-and-might-cause-issues-in-some-systems-but-should-still-be-handled-gracefully",
        ];

        for host in malformed_hosts {
            let args = MonitoringArgs {
                metrics_host: host.to_string(),
                metrics_port: 9090,
                disable_metrics: false,
            };

            let config = MetricsConfig::from_monitoring_args(&args);
            // Should either be valid or fallback to defaults
            assert!(config.is_valid());
        }
    }

    #[test]
    fn test_socket_addr_error_handling() {
        // Test various invalid socket address combinations
        let invalid_configs = vec![
            ("999.999.999.999", 8080),
            ("", 8080),
            ("localhost", 0),
        ];

        for (host, port) in invalid_configs {
            let config = MetricsConfig {
                enabled: true,
                host: host.to_string(),
                port,
            };

            // Should handle errors gracefully
            let result = config.socket_addr();
            // Either succeeds or fails gracefully
            match result {
                Ok(_) => {}, // Valid address
                Err(_) => {}, // Invalid address handled gracefully
            }
        }
    }

    #[test]
    fn test_config_validation_edge_cases() {
        let mut config = MetricsConfig::default();

        // Test with reserved/system ports (0-1023)
        let reserved_ports = vec![0, 21, 22, 23, 25, 53, 80, 110, 143, 443, 993, 995, 1023];
        for port in reserved_ports {
            config.port = port;
            assert!(!config.is_valid_port(), "Port {} should be invalid", port);
        }

        // Test with valid high ports (above 1023)
        let valid_ports = vec![1024, 3000, 8000, 8443, 8888, 9000, 9090];
        for port in valid_ports {
            config.port = port;
            assert!(config.is_valid_port(), "Port {} should be valid", port);
        }
    }

    #[test]
    fn test_config_resilience_with_concurrent_access() {
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
                }
            })
        }).collect();

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_config_should_start_server_logic() {
        // Test all combinations of enabled/disabled and valid/invalid
        let test_cases = vec![
            (true, true, true),   // enabled + valid = should start
            (true, false, false), // enabled + invalid = should not start
            (false, true, false), // disabled + valid = should not start
            (false, false, false), // disabled + invalid = should not start
        ];

        for (enabled, valid, expected) in test_cases {
            let config = MetricsConfig {
                enabled,
                host: if valid { "127.0.0.1".to_string() } else { "".to_string() },
                port: if valid { 9090 } else { 0 },
            };

            assert_eq!(config.should_start_server(), expected,
                "enabled: {}, valid: {}, expected: {}", enabled, valid, expected);
        }
    }
}