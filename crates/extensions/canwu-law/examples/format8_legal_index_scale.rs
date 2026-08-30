use canwu_law::format8_legal_temporal_scale_probe;
use canwu_law::{
    LegalTemporalQueryBudget, MAX_LEGAL_ARCHIVE_PAGE_BYTES, MAX_LEGAL_ARCHIVE_PAGE_ENTRIES,
};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let versions = std::env::args()
        .nth(1)
        .map_or(Ok(1_000_000_usize), |value| value.parse())?;
    let started = Instant::now();
    let metrics = format8_legal_temporal_scale_probe(versions)?;
    let elapsed_ms = started.elapsed().as_millis();
    let expansion_passed = metrics.max_interval_expansion <= 128;
    let authenticated_pages_present =
        metrics.membership_pages > 0 && metrics.effective_pages > 0 && metrics.recorded_pages > 0;
    let bounded_pages = metrics.max_membership_page_entries
        <= MAX_LEGAL_ARCHIVE_PAGE_ENTRIES as u64
        && metrics.max_temporal_page_entries <= MAX_LEGAL_ARCHIVE_PAGE_ENTRIES as u64
        && metrics.max_membership_page_encoded_bytes <= MAX_LEGAL_ARCHIVE_PAGE_BYTES as u64
        && metrics.max_temporal_page_encoded_bytes <= MAX_LEGAL_ARCHIVE_PAGE_BYTES as u64;
    let bounded_hot_candidates = metrics.peak_hot_compaction_candidates <= 4_096;
    let query_budget = LegalTemporalQueryBudget::default();
    let bounded_query_io = metrics.point_query_max_provider_calls
        <= query_budget.max_provider_calls as u64
        && metrics.point_query_max_segments <= query_budget.max_segments as u64
        && metrics.point_query_max_decoded_bytes <= query_budget.max_decoded_bytes;
    let expected_provider_index_entries = metrics
        .source_versions
        .saturating_add(1)
        .saturating_add(metrics.membership_pages)
        .saturating_add(metrics.effective_pages)
        .saturating_add(metrics.recorded_pages);
    let provider_backing_complete = metrics.provider_backing_store_bytes > 0
        && metrics.provider_index_entries == expected_provider_index_entries;
    let retention_bounded = metrics.retention_handles == metrics.archive_batches
        && metrics.retention_committed_roots == 1
        && metrics.retention_committed_objects == metrics.source_versions
        && metrics.retention_terminal_payload_items == 0;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "format": 8,
            "versions": versions,
            "authenticated_legal_archive_index": {
                "elapsed_ms": elapsed_ms,
                "metrics": metrics,
            },
            "gates": {
                "maximum_interval_expansion_at_most_128": expansion_passed,
                "bucket_entries_not_less_than_source_versions": metrics.bucket_entries >= metrics.source_versions,
                "all_authenticated_page_families_present": authenticated_pages_present,
                "all_legal_archive_pages_are_bounded": bounded_pages,
                "hot_compaction_candidates_are_bounded": bounded_hot_candidates,
                "sampled_queries_return_at_most_one_sparse_member": metrics.point_query_max_candidates <= 1,
                "sampled_queries_obey_provider_segment_and_byte_budgets": bounded_query_io,
                "canonical_ingress_retention_is_one_authenticated_root": metrics.canonical_ingress_retention_roots == 1,
                "disk_backing_indexes_only_current_pages_and_every_cold_blob": provider_backing_complete,
                "retention_ledger_keeps_one_current_root_and_compact_terminal_handles": retention_bounded,
            },
            "passed": expansion_passed && authenticated_pages_present && bounded_pages
                && bounded_hot_candidates
                && metrics.point_query_max_candidates <= 1
                && bounded_query_io
                && metrics.canonical_ingress_retention_roots == 1
                && provider_backing_complete
                && retention_bounded,
        }))?
    );
    if !expansion_passed
        || !authenticated_pages_present
        || !bounded_pages
        || !bounded_hot_candidates
        || metrics.point_query_max_candidates > 1
        || !bounded_query_io
        || metrics.canonical_ingress_retention_roots != 1
        || !provider_backing_complete
        || !retention_bounded
    {
        return Err("one or more Format-8 legal temporal scale gates failed".into());
    }
    Ok(())
}
