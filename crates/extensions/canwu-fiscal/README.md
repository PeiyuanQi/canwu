# canwu-fiscal

`canwu-fiscal` is the generic fiscal-procedure extension for Canwu. It models
versioned fiscal rules, regional adoption, assessments, remissions, execution
authorizations, evidence-backed receipts, audits, reform candidates, strategic
aggregates, and holder-relative estimated reports published as knowledge.

It deliberately does not own money, grain, inventory, market quotes, transport,
or physical transfer truth. A resource or logistics adapter performs the real
operation, then submits a `FiscalExecutionReceiptPacket` citing exact external
domain-record versions through `enqueue_execution_receipt`. At the live
settlement boundary the extension decodes the generic execution-evidence
envelope, validates every field against the authorization, and derives the
receipt quantity and disposition from the evidence. A configured evidence-kind
allowlist and the host integration's redundant semantic validator keep
arbitrary world records from proving execution. Exact evidence versions and
`(evidence kind, external_operation_id)` pairs can each settle at most one
receipt within one `FiscalState`.

Historical content periods and host-defined accounting cycles remain separate.
Aggregates retain institution, mechanism, scope, accounting cycle, unit, and payment-form
dimensions. Assessment, remission, collection, remittance, disbursement,
reserve, and return remain distinct; only remission and collection reduce an
assessment's outstanding quantity.

Historical packs compile into an immutable catalog containing their full time
range. `FiscalHistoricalContextPacket` changes the current year or replay mode;
reforms remain explicit candidates rather than automatic global date switches.
`ApplyTransition` installs its target rules and suspends superseded rules in one
settled action. Procedure commands use a dedicated revision, so derived report
or candidate refreshes do not make a valid player decision stale.

Fiscal collections are hard-capped at 4,096 assessments, 8,192 execution
requests, 8,192 receipts, 32 evidence versions per receipt, and a 32 MiB
serialized state budget. Aggregate rebuilds use single-pass indexes. A host
that needs larger campaigns must create separate simulation runs or shards;
this crate has no in-place partition or archive API, and the host owns
cross-shard operation-ID deduplication.
