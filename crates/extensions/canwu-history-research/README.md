# canwu-history-research

Optional, unpublished historical research assessment plugins for Canwu.
They record source, practice, and production-archaeology assessments without
changing the authoritative outcome of `canwu-technology`.

`HistoricalSourcesPlugin`, `HistoricalPracticePlugin`, and
`ProductionArchaeologyPlugin` are separately selectable. Use
`HistoricalResearchSuite::plugins()` only when a run deliberately enables all
three. `HistoricalAnalysis` is a trusted-host read and never writes simulation
state.

Assessment commands are trusted-host ingestion: the declared assessor controls
authority, but the plugin does not claim that the assessor had player-visible
access to every cited source. A game that needs that stronger rule must prove it
in its own research-workflow plugin before submitting the assessment. Commands
cite durable evidence rather than transient ingress identities.

Restore through `from_historical_research_snapshot_json` or the corresponding
checkpoint/replay wrapper, or call `validate_historical_research_runtime`
after another module-owned loader. The validator rechecks record bounds,
dates, generic citations, and exact subject/contradiction/supersession evidence.
Every cited item must exist no later than the assessment's as-of cut.
Contradictions and supersessions may target only another assessment of the same
exact subject; they cannot rewrite an unrelated technology record.
Assessment ingress must match an admitted authority-checked command. Each
plugin is capped at 1,000 retained records and 21 mutations per boundary.
Overflow is retained as an explicit rejection event, leaving later boundaries
usable. None of these plugins changes base technology outcomes or belongs in the
simulation kernel.
