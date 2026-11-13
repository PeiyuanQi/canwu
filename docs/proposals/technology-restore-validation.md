# Technology Restore Validation and Replay-Stable Citations

Status: implemented for the next release (0.23.0), unreleased. It closes
downstream gap G-95 (`canwu-technology`): a host can check the technology
state of a run restored from a continuation checkpoint, or of a sealed run,
which `validate_technology_runtime` could not. Closing it found two
determinism bugs in the technology plugin, fixed here too: a sealed or
continuation-restored run rejected later operations that its uninterrupted
twin applied, or failed every boundary with a technology operation. It
changes `canwu-technology` only: two public items, the plugin's citation
reads, the payload continuation its records declare, and its semantic
hash. No state shape, schema, snapshot, checkpoint, journal, or format
number changes.

## Problem

`validate_technology_runtime(canwu: &Canwu)` is the only technology check a
host can run after a load. It indexes the whole ingress log and the whole
boundary journal, replays every boundary's technology operations from the
scenario's records, and checks each consumed execution intent against its
ingress and its finalizer's boundary change. It therefore needs the full
`Canwu`.

A run restored from a continuation checkpoint is a `CompactedCanwu` with no
journal before its cut; a sealed `CompactedCanwu` keeps only the tail after
its last seal. Neither can be handed to the validator, so a host that
continues from a continuation checkpoint, the normal save of a long run,
loads technology state that no technology rule has checked. The
continuation-checkpoint package stated this as a non-goal
([continuation checkpoint](continuation-checkpoint.md), "Functions that read
history or exact version bodies keep `Canwu`"). The downstream restores from
continuation checkpoints and needs the technology rules checked there.

The plugin had the same blind spot at boundary time. Each technology
operation re-checks every live record's citations through `SimulationView`,
whose `evidence_exists` and `evidence_time` answer for a superseded record
version only while the run still keeps it. A seal or a continuation restore
drops a superseded version that no current record declares. Once a live
claim or assessment cites such a version as generic evidence, every later
technology operation on the sealed or restored run is rejected, while the
uninterrupted run applies it, and the two runs' state hashes part. A new
operation citing that version diverged the same way, and so did a new exact
citation of a superseded body that no current record declares. A host that
saves and loads through continuation checkpoints meets this in play.

The kernel keeps a body that a current record declares in its payload
continuation, but not the bodies that body declares once its own record
has moved on. A technology rule can read through such a body: a
transmission whose source is an implementation's first version checks that
version's qualification at every boundary, and the plugin loads every body
a loaded body cites. When the qualification is renewed and the
implementation moved onto the renewal, the first qualification is cited
only by the superseded implementation body. A seal or a continuation
restore drops it, and every later boundary with a technology operation
then fails on that run (`EvidenceContentUnavailable`), while the
uninterrupted run proceeds.

## Decision

### Restore validator

Add a restore validator over either handle:

```rust
pub trait TechnologyRestoreAccess { /* eleven reads, below */ }
impl TechnologyRestoreAccess for Canwu { /* ... */ }
impl TechnologyRestoreAccess for CompactedCanwu { /* ... */ }

pub fn validate_technology_runtime_on<H: TechnologyRestoreAccess + ?Sized>(
    run: &H,
) -> Result<(), CanwuError>;
```

It checks everything a run still holds and reads no journal entry at or
before the run's history cut:

1. **Live records.** Every current technology record's own invariants, as the
   boundary that applies an operation checks them: the shared total cap and
   the per-kind cap, payload size and reference limits, the canonical payload
   continuation of each record and of each exact version it cites, references,
   time windows, and evaluations recomputed from their evidence.
2. **Exact versions.** The body of every exact version a live record cites,
   and of each exact version those bodies cite. The kernel keeps the bodies a
   current record declares in its payload continuation; a missing body is
   refused, as the boundary that applies an operation refuses it.
3. **Generic evidence.** Each generic citation exists, and each observation,
   claim, and assessment cites only evidence established by its semantic time.
   A record version the run does not know is accepted by the identity rule
   below, as the plugin accepts it.
4. **Consumed execution intents.** Each consumed intent names the applied
   operation and the result that the same boundary established, the pending
   version just before it, and the result its request authorized; each
   applied provider operation consumed its intent, and no intent is consumed
   by two operations. The provider ingress envelope, and the finalizer's
   change in the consuming boundary, are each checked as the full validator
   checks them when the run still retains that journal entry.
5. **Knowledge.** Each published technology knowledge record's binding, and
   its payload against the body of the version it cites, or against that
   version's identity when the run keeps only the identity.

Provenance replay stays in `validate_technology_runtime`, the full-journal
mode: it needs the whole history, so a host that needs it keeps a full save.

### Replay-stable citations in the plugin

The plugin decides each operation through reads that do not depend on how
much evidence a seal or a restore kept (private `ReplayStableView`):

- a cited record version resolves through
  `SimulationView::replay_stable_domain_record_version` when a committed
  boundary, or the scenario, established it: the record's committed current
  version, or an initial-scenario version;
- an exact citation in a new operation, whose body a rule reads, must
  resolve that way, or be a body the plugin's loaded state already holds (one
  a current technology record declares, one such a body cites, or one
  hydrated for an earlier operation of the boundary), which the run keeps on
  every handle;
- a generic citation, and the semantic time of any cited version, otherwise
  falls back to the identity rule below;
- ordinal evidence (commands, attempts, events, ingress, boundaries, draws)
  answers from the journal counters and the time index as before, which a
  seal keeps.

The plugin still loads the bodies current records declare through the view,
as before: the kernel keeps them on every handle.

### Bodies read through a superseded version

A runtime technology record's payload continuation declares each version
it cites exactly and, for each cited version of a kind that a later
version can supersede (programs, execution intents, capability
qualifications, asset bindings, implementations, adoptions, transmissions),
the versions that body declares (private `continuation_versions`). The
kernel keeps every body a current record declares, so every body the
plugin loads, and every body a rule reads through a superseded version,
stays with the run on every handle while the citing record is current:
each loaded body is current, or a current record declares it together with
what it declares. Records of the other kinds are written once and stay
current, so their own continuations cover what they cite. The plugin
computes the continuation from the cited bodies it holds, or reads them
replay-stably, as the citation itself was checked. The intent finalizer's
consumed version declares what the pending version declares, plus the
operation and its result, which no later version supersedes. An operation
record still declares only its intent: a later intent version cites what
the earlier one cites. A rejected provider operation therefore names the
result's intent only when that is an execution intent the run keeps on
every handle (held, or current); otherwise it names none, since loading
follows the version it names. Both validators check each continuation
against that canonical form.

The full validator replays each boundary through the same reads, rebuilt
from the journal (private `ReplayStableRestoreAccess`: a version resolves
while it is the latest that an earlier boundary established), and it
reloads the held bodies at each boundary as the plugin does. Its final
checks, and the restore validator, accept a version the run does not know
by the same identity rule.

### The history cut

`TechnologyRestoreAccess::history_cut` is, for each journal, how many
entries the run no longer holds: the evidence cursor (sealed and retained
together) less the retained journal's length. It is zero on a full `Canwu`,
the last seal's cut on a sealed `CompactedCanwu`, and the continuation base
on a run restored from a continuation checkpoint (until it seals). The two
journal reads, `retained_ingress` and `retained_boundary`, answer only for
retained entries, and the restore validator asks them only for IDs after the
cut. The identity rule does not use the cut.

### The identity rule

One private function, `version_identity_boundary`, used by the plugin and by
both validators. A cited record version that the run cannot resolve is
accepted by its identity when all of these hold, and refused otherwise:

- a committed boundary established it (`BoundaryChange`), and its boundary
  ID and version are not zero;
- its record is current. Deletion keeps a versioned tombstone, so a record
  that a boundary ever wrote never leaves current state, and a record with no
  current version never had the cited version;
- it precedes the current version, and the current version was established
  by a boundary no earlier than it (a scenario-established current version
  has no earlier boundary version).

Every real version satisfies it or resolves replay-stably. Its time is its
establishing boundary's, which the time index answers on every handle.
Knowledge does not use the rule: the kernel keeps the identity of every
version committed knowledge cites, so knowledge accepts a version only when
the run knows it.

## Invariant

A sealed run, a run restored from a continuation checkpoint at any cut, and
the uninterrupted run decide every technology operation alike and reach
equal checkpoint hashes, as exact replay does. On a full `Canwu`,
`validate_technology_runtime_on` accepts what `validate_technology_runtime`
accepts, apart from provenance replay. On a `CompactedCanwu`, it accepts the
states an uninterrupted run reaches (except as
[what can still fail](#what-can-still-fail) says) and refuses every state
whose live technology records, retained bodies, retained journal entries, or
knowledge break a technology rule. No journal entry at or before the cut is
read.

## Ownership and affected surfaces

| Surface | Change |
| --- | --- |
| `canwu-technology` `query.rs` | `TechnologyRestoreAccess` and its implementations for both handles; `validate_technology_runtime_on`; private `RestoreCutAccess`, `TechnologyRecordSet::load_restored`, the restored consumed-intent check, and the retained-ingress envelope check. The shared `version_identity_boundary`, the plugin's `ReplayStableView`, the replay's `ReplayStableRestoreAccess` (with a per-record version history), and the identity fallback of the full-run evidence reads. The canonical continuation check reads the cited bodies. The finalizer and knowledge-binding checks are factored so both validators share them. |
| `canwu-technology` `plugin.rs` | the apply system decides operations through `ReplayStableView`; a new exact citation may also be a held body; drafts declare `continuation_versions`, and the finalizer's consumed intent what the pending one declares; a rejected provider operation names only an intent the run keeps; new semantic hash. |
| `canwu-technology` `model.rs` | private `supersedable_kind`, `continuation_versions`, and `declared_continuation`; a draft takes the versions its continuation declares. |
| `canwu-technology` `lib.rs` | re-exports the two public items. |
| `canwu-technology` `schema.rs` | none. |
| `canwu-api`, `canwu-sim`, other extensions, `canwu-debug`, examples | none. The trait lives in `canwu-technology`, which depends on `canwu-api`; it reads only public handle methods. |

## API surface

Added to `canwu-technology`, both re-exported from its root:

- `pub trait TechnologyRestoreAccess` with `committed_time`,
  `current_records_of_kind`, `current_record_version`, `record_version_body`,
  `record_version_known`, `evidence_known`, `evidence_established_at`,
  `committed_knowledge`, `history_cut`, `retained_ingress`, and
  `retained_boundary`; implemented for `Canwu` and `CompactedCanwu` by
  forwarding to the handle methods of the same meaning. It is object safe,
  so `&dyn TechnologyRestoreAccess` works. Its reads are trusted-host reads
  of ground truth.
- `pub fn validate_technology_runtime_on<H: TechnologyRestoreAccess + ?Sized>(run: &H) -> Result<(), CanwuError>`.

`validate_technology_runtime` keeps its signature; it accepts by the
identity rule what the plugin now accepts, and its rustdoc calls it the
full-journal mode. Refusals use `ErrorCode::InvalidDomainRecord`, as before.

The trait is a local trait rather than new `CanwuRun` methods: the reads it
needs (exact bodies, evidence identity and time, the retained journals, the
evidence cursor) already exist on both handles, and widening `CanwuRun` would
make every implementor add methods for one extension.

## Behavior that changes for an uninterrupted run

- A generic citation of a version the run never established, but that obeys
  the identity rule, is accepted. A sealed run cannot tell it from a real
  one, so deciding alike requires it.
- A new operation's exact citation of a superseded version that no current
  technology record holds is rejected (`invalid_domain_record`), where an
  unsealed run used to apply it. A sealed run no longer had the body.
- A provider result whose intent's program version is no longer current is
  rejected with the same code as before, with a different message.
- A citation of a version that owner-authorized maintenance wrote earlier
  in the same boundary is rejected; the plugin used to accept it. Only
  committed versions resolve. No system's proposal is visible to the apply
  system (the kernel extends the record overlay after every phase-7 system
  has run), and a command cannot name a version before it is written.
- A rejected provider operation's outcome record names no intent unless the
  result names an execution intent the run holds, or a current one; it
  used to name whatever version the result named.
- A runtime technology record's payload continuation also declares what
  each supersedable cited version declares, so its payload is larger, and
  the run keeps those bodies while it is current. A record near the
  16 KiB payload limit (`TechnologyLimitsV1::max_payload_bytes`) can now
  exceed it and be rejected. An adoption citing four implementations, each
  with a qualification and two assets, declares about 20 versions, some
  3.6 KB.

## Cost and complexity

With `r` live technology records, `e` exact bodies they cite transitively,
`g` generic citations, `i` consumed intents, `k` technology knowledge records,
and `t` the retained journal's length, the restore validator costs:

- loading: `O(r + e)` record reads, each a map lookup in the kernel;
- record checks: as each boundary that applies an operation runs them;
- consumed intents: `O(i)` body reads, plus `O(log t)` per retained ingress
  or boundary looked up;
- evidence and knowledge: `O(g + k)` identity and time lookups.

It reads no history. In the plugin, a cited version costs one replay-stable
read, and at most a boundary-existence read, one current-version read, and
one time read when it falls back to the identity rule: map lookups, as the
reads they replace. Writing a record reads the continuation of each
supersedable version it cites, one map lookup each, and the continuation
check at each boundary does the same for each loaded record. The full
validator reloads the held bodies at each replayed boundary, `O(e)` map
lookups per boundary, and finds a record's latest committed version by a
binary search of its version history, `O(log v)` for `v` versions.

## Persistence and replay

No state shape, record schema, snapshot, checkpoint, journal, or format
number changes. A runtime technology record's payload continuation, part of
its payload, declares more versions (see
[bodies read through a superseded version](#bodies-read-through-a-superseded-version)),
so the same operations write different payloads and the run's hashes
differ from 0.22.x. The technology plugin's semantic hash changes, from
`8b4703e2106176664e83721607e8e997bf7323347bb1d054142c4fc7604dce3a` to
`adab965f78db6d88d2da20afd376f8bbd05dd0d237503d32168e4958abbad44d`, the BLAKE3
hash in lowercase hex of the UTF-8 string, on one line:

`canwu-technology 0.23.0 semantics: domain-entity holders, operators, and sites referenced as domain records, cited record versions read replay-stably, older versions of current records accepted by identity, payload continuations that also declare what each supersedable cited version declares`.

The plugin version follows the crate version, which the release sets.
Same version only: a run is restored with the plugins it ran with, so a run
saved by an earlier release does not load. Exact replay is unaffected:
replay re-decides each boundary with the same reads.

## What can still fail

- A record whose payload, with its larger continuation, passes the 16 KiB
  payload limit is rejected (see
  [behavior that changes](#behavior-that-changes-for-an-uninterrupted-run)).
- When owner-authorized maintenance updates a record earlier in the same
  boundary, the plugin judges a forged identity of that record against the
  maintenance version, and the full validator's replay against the last
  committed version, so the full validator may refuse a run the engine
  produced. Every handle still decides alike, and a real citation is
  judged alike by both. Technology registers no maintenance; it needs
  another plugin's maintenance of a record that the forged identity
  names.
- A run whose kernel state is inconsistent (a retained journal longer than
  its evidence cursor) is refused with `InvalidDomainRecord` rather than read
  out of range.
- The validators need no plugin and read only committed state, so they
  answer alike before and after a host rehydrates plugins.

## Alternatives not taken

- **Make `validate_technology_runtime` generic over `CanwuRun`.** Its
  provenance replay needs the whole history, so it would still refuse every
  restored run; the replay must stay a full-journal mode.
- **Add the reads to `CanwuRun`.** See the API surface: wider, and every
  implementor pays for one extension.
- **Accept only current versions in new citations.** Deterministic and
  unforgeable, but it rejects a claim about a version a live record still
  holds, which runs used to apply (a claim citing a paused program's first
  version after a seal is a tested flow).
- **Declare generic citations as identity-only dependencies.** It would keep
  each cited identity across a seal and refuse forged ones, but it declares
  every generic citation on every record, far more than the exact closure
  adds, to refuse forgeries no first-party flow makes.
- **Keep transitive bodies in the kernel.** A seal could keep what each kept
  body declares, transitively, for every plugin. It is the general fix, but
  it changes `canwu-sim`'s continuation closure and every plugin's
  checkpoints, outside this package.
- **Declare the full transitive closure.** Following every cited body,
  not only supersedable ones, would declare whole technique lineages
  (revisions cite their parents) and pass the payload limit; a version of a
  write-once kind stays current, so its own continuation already covers
  what it cites.
- **Load only bodies a current record declares.** The plugin would skip a
  body a superseded version cites, alike on every handle, but rules read
  through those bodies (a transmission's source implementation's
  qualification), so runs would start refusing records they apply now.
- **Merge the two consumed-intent checks.** Making the full validator call
  the restore validator and then replay provenance would remove about 100
  duplicated lines, but it changes which refusal the full validator reports
  first. Left as a follow-up.

## Known limits

- **No provenance replay on a restored run.** Operation outcomes before the
  cut are not recomputed from their ingress. A host that needs that keeps a
  full save and runs `validate_technology_runtime`.
- **Before the cut, consumed intents are checked without their journal.**
  The provider ingress envelope and the finalizer's boundary change are
  checked only when retained. Before the cut, what binds an intent to its
  operation and result is the kernel's identity of their versions, and what
  the run held there is covered by the continuation hash the host stored, or
  by the sealed segments.
- **A forged identity that obeys the rule is accepted.** An older version
  number of a current record, from a boundary no later than the current
  version's, cannot be told from a real one without the full journal, so the
  plugin and both validators accept it on every handle. Its time is the
  forged boundary's, which can be earlier than the record existed, so a
  claim can cite it as evidence older than the record.
- **The trait is open.** An outside implementor must add any read a later
  release adds to it.

## Tests

`crates/extensions/canwu-technology/tests/restore_validation.rs`:

- **Twin runs decide alike**
  (`sealed_and_restored_runs_decide_later_operations_as_their_uninterrupted_twin`).
  A live claim cites a transmission's first version and the transmission
  closes; the run is sealed, and restored from its continuation checkpoint.
  On all three runs a transmission is then opened and a new claim cites the
  same version: both apply everywhere, and the checkpoint hashes are equal.
  On the old plugin the restored run rejected both.
- **Bodies read through a superseded version**
  (`sealed_and_restored_runs_keep_the_bodies_a_superseded_version_cites`).
  A transmission's source is an implementation's first version; the
  qualification is renewed and the implementation moved onto the renewal,
  so only the superseded implementation body cites the first
  qualification. After a seal, after a continuation restore, and on a run
  sealed before the move and again after it, a later technology operation
  applies as on the uninterrupted run, with equal checkpoint hashes. On the
  old plugin the sealed and restored runs failed the boundary
  (`EvidenceContentUnavailable`).
- **Rejected results name only kept intents**
  (`a_rejected_result_names_no_intent_the_run_does_not_keep`). A provider
  result names the superseded first version of that implementation as its
  intent and is rejected; its outcome names no intent, and later
  operations apply alike on the sealed and restored runs. The outcome used
  to declare that version, whose first qualification no current record
  declares.
- **New exact citations of superseded versions**
  (`new_exact_citations_of_superseded_versions_decide_alike_on_every_handle`).
  Two destination programs are paused after a transmission names the first
  as its outcome. A new transmission naming the held first version applies,
  and one naming the other program's first version is rejected, on the
  uninterrupted, restored, and sealed runs alike, with equal hashes. The old
  plugin applied the second on the uninterrupted run.
- **Agreement on generated runs.** Six seeds of generated operations
  (experiments whose provider results consume their intents, passive
  observations by a domain-entity knowledge holder, external-source
  transmissions opened and closed, claims, capabilities qualified and
  deactivated, programs paused, rejected operations). Both validators accept
  each full run, as does the trait object; the same state sealed, and
  restored from its continuation checkpoint, validates with no journal read.
- **Continuation.** Operations before and after a continuation checkpoint,
  restored as `CompactedCanwu`, including a provider result queued before
  the cut that consumes its intent after it, and knowledge citing a version
  the restored run knows by identity only. The restored run validates at the
  cut, after more operations, and after a further seal; a counting wrapper
  shows no ingress or boundary at or before the cut is read, and the entries
  after it are.
- **Identity.** A claim citing a superseded version validates on the
  restored and the sealed run, which no longer know the version. A claim
  citing a forged identity instead (the current version number from another
  change, an older version from a boundary after the current version's, or a
  version of a record that was never current) is refused on the full,
  restored, and sealed runs.
- **Refusals on both handles.** Records past the shared cap; a noncanonical
  payload continuation; citations of versions no boundary established (one
  not yet committed, a later version of a scenario-current record, version
  zero); a missing exact body; an external source the scenario did not
  establish (established later, or never); an intent consumed by two
  operations. Each is refused on the full run (by both validators where the
  kernel can hold the state) and on the restored run. State the kernel holds
  is installed through the scenario; state the kernel refuses to write into a
  checkpoint is presented by a host-side wrapper over each handle.

The framework tests keep passing, including a claim citing a paused
program's first version after a provider-backed seal, and an implementation
deactivated while it still cites a superseded asset binding it holds.

## Upgrading notes

A run restored from a continuation checkpoint, or sealed, now decides
technology operations as the uninterrupted run does. A host that restores
from a continuation checkpoint, or validates a sealed run, calls
`validate_technology_runtime_on(&run)` after the restore. Content that cites,
in a new operation's exact field, a superseded version no current
technology record holds now gets a rejection; cite the current version.
Content whose records come near the 16 KiB payload limit should check them
again, since a record now also declares what each supersedable version it
cites declares.

## Documentation delta

`docs/architecture.md` (the technology restore sentences), `docs/versioning.md`
(the next release's notes), the crate README, and in both languages
`website/src/content/docs/{,en/}architecture/technology.mdx` (restore and
replay) and `website/src/content/docs/{,en/}developer/persistence.mdx` (host
helpers on a continued run), the proposal index in `docs/README.md`, the
agent documentation map
(`agent-interface/plugins/canwu-engine/skills/canwu-engine-docs/references/documentation-map.md`),
plus the rustdoc. The
[continuation checkpoint](continuation-checkpoint.md) proposal stays as
written: it records the 0.16.0 decision this package revisits for technology.

## Terminology candidates

"history cut" (the cut below a run's retained journals) is used in the
rustdoc only; it is not added to `docs/terminology.md` until a second module
needs it.

## Release notes text (for `docs/versioning.md`)

- **Technology restore validation on either run handle** (downstream gap
  G-95; [proposal](proposals/technology-restore-validation.md)).
  `canwu_technology::validate_technology_runtime_on(&run)` checks the
  technology state of a `Canwu` or a `CompactedCanwu`, including one restored
  from a continuation checkpoint or sealed, without reading any journal entry
  before the run's history cut. The new trait `TechnologyRestoreAccess`,
  implemented for both handles, is what it reads.
- **Sealed and restored runs decide technology operations as the
  uninterrupted run does.** The plugin reads cited record versions
  replay-stably and accepts an older version of a current record by its
  identity. A record's payload continuation also declares what each
  supersedable version it cites declares, so the bodies a rule reads
  through a superseded version stay with a sealed or restored run. New
  technology semantic hash.

## Review record

One independent review by a sub-agent given the code and the contract, not
the author's reasoning: nothing blocking. Findings and what was done:

1. The identity rule accepted version zero and versions a current record
   cannot have had. Fixed: version zero is refused, and while its record is
   current a cited version must precede the current one. A forged identity
   that obeys the rule is a stated known limit.
2. The restored-handle paths were tested only on fixed sequences. Fixed: the
   generated test also validates each seed sealed and continuation-restored;
   the continuation test covers a provider result queued before the cut and
   consumed after it, and knowledge citing an identity-only version.
3. The docs did not mention the new API. Fixed (documentation delta).
4. The rustdoc overstated what the kernel keeps and coupled the two journal
   checks. Fixed.
5. Knowledge used the identity rule though the kernel keeps every identity it
   cites. Fixed: knowledge accepts only versions the run knows.
6. The "consumed more than once" map insert could never fire. Fixed: a
   comment says the operations loop enforces it.
7. The consumed-intent logic is duplicated. Left as a follow-up (alternatives).
8. Test details (labels from the brief, vacuous checks, names that
   overstated what they test, hash assertions that test the kernel). Fixed.
9. The trait is open. Stated in its rustdoc and the known limits.

A delta review of those fixes, by a second sub-agent reading the code: no
blocking or must-fix findings. It confirmed that the kernel never removes a
domain record from current state (deletion keeps a versioned tombstone), so
the identity rule accepts every state an uninterrupted run reaches. Four
small findings and what was done:

1. Only the scenario-established branch of the identity rule had a refusal
   test. Fixed: the identity test refuses, on the full, restored, and sealed
   runs, a forged current version number and an older version from a boundary
   after the current version's, beside the real identity it accepts.
2. A version of a record with no current version could pass. Fixed: the rule
   requires the record to be current, which closes the forged-identity case
   for records that never existed.
3. The website's "accepted by its identity" was unclear. Fixed: both
   languages state the rule.
4. The test file mixed section-comment styles. Fixed.

A third review, of the plugin fix, by a sub-agent at extra-high reasoning
effort, given the code and the requirement, not the author's reasoning:
nothing blocking. Findings and what was done:

1. Bodies cited only through a superseded body were still lost after a
   seal, and a normal flow reaches it (a transmission sourced from an
   implementation moved onto a renewed qualification): every later
   technology boundary failed on the sealed or restored run. Confirmed by a
   test and fixed: a runtime record's payload continuation also declares
   what each supersedable cited version declares.
2. The full validator found a record's latest committed version by a
   backward scan, quadratic over a long full save. Fixed: a per-record
   version history and a binary search.
3. The plugin and the full validator could judge a forged identity against
   different current versions within one boundary. On re-review this was
   mostly a false positive: the apply system sees no other system's
   proposal, only owner-authorized maintenance. An emulation added for it
   was wrong and was removed; the maintenance case is stated in what can
   still fail.
4. Tests lacked exact citations of held and unheld superseded versions.
   Fixed (the new exact citation test). No test drives the per-boundary
   reload of held bodies in the full validator's replay on its own.
5. Unlisted behavior changes and wording. Fixed: the behavior changes, the
   forged identity's time, and the rustdoc.

A re-review of those fixes by the same sub-agent: nothing blocking, and it
confirmed the continuation invariant (every body loading reaches is current,
from the scenario, or declared by a current record), the finalizer's
continuation, and that the full validator's replayed drafts match the
journal. Two further findings and what was done:

1. The same-boundary emulation was unsound (see 3 above). Removed.
2. A rejected provider operation recorded whatever version the result named
   as its intent, and loading follows it: a superseded non-intent version
   brought back the lost-body failure. Fixed: a rejected outcome names an
   intent only when it is an execution intent the run holds or a current
   one (the rejected result test). The chain test also seals a run before
   the move and again after it.
