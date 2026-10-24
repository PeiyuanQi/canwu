# Resource Continuation Evidence and Completion Reports

Status: implemented in 0.20.0. It closes downstream gaps G-77 and G-75 in
`canwu-resource` and `canwu-production`, G-74 in `canwu-production`, and G-76
in `canwu-technology`. A continuation checkpoint or a seal taken while a
completion lease's outcomes are hot now keeps proof of every record version
those outcomes cite, so the run restores, validates, and continues exactly. A
completion whose holder report has nothing to report from archives its
participant without the report and never wakes the report system again for
it; a holder that cannot receive knowledge is refused before a lease reserves
its report, never at publication. Production declares the two technology
record kinds it reads, and technology references a domain-entity holder or
site as its record.

It builds on [resource series scale](resource-series-scale.md) (0.18.0), whose
hot terminal and live buckets it extends, and on
[resource transit, place, and inflow](resource-transit-place-and-inflow.md)
(0.19.0), whose payload-required continuation it keeps unchanged. It follows
the engine rules that a boundary decision never depends on retained evidence
and that an expected refusal is a recorded rejection, never a failed
boundary.

## Problem and evidence

Each gap was verified in the published 0.19.0 source.

1. **A continuation checkpoint taken mid-lease did not load (downstream gap
   G-77, High).** The live root's payload-required continuation
   (`ResourceState::computed_continuation`) declared the evidence of open
   transfers and unreleased acquisitions only. Every hot operation outcome
   keeps `exact_evidence`: a cross-plugin completion's participant steps
   (`GrantExternalParticipant`, `PrepareExternalParticipant`,
   `ConsumeExternalParticipant`, `ReleaseExternalParticipant`) cite the
   coordinator's source, the production scope root's version at the step,
   and an output credit cites the root version the dispatch pinned. The
   production root moves on at its next write, so each cited version is
   superseded within the instant. A continuation checkpoint keeps only the
   versions current state declares, so it restored without them, and
   `validate_resource_runtime`, which a load runs, refused the run with
   `evidence_unavailable` ("resource operation exact evidence is
   unavailable"). A seal dropped them the same way. Every save taken after a
   production lease's first participant step failed to load while its
   outcomes were hot. A force-supply consumption, whose consumer evidence is
   the force version its grant locked, had the same exposure.
2. **A reserved report without an observation head re-woke the report system
   every boundary, and an organization holder failed the boundary
   (downstream gap G-75, High).** A completion lease reserves its holder's
   report, and the participant waits for that report before it archives. The
   report is published only from a provider-recorded observation head; while
   the holder's grant had none, `materialize_reports_on` kept the grant dirty
   and queued a wake at its cadence, so the report system ran and rewrote the
   live root on every boundary for the rest of the run. A report grant, a
   lease, and an external participant accept any `KnowledgeHolderRef`, but
   the engine refuses to publish to a core organization (and to a missing or
   dead person, a retired entity, or a domain entity whose schema admits no
   holders), so the report publication failed its boundary.
3. **Production did not declare two technology reads (downstream gap G-74,
   Medium).** `validate_authoritative_provider_binding` reads the
   `canwu_technology::AssetBinding` an evidence binding names, and adoption
   checks read the adoption's `ApplicationSpec`, but
   `production_external_state_keys` declared neither, so a `StartExecution`
   or `CreateFacilityProject` citing an asset binding failed its boundary with
   `UndeclaredStateRead`.
4. **Technology referenced domain entities as core targets (downstream gap
   G-76, Low).** `TechnologyRecordPayload::references` referenced every
   holder, operator, owner, adopter, and site through
   `DomainReferenceTarget::Core`, which the engine refuses for a domain entity
   ("domain entities must use domain-record references").

## Invariant

Every record version a hot resource record cites stays provably existing, by
identity, until that record leaves for the archive, whatever the run sealed or
checkpointed in between. A version is cited from the moment it is persisted:
by the hot terminal record that carries it, by the live-root state that locks
or names it, or, for a production output source, by the production root that
pinned it before the credit settles. Proof is identity only: no body is kept,
because no rule reads these versions' payloads after they were cited.

A completion's reserved holder report is either published from a head at or
after the completion's revision, or given up without a wake when there is
nothing to report from or no one to report to. No report publication fails a
boundary for its holder.

## Design

### Identity-only evidence of the resource root (downstream gap G-77)

The engine already offers a compact identity-only declaration: a record whose
schema declares the reserved property `canwu_identity_evidence_dependencies`
(`IdentityEvidenceDependenciesV1`) has its declared versions kept as
membership, without bodies, by every seal and continuation checkpoint, and
`domain_record_version_evidence_exists` answers `true` for them after a seal
or a restore. The resource root now declares one in two places:

- **Each hot terminal bucket** (`ResourceHotTerminalBucketV1::evidence`)
  declares the exact evidence of its entries,
  `ResourceTerminalArchivePayloadV1::exact_evidence`: an outcome's
  `exact_evidence`, a consumption's `consumer_evidence`, a loss's cause when
  it is a record version, a terminal transfer's `exact_evidence`, and an
  external participant's `coordinator_source`, which is also exactly what
  the record's archive entry carries as `exact_evidence`
  (`terminal_archive_record` builds it from the same function). The
  declaration lives with the
  records: a boundary that writes a bucket recomputes it from the entries the
  bucket keeps (those it attached, and the stored entries it did not, which it
  decodes), so a boundary still writes only the buckets it touches. When an
  archive commit takes a record out of its bucket, its evidence leaves the
  declaration with it.
- **The live root** (`ResourceState::identity_evidence`, computed by
  `computed_identity_evidence`) declares the external record versions its
  completion lease grants lock, and, for each external participant it holds
  open (held, prepared, or consumed) or holds completed and waiting for its
  report, the participant's coordinator source and the external record
  versions its own grant locks; and the source versions of every observation
  head. Each is a version the plugin required to be current when it recorded
  it. An activation certificate's locked targets are not declared: a local
  lease's are its grants' targets, and an external participant's certificate
  also names the targets of the coordinator's own grants, which the resource
  never proved and the coordinator may have superseded (and a seal dropped)
  before the certificate arrived. A released, expired, or rejected
  participant, which stays in `grants` for its coordinator's
  acknowledgement, declares nothing: nothing cites its versions. A
  participant that is a hot archive candidate is left to its bucket, so a
  partial state computes what the whole state does. The live root refreshes
  the declaration with its continuation, after every operation and archive
  commit, when a lease retires, and when a participant becomes an archive
  candidate; validation refuses a live root whose declaration differs.

A **live bucket** carries the reserved property too, since both bucket
families are one record kind, and it is always empty: a live record's
citations (an access grant's authority evidence) are read only while current.

Initial-scenario versions are never declared: they stay readable after any
seal, and the engine refuses them as declarations.

Every declared version is proven when it is first cited, so a declaration
never names a version the cut does not hold: a participant step's coordinator
source and a lease's locked external targets must be current, a force
consumption cites the version its grant locked while the grant declares it,
an output credit cites the source production declares (below), and, since
this change, every source version of a provider's `RecordObservation` head
must be its record's current version, and of a kind the resource plugin
reads, when the head is recorded. A head that cites anything else is the
operation's recorded rejection (`version_conflict`, or `invalid_authority`
for a kind the plugin does not read); only the current-version index is read,
so a sealed run and its replay decide alike.

The payload-required continuation is unchanged: it still declares the open
transfers' evidence and the unreleased acquisitions' envelopes, whose bodies
the plugin reads. A version can be declared by both; the engine keeps the
stronger requirement.

### Pending output sources in production (part of G-77)

An output credit cites the production source the dispatch pinned on the
execution (`output_source`): the root's version at the dispatch, which the
dispatch's own write supersedes. The resource bucket declares it only once the
credit settles, one boundary later at the same instant. A seal between the two
would drop it, and the credit's declaration would then name a version the cut
no longer holds, which refuses every later seal and checkpoint. The production
root therefore declares identity-only (its own
`canwu_identity_evidence_dependencies`) the output source of every execution
that has no output outcomes yet. Production compares the source by identity
only, so no body is kept, and once the output outcomes are acknowledged the
resource root's bucket carries the proof.

### Completion reports without a head, and holders that cannot receive knowledge (downstream gap G-75)

The row offered two shapes: let the completion publish its own holder report,
or archive the participant without one when the holder's grant has no head.
The engine publishes a resource report only from a provider-recorded
observation head, never from current truth, so a report the completion
published itself would be a second, unobserved source. This takes the second
shape:

- Several leases of one holder may reserve the same report grant. The report
  system takes, for a dirty grant, every completed lease whose reservation
  names the grant and is ready.
- When that grant has **no observation head**, or its **holder the engine
  would refuse** knowledge now
  (`SimulationView::knowledge_holder_accepts_publication`), it drops the grant
  from the dirty set with no wake, and every ready lease gives its reservation
  on the grant up (`ResourceState::release_completion_report`); for each lease
  whose last reserved report that was, the completed participant becomes an
  archive candidate without it. A provider that records a head later dirties
  the grant again, and its ordinary reports resume.
- Otherwise the one report from the grant's head is the report every ready
  lease completed at or before the head's revision waits for: it publishes as
  a mandatory report and releases all of them. A lease completed after the
  head's revision waits for a newer head as before; the provider's
  `RecordObservation` dirties the grant. Nothing wakes for it in between.
- An `Acquire`, a `SingleStepLease`, or a `GrantExternalParticipant` whose
  recipe reserves at least one holder report, for a holder the engine refuses
  now, is the operation's recorded `invalid_authority` rejection ("completion
  reserves holder reports for a holder that cannot receive knowledge"), on the
  completion ingress and on a tracked command. The lease, grant, and
  reservation do not exist.
- `ResourceState::install_report_grant` refuses a holder of a kind that can
  never hold knowledge (`holder_kind_can_hold_knowledge`: a core
  organization, resource, route, or territory, or a person given as an
  entity) with `InvalidDefinition`, and root validation refuses such a grant,
  so activation refuses a hand-built root that holds one. Whether a person or
  a domain entity can hold knowledge depends on the run, so a grant to one is
  installed and checked when it is used.
- Production's own site reports skip a holder the engine refuses, rather than
  failing the boundary.

### Declared technology reads (downstream gap G-74)

`production_external_state_keys` adds the `AssetBinding` and
`ApplicationSpec` state keys to the reads of the command handlers and the
lifecycle system. A cited version of another kind is refused before it is
read, as the operation's recorded `invalid_domain_record` rejection, since a
read of an undeclared kind would fail the boundary: an evidence binding must
name an asset binding, a capability qualification, an implementation, an
adoption, or an attempt observation; a technology binding's technique
revision, qualification, implementation, and adoption must be of those kinds;
an adoption's application must be an application spec, an observation's
attempt an experiment attempt, and a participant acknowledgement's provider
source the resource runtime.

### Domain-entity references in technology (downstream gap G-76)

`TechnologyRecordPayload::references` keeps every holder, operator, owner,
adopter, and site under the `core` role, whose targets are `AnyEntity`, and
references one that is a domain entity as `DomainReferenceTarget::Domain`
(its record), as `canwu-information` does; a core entity stays a core target.

## Persistence and replay

Nothing written by 0.19.0 is read; there is no migration.

| Item | Change |
| --- | --- |
| `RESOURCE_STATE_FORMAT_VERSION` | 6 (was 5). The live root gains `canwu_identity_evidence_dependencies` (`IdentityEvidenceDependenciesV1`, sorted, unique, no initial-scenario version), counted in `max_state_bytes` with the rest of the live root. |
| Resource record schemas | `canwu.resource` `runtime` declares both reserved properties (payload-required continuation and identity-only dependencies); `canwu.resource` `bucket` declares the identity-only one, required on both bucket families. |
| `RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION` | 2 (was 1). A hot terminal bucket has four fields: `format_version`, `bucket`, `entries`, and `canwu_identity_evidence_dependencies`, the union of its entries' exact evidence. A whole-root read (`resource_state`, `whole_resource_root_in_view`, and so `validate_resource_runtime`) and `ResourceState::from_records` refuse a bucket whose declaration is not the one its entries make; a boundary's partial read checks the field's shape only and rewrites the declaration whenever it writes the bucket. |
| `RESOURCE_LIVE_BUCKET_FORMAT_VERSION` | 2 (was 1). A live bucket carries the same field, always empty; any read refuses one that declares evidence. |
| Validation (`validate_within`) | Refuses a live root whose identity declaration differs from `computed_identity_evidence`, and a report grant whose holder kind can hold no knowledge. |
| Production runtime record | Its payload gains `canwu_identity_evidence_dependencies`, the pending output sources; its schema declares the property. The canonical-encoding check of restore and activation covers it. Production state formats are otherwise unchanged. |
| Archive | Unchanged: blob format 1; an archived entry carries the same `exact_evidence` as before, now built from `ResourceTerminalArchivePayloadV1::exact_evidence`; archive validation compares identities and digests and reads no cited version. |
| Kernel formats | Unchanged; no `canwu-sim` or `canwu-api` change. |
| Semantic hashes | `RESOURCE_SEMANTIC_HASH`, `PRODUCTION_SEMANTIC_HASH`, and the technology plugin's semantic hash change (below). |

What a seal and a continuation checkpoint keep for the resource root:

- **Bodies (payload-required, unchanged):** the exact and link-history
  evidence of every open transfer, and the exact evidence, capability
  bindings, and route evidence of every acquisition that is not released or
  expired.
- **Membership (identity-only, new):** every version a hot terminal bucket
  declares, every version the live root declares, and every pending output
  source a production root declares, until the declaring record no longer
  names it. A version that is current needs no membership; the engine keeps
  its body and identity as the current version.

Replay: each declaration is a function of the record that carries it, so it is
part of the state commitment, and an exact replay, a sealed run, its unsealed
twin, and a run restored from a continuation checkpoint hold the same
declarations and the same state hash. No boundary decision reads a declared
version: the resource plugin compares coordinator sources and locked targets
by identity and checks a cited version against the current-version index
(`require_known_version`, `is_current_version`), as before.
`validate_resource_runtime`, which a load runs, still requires every hot
outcome's exact evidence to exist, and now answers the same for a run, its
sealed twin, and its continuation restore.

## Refusals

- A completion operation that reserves holder reports for a holder the engine
  refuses: recorded `invalid_authority` rejection, which takes its operation
  key as every recorded rejection does.
- A report grant whose holder kind can hold no knowledge: refused by
  `install_report_grant` and by root validation, on the host before the run
  starts.
- A `RecordObservation` head with a source that is not its record's current
  version: recorded `version_conflict` rejection; a source of a kind the
  resource plugin does not read: recorded `invalid_authority` rejection.
- A production evidence binding, technology binding, adoption application, or
  observation attempt of the wrong record kind: recorded
  `invalid_domain_record` rejection, before anything is read.
- A reserved report with nothing to report from or no one to report to: not a
  refusal; it is given up, and the participant archives.

## What can still fail a boundary

The lists of [resource refusals without stalls](resource-refusals-without-stalls.md#what-can-still-fail-a-boundary),
[resource series scale](resource-series-scale.md#what-can-still-fail-a-boundary),
and [resource transit, place, and inflow](resource-transit-place-and-inflow.md#what-can-still-fail-a-boundary)
hold. This change adds nothing to them, and removes from them the report
publication to an ineligible holder and production's read of a cited record
of an undeclared kind. A hot terminal or live bucket whose stored
declaration is malformed is the existing "malformed hot terminal bucket"
class (`invalid_domain_record`), a tampered record.

A seal or a continuation checkpoint, which is not a boundary, is refused with
`ArchiveNotReady`, the run unchanged, when a declared version does not exist
at its cut. By construction a run's own boundaries never cite such a version
(every citation is proven when it is first persisted, see the design). A
scenario that installs hot records or heads citing boundary versions the run
never established still does, until the citing record is archived or
replaced. In 0.19.0 the same states sealed and failed on load instead.

## Behaviour changes for the versioning notes

- A completion's reserved report whose grant has no observation head, or
  whose holder the engine refuses, is given up when the completion makes it
  due: the participant archives without the report, and no report wake is
  queued for it. A downstream that records its holder's head the instant the
  participant completes, at or after the completion's revision, still gets the
  mandatory report.
- A dirty report grant with no head, or with an ineligible holder, leaves the
  dirty set at once, whatever the publication capacity.
- A lease acquisition, single-step lease, or external participant grant that
  reserves reports for a holder the engine refuses is a recorded
  `invalid_authority` rejection.
- `install_report_grant` refuses an organization, resource, route, or
  territory holder, and a person given as an entity.
- Several completions of one holder that reserved one grant are all served,
  or all given up, by the boundary that takes the grant (0.19.0 released one
  of them and left the others waiting).
- A `RecordObservation` whose head cites a source that is not current, or of
  a kind the resource plugin does not read, is a recorded rejection.
- A resource root near its byte cap reaches no room (`NoRoom` skips, or
  `limit_exceeded` where a limit is measured) a little earlier: its live
  root's identity declaration counts toward `max_state_bytes`.
- Production skips a site report to a holder the engine refuses, and refuses
  a cited version of the wrong record kind before reading it.
- Production commands and the lifecycle system may read technology asset
  bindings and application specs.
- Technology records reference a domain-entity holder, operator, owner,
  adopter, or site as its domain record.

## Public API delta

- `canwu-resource`: `ResourceState::identity_evidence` (field, serde
  `canwu_identity_evidence_dependencies`) and
  `ResourceState::computed_identity_evidence`;
  `ResourceHotTerminalBucketV1::evidence` and `ResourceLiveBucketV1::evidence`
  (serde `canwu_identity_evidence_dependencies`);
  `ResourceTerminalArchivePayloadV1::exact_evidence`;
  `holder_kind_can_hold_knowledge`; `install_report_grant` refuses a holder
  kind that can hold no knowledge; `RESOURCE_STATE_FORMAT_VERSION` 6,
  `RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION` 2,
  `RESOURCE_LIVE_BUCKET_FORMAT_VERSION` 2, a new `RESOURCE_SEMANTIC_HASH`.
- `canwu-production`: a new `PRODUCTION_SEMANTIC_HASH`; the runtime record's
  identity-only declaration.
- `canwu-technology`: a new semantic hash; record references as above.
- `canwu-api` and `canwu-sim`: no change.

Semantic hashes, each the BLAKE3 hash in lowercase hex of the UTF-8 string, on
one line:

- `RESOURCE_SEMANTIC_HASH`
  `6fbe03085ee5cde95721728f49cc32b6aa48f0eccc1ad2a17fd73f1f8e7f159a`:
  `canwu-resource plugin semantic hash 0.20.0: identity-only evidence of hot terminal records and held completion state, current observation sources, completion reports given up without an observation head, report holders that cannot receive knowledge refused`.
- `PRODUCTION_SEMANTIC_HASH`
  `eded50444992d0a08392ecf601ab9b38954d244e65aa8f06f83ed3ef3f8f6d9e`:
  `canwu-production 0.20.0 semantics: asset binding and application spec reads declared, cited record kinds checked before reading, pending output sources declared identity-only, report holders that cannot receive knowledge skipped`.
- Technology plugin semantic hash
  `8b4703e2106176664e83721607e8e997bf7323347bb1d054142c4fc7604dce3a`:
  `canwu-technology 0.20.0 semantics: domain-entity holders, operators, and sites referenced as domain records`.

## Durable tests

- `canwu-production` `g3_contract`
  `continuation_checkpoints_through_a_live_completion_restore_and_continue_exactly`
  (G-77): a live completion (the completion, its output dispatch and credit,
  the participant's completion, the report, and the release) runs on a
  compact run that seals after every boundary; a continuation checkpoint
  taken after every boundary restores, passes `validate_resource_runtime`,
  and continues to the same checkpoint hash and state hash as the run it was
  taken from. The credit cites a production version the root has superseded.
  On 0.19.0 the third checkpoint failed validation; without the production
  root's pending output source, the seal after the credit is refused.
- `canwu-production` `g3_contract`
  `a_completion_whose_report_grant_has_no_head_archives_its_participant_and_stays_quiet`
  (G-75): the completed participant becomes an archive candidate, no
  reservation, readiness, or dirty grant remains, no report wake is queued,
  and later boundaries leave the resource live root's version unchanged. On
  0.19.0 the grant stayed dirty and woke every boundary.
- `canwu-production` `g3_contract`
  `continuation_checkpoints_through_live_participant_steps_restore_and_continue_exactly`
  (G-77): the canonical coordinator drives the resource participant's grant,
  prepare, and consume by live ingress, with a production write between
  steps, and a provider records an observation head whose source the next
  production write supersedes; the run seals after every boundary, and a
  continuation checkpoint taken after every boundary restores, validates, and
  continues to the same hashes. Every step outcome and the head cite a
  superseded boundary version. Without the bucket declarations the restored
  runs fail validation.
- `canwu-production` `g3_contract`
  `a_tampered_evidence_declaration_is_refused_when_the_roots_are_installed`
  (G-77): a hot terminal bucket declaring other evidence than its entries
  cite, a live bucket declaring any, a live root whose declaration is not the
  computed one, and a production root whose declaration is not canonical are
  each refused at installation, with their own message.
- `canwu-resource` `gap_g75_resource_report_holder` (G-75): an organization
  report grant is refused at install; of two completion acquisitions that
  reserve a report, the one whose holder is not in the run is a recorded
  `invalid_authority` rejection that reserves nothing, the other applies, and
  the boundary settles; two completed participants of one holder that
  reserved the same headless grant both give their reports up in that
  boundary and both become archive candidates (one only, before the fix).

G-74 and G-76 have no committed test (the owner asked for no extensive
testing): each is a declaration or a guard whose effect is the engine's
existing read gate and reference rule.

## Known limits

- The bucket declarations are not counted in `max_state_bytes`; each repeats
  evidence its entries already carry, so a bucket's declaration is never
  larger than its entries. The live root's declaration is counted and repeats
  the versions its grants, participants, and heads name.
- A version a hot record cites is kept as membership for as long as the
  record is hot. A run that never archives keeps it until the root's
  candidate budget forces a commit.
- A reserved report whose grant has an older head waits for the provider to
  record a newer one; until then the completed participant stays in the live
  root (within the participant budget). Nothing wakes for it.
- A holder that becomes ineligible after its report was reserved has the
  report given up when it falls due; one that becomes eligible again gets
  reports only once its grant is dirtied again.
- External completion participants that end released, expired, or rejected
  stay in `ExternalCompletionParticipantBookV1::grants` for the run (since
  before 0.20.0): each coordinator (production, the force-supply reference)
  acknowledges a participant's end by citing the resource root's current
  version and reading the participant from that version's live `grants`, and
  the resource plugin learns of no acknowledgement, so retiring one would
  break an acknowledgement still to come. After 1,024 have accumulated
  (`MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL`), `grant` refuses every new
  participant (a recorded `limit_exceeded`). The fix is a retirement the
  coordinator triggers once it has acknowledged, a change to the production
  and force-supply coordinator protocols beyond this package. They declare no
  evidence meanwhile.
- In `canwu-resource`, a host-supplied version of a kind the plugin does not
  read (a lease's or participant's locked external target, an acquisition's
  envelope evidence, a prepare's targets) is still refused as a skip with a
  trace (`validate_completion_ingress`, `prepare_external_targets_current`)
  or, for an access grant's authority evidence and a tracked acquisition, at
  command admission; it never fails a boundary, but a skip leaves the
  operation key free rather than recording a rejection under it.
- Technology checks other than references (authority subjects, knowledge
  holders) are unchanged; whether every technology operation accepts a
  domain-entity holder end to end is not covered by a test.

## Review record

Two independent reviews of the first implementation (persistence half, API
half) approved it after fixes, all taken here:

- API: giving up or publishing a report released only one of the leases that
  reserved the grant; every ready lease is now released (tested). A cited
  production evidence or technology record of an undeclared kind failed the
  boundary with `UndeclaredStateRead`; kinds are now checked before the read.
- Persistence: the participant-step test now drives grant, prepare, and
  consume by live ingress with an observation head whose source is
  superseded; the four tamper cases are tested; the live root no longer
  declares released, expired, or rejected participants; an observation head's
  sources must be current when it is recorded; archive entries build their
  evidence from `exact_evidence`. Retiring ended participants from `grants`
  is recorded as a known limit (it needs a coordinator protocol change).
- The release continuation gate then found the live root declaring an
  external participant certificate's locked targets, which include the
  coordinator's own grants' targets that the resource never proved and a
  seal had dropped; the live root now declares only versions the plugin
  proved current when it recorded them.
