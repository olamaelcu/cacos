//! Observability: tracing registry, metrics recorder, timing histograms, /metrics route.
pub mod http;
pub mod metrics;
pub mod timing;
pub mod tracing;

#[cfg(test)]
mod tests {
    #[test]
    fn module_tree_wired() {
        // Empty-module skeleton: this test's compilation proves the observability
        // module tree (metrics/timing/tracing) is wired into the crate.
        assert!(true);
    }
}
