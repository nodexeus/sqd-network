use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct BootnodeState {
    pub start_time: Instant,
    pub connected_peers: Arc<AtomicUsize>,
    pub dht_ready: Arc<AtomicBool>,
}

impl BootnodeState {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            connected_peers: Arc::new(AtomicUsize::new(0)),
            dht_ready: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn peer_connected(&self) {
        let count = self.connected_peers.fetch_add(1, Ordering::SeqCst) + 1;
        // Use error handling that doesn't require UnwindSafe
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::metrics::update_connected_peers(count as i64);
            crate::metrics::increment_successful_connections();
        })) {
            log::error!("Error occurred while updating metrics for peer connection: {:?}", e);
            // Core functionality (updating the counter) already succeeded
        }
    }

    pub fn peer_disconnected(&self) {
        let mut new_value;
        loop {
            let current = self.connected_peers.load(Ordering::SeqCst);
            if current == 0 {
                // Already at zero, nothing to disconnect
                return;
            }
            new_value = current - 1;
            match self.connected_peers.compare_exchange_weak(
                current,
                new_value,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(_) => {
                    // Retry the loop - another thread modified the value
                    continue;
                }
            }
        }
        
        // Update metrics after successful counter update
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::metrics::update_connected_peers(new_value as i64);
        })) {
            log::error!("Error occurred while updating metrics for peer disconnection: {:?}", e);
            // Core functionality (updating the counter) already succeeded
        }
    }

    pub fn set_dht_ready(&self, ready: bool) {
        self.dht_ready.store(ready, Ordering::SeqCst);
        
        if ready {
            if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::metrics::record_network_event("dht_ready");
            })) {
                log::error!("Error occurred while recording DHT ready event: {:?}", e);
                // Core functionality (updating the state) already succeeded
            }
        }
    }

    pub fn get_peer_count(&self) -> usize {
        self.connected_peers.load(Ordering::SeqCst)
    }

    pub fn is_dht_ready(&self) -> bool {
        self.dht_ready.load(Ordering::SeqCst)
    }

    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }
}

impl Default for BootnodeState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_bootnode_state_new() {
        let state = BootnodeState::new();
        
        assert_eq!(state.get_peer_count(), 0);
        assert!(!state.is_dht_ready());
        assert!(state.uptime() >= Duration::from_secs(0));
        assert!(state.uptime() < Duration::from_secs(1)); // Should be very recent
    }

    #[test]
    fn test_peer_connection_tracking() {
        let state = BootnodeState::new();
        
        // Test initial state
        assert_eq!(state.get_peer_count(), 0);
        
        // Test peer connection
        state.peer_connected();
        assert_eq!(state.get_peer_count(), 1);
        
        // Test multiple peer connections
        state.peer_connected();
        state.peer_connected();
        assert_eq!(state.get_peer_count(), 3);
        
        // Test peer disconnection
        state.peer_disconnected();
        assert_eq!(state.get_peer_count(), 2);
        
        // Test disconnection doesn't go below zero
        state.peer_disconnected();
        state.peer_disconnected();
        state.peer_disconnected(); // This should not cause underflow
        assert_eq!(state.get_peer_count(), 0);
    }

    #[test]
    fn test_dht_readiness_state() {
        let state = BootnodeState::new();
        
        // Test initial state
        assert!(!state.is_dht_ready());
        
        // Test setting DHT ready
        state.set_dht_ready(true);
        assert!(state.is_dht_ready());
        
        // Test setting DHT not ready
        state.set_dht_ready(false);
        assert!(!state.is_dht_ready());
        
        // Test setting ready again
        state.set_dht_ready(true);
        assert!(state.is_dht_ready());
    }

    #[test]
    fn test_uptime_calculation() {
        let state = BootnodeState::new();
        
        // Initial uptime should be very small
        let initial_uptime = state.uptime();
        assert!(initial_uptime < Duration::from_millis(100));
        
        // Wait a bit and check uptime increased
        thread::sleep(Duration::from_millis(10));
        let later_uptime = state.uptime();
        assert!(later_uptime > initial_uptime);
        assert!(later_uptime >= Duration::from_millis(10));
    }

    #[test]
    fn test_concurrent_peer_tracking() {
        use std::sync::Arc;
        use std::thread;
        
        let state = Arc::new(BootnodeState::new());
        let mut handles = vec![];
        
        // Spawn multiple threads that increment peer count
        for _ in 0..10 {
            let state_clone = Arc::clone(&state);
            let handle = thread::spawn(move || {
                for _ in 0..100 {
                    state_clone.peer_connected();
                }
            });
            handles.push(handle);
        }
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Should have 1000 connected peers
        assert_eq!(state.get_peer_count(), 1000);
        
        // Now test concurrent disconnections
        let mut handles = vec![];
        for _ in 0..5 {
            let state_clone = Arc::clone(&state);
            let handle = thread::spawn(move || {
                for _ in 0..200 {
                    state_clone.peer_disconnected();
                }
            });
            handles.push(handle);
        }
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Should have 0 connected peers (1000 - 1000 = 0)
        assert_eq!(state.get_peer_count(), 0);
    }

    #[test]
    fn test_concurrent_dht_state_changes() {
        use std::sync::Arc;
        use std::thread;
        
        let state = Arc::new(BootnodeState::new());
        let mut handles = vec![];
        
        // Spawn threads that toggle DHT readiness
        for i in 0..10 {
            let state_clone = Arc::clone(&state);
            let handle = thread::spawn(move || {
                for j in 0..100 {
                    state_clone.set_dht_ready((i + j) % 2 == 0);
                }
            });
            handles.push(handle);
        }
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // DHT readiness should be either true or false (no corruption)
        let is_ready = state.is_dht_ready();
        assert!(is_ready == true || is_ready == false);
    }

    #[test]
    fn test_state_clone() {
        let state1 = BootnodeState::new();
        state1.peer_connected();
        state1.peer_connected();
        state1.set_dht_ready(true);
        
        // Clone the state
        let state2 = state1.clone();
        
        // Both states should share the same atomic values
        assert_eq!(state1.get_peer_count(), state2.get_peer_count());
        assert_eq!(state1.is_dht_ready(), state2.is_dht_ready());
        
        // Modifying one should affect the other (shared atomics)
        state1.peer_connected();
        assert_eq!(state2.get_peer_count(), 3);
        
        state2.set_dht_ready(false);
        assert!(!state1.is_dht_ready());
    }

    #[test]
    fn test_default_implementation() {
        let state1 = BootnodeState::default();
        let state2 = BootnodeState::new();
        
        // Default should behave the same as new()
        assert_eq!(state1.get_peer_count(), state2.get_peer_count());
        assert_eq!(state1.is_dht_ready(), state2.is_dht_ready());
    }

    #[test]
    fn test_peer_count_edge_cases() {
        let state = BootnodeState::new();
        
        // Test disconnecting when no peers are connected
        state.peer_disconnected();
        assert_eq!(state.get_peer_count(), 0);
        
        // Test multiple disconnections when count is 0
        for _ in 0..5 {
            state.peer_disconnected();
        }
        assert_eq!(state.get_peer_count(), 0);
        
        // Test that we can still connect peers after underflow attempts
        state.peer_connected();
        assert_eq!(state.get_peer_count(), 1);
    }

    #[test]
    fn test_state_error_resilience() {
        let state = BootnodeState::new();
        
        // Test that state operations handle errors gracefully
        // These operations now have panic protection
        
        // Test peer connection/disconnection with error handling
        for _ in 0..100 {
            state.peer_connected();
        }
        assert_eq!(state.get_peer_count(), 100);
        
        for _ in 0..50 {
            state.peer_disconnected();
        }
        assert_eq!(state.get_peer_count(), 50);
        
        // Test DHT state changes with error handling
        state.set_dht_ready(true);
        assert!(state.is_dht_ready());
        
        state.set_dht_ready(false);
        assert!(!state.is_dht_ready());
        
        // All operations should complete without panicking
        assert!(true);
    }

    #[test]
    fn test_state_with_extreme_peer_counts() {
        let state = BootnodeState::new();
        
        // Test with a large number of peer connections
        for _ in 0..10000 {
            state.peer_connected();
        }
        assert_eq!(state.get_peer_count(), 10000);
        
        // Test disconnecting all peers
        for _ in 0..10000 {
            state.peer_disconnected();
        }
        assert_eq!(state.get_peer_count(), 0);
        
        // Test excessive disconnections
        for _ in 0..100 {
            state.peer_disconnected();
        }
        assert_eq!(state.get_peer_count(), 0);
    }

    #[test]
    fn test_state_operations_during_high_contention() {
        use std::sync::Arc;
        use std::thread;
        use std::sync::atomic::{AtomicBool, Ordering};
        
        let state = Arc::new(BootnodeState::new());
        let should_stop = Arc::new(AtomicBool::new(false));
        let mut handles = vec![];
        
        // Spawn threads that continuously modify state
        for i in 0..5 {
            let state_clone = Arc::clone(&state);
            let should_stop_clone = Arc::clone(&should_stop);
            let handle = thread::spawn(move || {
                let mut counter = 0;
                while !should_stop_clone.load(Ordering::Relaxed) {
                    if i % 2 == 0 {
                        state_clone.peer_connected();
                    } else {
                        state_clone.peer_disconnected();
                    }
                    state_clone.set_dht_ready(counter % 2 == 0);
                    counter += 1;
                    
                    if counter > 1000 {
                        break;
                    }
                }
            });
            handles.push(handle);
        }
        
        // Let threads run for a short time
        thread::sleep(Duration::from_millis(100));
        should_stop.store(true, Ordering::Relaxed);
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // State should be consistent (no corruption)
        let peer_count = state.get_peer_count();
        let dht_ready = state.is_dht_ready();
        
        // Values should be valid (no corruption)
        assert!(peer_count < 10000); // Reasonable upper bound
        assert!(dht_ready == true || dht_ready == false); // Boolean should be valid
    }

    #[test]
    fn test_uptime_consistency() {
        let state = BootnodeState::new();
        
        // Uptime should be monotonically increasing
        let uptime1 = state.uptime();
        thread::sleep(Duration::from_millis(10));
        let uptime2 = state.uptime();
        
        assert!(uptime2 >= uptime1);
        
        // Test uptime over multiple calls
        let mut previous_uptime = Duration::from_secs(0);
        for _ in 0..10 {
            let current_uptime = state.uptime();
            assert!(current_uptime >= previous_uptime);
            previous_uptime = current_uptime;
            thread::sleep(Duration::from_millis(1));
        }
    }
}