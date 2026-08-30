use canwu_sim::{
    format8_paged_checkpoint_scale_probe, format8_patricia_scale_probe,
    format8_trace_locator_scale_probe,
};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let keys = std::env::args()
        .nth(1)
        .map_or(Ok(1_000_000_usize), |value| value.parse())?;
    let started = Instant::now();
    let metrics = format8_patricia_scale_probe(keys)?;
    let elapsed_ms = started.elapsed().as_millis();
    let locator_started = Instant::now();
    let checkpoint_metrics = format8_paged_checkpoint_scale_probe(keys)?;
    let locator_elapsed_ms = locator_started.elapsed().as_millis();
    let locator_metrics = &checkpoint_metrics.decision_locator;
    let trace_started = Instant::now();
    let trace_metrics = format8_trace_locator_scale_probe(keys)?;
    let trace_elapsed_ms = trace_started.elapsed().as_millis();
    let structural_per_entry = metrics
        .primary
        .structural_bytes
        .checked_div(metrics.primary.entries.max(1))
        .unwrap_or(0);
    let resident_per_entry = metrics
        .primary
        .estimated_resident_bytes
        .checked_div(metrics.primary.entries.max(1))
        .unwrap_or(0);
    let total_patricia_structural_per_record = metrics
        .total_patricia_structural_bytes
        .checked_div(metrics.records.max(1))
        .unwrap_or(0);
    let total_patricia_resident_per_record = metrics
        .total_patricia_estimated_resident_bytes
        .checked_div(metrics.records.max(1))
        .unwrap_or(0);
    let gates = serde_json::json!({
        "primary_nodes_at_most_2n_minus_1": metrics.primary.logical_nodes <= metrics.primary.entries.saturating_mul(2).saturating_sub(1),
        "encoded_structural_bytes_per_entry_at_most_256": structural_per_entry <= 256,
        "resident_structural_bytes_per_entry_at_most_384": resident_per_entry <= 384,
        "all_patricia_encoded_bytes_per_record_at_most_640": total_patricia_structural_per_record <= 640,
        "all_patricia_resident_bytes_per_record_at_most_896": total_patricia_resident_per_record <= 896,
        "p99_depth_at_most_64": metrics.primary.depth_p99 <= 64,
        "max_depth_at_most_256": metrics.primary.max_depth <= 256,
        "decision_locator_pages_are_bounded": locator_metrics.max_page_entries <= 64 && locator_metrics.max_page_encoded_bytes <= 1024 * 1024,
        "decision_restart_queries_completed": locator_metrics.exact_restart_queries > 0,
        "decision_gc_reachability_is_complete": locator_metrics.reachable_blob_locators == locator_metrics.entries,
        "real_paged_checkpoint_pages_are_bounded": checkpoint_metrics.max_state_page_bytes <= 4 * 1024 * 1024,
        "real_paged_checkpoint_directory_is_paged": checkpoint_metrics.decision_directory_pages == locator_metrics.locator_pages.div_ceil(1_024),
        "real_paged_checkpoint_restore_matches": checkpoint_metrics.restored_root_matches,
        "real_paged_checkpoint_replay_matches": checkpoint_metrics.replayed_root_matches,
        "real_paged_checkpoint_repeat_delta_is_empty": checkpoint_metrics.repeat_delta_pages == 0,
        "real_paged_checkpoint_repeat_reads_only_manifest_paths": checkpoint_metrics.repeat_provider_calls
            <= checkpoint_metrics.decision_directory_pages.saturating_add(16),
        "real_paged_checkpoint_single_page_change_reads_only_touched_paths": checkpoint_metrics.single_page_change_provider_calls
            <= checkpoint_metrics.decision_directory_pages.saturating_add(20),
        "trace_heavy_locator_uses_indexed_commit_path": trace_metrics.hot_trace_entries == keys as u64
            && trace_metrics.indexed_lookup_samples > 0
            && trace_metrics.archive_commit_entries == 1
            && trace_metrics.target_archived,
    });
    let passed = gates
        .as_object()
        .is_some_and(|values| values.values().all(|value| value == true));
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "format": 8,
            "keys": keys,
            "elapsed_ms": elapsed_ms,
            "metrics": metrics,
            "decision_locator": {
                "elapsed_ms": locator_elapsed_ms,
                "metrics": locator_metrics,
                "estimated_resident_structural_bytes_per_entry": locator_metrics
                    .estimated_resident_structural_bytes
                    .checked_div(locator_metrics.entries.max(1))
                    .unwrap_or(0),
                "locator_pages": locator_metrics.locator_pages,
            },
            "paged_checkpoint": checkpoint_metrics,
            "trace_heavy_locator": {
                "elapsed_ms": trace_elapsed_ms,
                "metrics": trace_metrics,
            },
            "structural_bytes_per_entry": structural_per_entry,
            "estimated_resident_bytes_per_entry": resident_per_entry,
            "all_patricia_structural_bytes_per_record": total_patricia_structural_per_record,
            "all_patricia_estimated_resident_bytes_per_record": total_patricia_resident_per_record,
            "gates": gates,
            "passed": passed,
        }))?
    );
    if !passed {
        return Err("one or more Format-8 Patricia scale gates failed".into());
    }
    Ok(())
}
