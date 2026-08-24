# Social Belief and Institutional Diffusion Framework

Status: implemented as a published experimental social diffusion simulation module and domain extension.

Implementation baseline: Canwu `main` at
`d026c669c91fcf5546a87c2b28e669b154a0745a` on 2026-08-20. DecisionTicket is
merged and is used through `canwu-api`. The separate information-lifecycle
design remains outside this implementation; society projections use the
existing authorized `ViewerContext` boundary and never fall back to truth.

## Decision

Do not add a `Religion` entity or religion-specific solver to Canwu core.

Prototype a generic, published experimental `canwu-society` social diffusion simulation
module as a domain extension that uses Canwu's public plugin, domain-record,
boundary, persistence, and replay contracts. Keep historical beliefs, rituals,
institutions, policies, people, and conflicts in downstream domain packages such
as Celestial Mandate.

Promote a capability into Canwu core only when an independent second system
needs the same primitive and the experimental implementation has produced
conformance evidence.

## Why this boundary

Three historical comparisons were used as non-deliverable design stress tests:

1. A hierarchical external mission validates assigned agents, elite access,
   cross-cultural adaptation, and negotiated legal presence.
2. A clandestine local network validates overlapping identity, family and
   locality ties, secrecy, detection error, suppression, and underground
   persistence.
3. A media-amplified confessional reform validates ideas spreading before
   organizations exist, variant branching, institutional adoption, public
   conformity versus private assent, and political protection.

Together they show that the reusable subject is not "religion". It is the
interaction among population distributions, affiliation targets, social and
information networks, organizations, institutions, policies, knowledge, and
named decisions.

## Hard gates

The implementation must preserve these existing Canwu contracts:

- Application-specific entity types and historical rules remain outside core.
- All authoritative changes enter through commands, canonical ingress, or
  phased boundary directives.
- Runtime handlers never receive mutable live state.
- Actor-facing reads never fall back to ground truth.
- Ordered collections, integer units, explicit tie-breaks, owned random
  streams, and atomic boundary commit preserve determinism.
- New state is covered by validation, persistence, hashing, exact replay,
  rollback, and plugin rehydration.
- A policy or ruler decision cannot instantly rewrite population belief.
- Affiliation counts cannot directly create rebellion or war.
- Public practice, private assent, organizational membership, and political
  mobilization remain distinct.
- Adding an observer, report, or presentation consumer cannot change
  authoritative results or random draws.

## Layering

```text
Canwu kernel
  stable identity, time, ingress, boundary settlement, allocation,
  actor knowledge, decisions, causality, persistence, replay, hashing

Experimental official domain extensions
  canwu-information   content, representations, transfers, access,
                      interpretation, releases
  canwu-society       social diffusion simulation module: cohorts, sparse
                      disposition distributions, social influence, organization
                      topology, institutional alignment

Downstream domain packages
  doctrine and ritual content, historical institutions and people,
  period policy meanings, cultural compatibility, balance coefficients,
  incidents, rebellion qualification, narration, UI
```

`canwu-society` should depend on the supported public API (`canwu-api`). It must not
be re-exported by `canwu-api`, and `canwu-api` must not depend on it. During the
experimental period it is published to crates.io while retaining an explicitly
unstable API surface.

The social diffusion simulation module may consume information-derived exposure
inputs through public records or ingress, but it must not own documents,
printing, interception, translation, or interpretation. It may produce
actor-estimate drafts, but the kernel owns authorization and actor-relative knowledge publication.

## Authoritative model

### Cohorts

`SocietyCohort` is an aggregate population identity, not an individual and not
a religious group.

```rust
pub struct SocietyCohort {
    pub id: SocietyCohortId,
    pub territory: EntityRef,
    pub headcount: u64,
    pub classification: BTreeMap<String, serde_json::Value>,
}
```

Classification is scenario data such as locality, occupation, status, language,
or migration background. The framework must not prescribe one universal set of
demographic fields.

### Affiliation targets

The society framework does not own doctrine. It refers to extension-owned
targets through a typed domain reference:

```rust
pub struct AffiliationTargetRef(DomainRecordRef);
```

A target may represent a belief variant, public confession, movement, school,
ritual association, or another domain-defined position. Variant ancestry,
claims, texts, and historical labels remain extension data.

### Multidimensional disposition

A single linear conversion stage is insufficient. For every active
`(cohort, affiliation target)` pair, the extension stores a sparse distribution
of headcount across disposition profiles.

```rust
pub struct DispositionProfile {
    pub awareness: AwarenessBand,
    pub assent: AssentBand,
    pub practice: PracticeBand,
    pub public_alignment: PublicAlignmentBand,
    pub organizational_tie: OrganizationalTieBand,
    pub mobilization: MobilizationBand,
    pub visibility: VisibilityBand,
}

pub struct DispositionBucket {
    pub profile: DispositionProfile,
    pub headcount: u64,
}

pub struct DispositionDistribution {
    pub cohort: SocietyCohortId,
    pub target: AffiliationTargetRef,
    pub buckets: BTreeMap<DispositionProfile, DispositionBucket>,
}
```

Only active `(cohort, target)` pairs and non-empty buckets are stored. For each
pair, bucket headcounts must sum to the cohort headcount. Different targets are
independent distributions, so one person may be represented as sympathetic to
or participating in more than one tradition without violating conservation.
An absent pair means that the scenario is not yet tracking that relationship;
the first relevant exposure materializes a distribution with the entire cohort
in the configured neutral profile.

Fractional expected-flow remainders are stored separately by stable transition
identity, rather than on a source bucket that may have several outgoing rules:

```rust
pub struct TransitionRemainder {
    pub cohort: SocietyCohortId,
    pub target: AffiliationTargetRef,
    pub rule: String,
    pub from: DispositionProfile,
    pub to: DispositionProfile,
    pub remainder: u32,
}
```

Profile bands and transition rules should initially be versioned extension
configuration rather than fixed Canwu-core enums. The Rust sketch communicates
the required semantic separation, not a frozen public shape.

### Social influence

`SocialInfluenceEdge` describes an opportunity for exposure or reinforcement.
It does not itself convert population.

```rust
pub struct SocialInfluenceEdge {
    pub source: EntityRef,
    pub target: EntityRef,
    pub channel: String,
    pub reach_per_mille: u16,
    pub trust_per_mille: u16,
    pub access_cost: u64,
    pub observability_per_mille: u16,
}
```

Channels are capability profiles, not era labels. A family tie, public sermon,
school, market route, printed edition, patronage link, or clandestine meeting is
configured by the downstream package.

### Organization topology

Organizations use stable nodes and explicit edges rather than ownership trees.
This supports hierarchy, federations, local congregations, informal teachers,
and clandestine cells with the same records.

Required relations include delegation, sponsorship, doctrinal influence,
resource support, reporting, and concealment. The framework provides topology
and deterministic traversal; domain packages define the meaning and effects of
specific relations.

### Institutional alignment

An institution can publicly support an affiliation target without implying that
its population privately assents.

```rust
pub struct InstitutionalAlignment {
    pub institution: EntityRef,
    pub target: AffiliationTargetRef,
    pub support_per_mille: u16,
    pub enforcement_per_mille: u16,
    pub access_grant_per_mille: u16,
    pub visibility: CommitVisibility,
}
```

Leadership decisions alter alignment, appointments, access, and policy inputs.
They do not mutate cohort dispositions directly.

### Policy pressure

The framework should store orthogonal policy capabilities rather than a fixed
historical enum such as "tolerated" or "heretical":

- institutional support
- legal access
- surveillance
- censorship
- coercive enforcement
- material penalty
- organizational disruption
- migration pressure

Downstream packages can label combinations as establishment, toleration,
restriction, criminalization, or persecution.

### Derived outputs

The following are calculated outputs, not independent ground truth:

- exposure pressure by cohort and target
- proposed disposition transfers
- public activity estimates
- underground persistence estimates
- organization reach and resilience
- institutional enforcement reach
- cultural or policy tension
- mobilization candidates

A mobilization candidate contains evidence and capability, not a rebellion
result. Political, security, diplomatic, and military packages decide whether a
candidate becomes protest, repression, migration, riot, coalition, or war.

## Boundary execution

The intended fourteen-phase integration is:

1. Ingress admits publications, reports, appointments, policy orders, and
   scheduled social work.
2. Snapshot freezes authoritative cohorts, networks, alignments, and policies.
3. Derived solve computes exposure opportunities and institutional reach.
4. Perception and attention consume permitted information inputs and prepare
   actor-specific observations.
5. Decision intake admits choices by named people, councils, and organizations.
6. Reservation allocates scarce agents, meeting capacity, publication capacity,
   patronage, administration, surveillance, and enforcement resources.
7. Domain delta proposes disposition transfers, network changes, organization
   lifecycle changes, and institutional alignment changes.
8. Invariant validation checks conservation, reference validity, ownership,
   declared writes, policy bounds, and visibility.
9. Atomic commit installs ordinary changes.
10. Historical candidate evaluation computes incidents and mobilization
    candidates from committed facts.
11. Conditional transition commits separately qualified transitions.
12. Strategic aggregation derives public adherence, underground activity,
    organization reach, tension, and institutional control.
13. Perspective and report materialization publishes actor-relative estimates.
14. Save, replay, and hashing bind all authoritative state and evidence.

Mass population movement should use deterministic expected transfers with
integer remainders. Scoped randomness is reserved for discrete incidents and
must use separate named streams for exposure, detection, interpretation, and
incident qualification so unrelated work cannot perturb results.

## Decisions

The merged `DecisionTicket` contract is the institutional decision boundary.
The society package generates domain-specific context and options; it does not
implement controller policy.

Appropriate tickets include:

- assign, recall, protect, or restrict a named propagator
- publish, translate, conceal, or withdraw a representation
- tolerate, monitor, suppress, negotiate with, or sponsor an organization
- adopt or reject an institutional alignment
- expand, split, merge, migrate, or go underground

Population cohorts never receive one decision ticket per person. Their changes
come from deterministic aggregate transition systems after named decisions and
environmental inputs have been admitted.

## Knowledge and information

The information lifecycle and belief lifecycle must stay distinct:

```text
Information access != interpretation
Interpretation != actor knowledge
Actor knowledge != private assent
Private assent != public alignment
Public alignment != organization membership
Organization membership != mobilization
Mobilization != rebellion
```

Ground-truth membership and actor estimates must be separate. Officials,
leaders, rival organizations, and ordinary observers may have different,
contradictory estimates with different confidence, fidelity, source, observed
time, and learned time.

Until generic actor knowledge publication exists, experimental authoritative
simulation may proceed, but no semantic or player-facing API may expose raw
society domain records as an actor observation.

## Public extension surface

The first experimental API should be narrow:

```rust
SocietyPluginBuilder
SocietySchemaSet
SocietyCohortDraft
AffiliationTargetRef
DispositionDistributionDraft
SocialInfluenceEdgeDraft
OrganizationTopologyDraft
InstitutionalAlignmentDraft
PolicyPressureDraft
TransitionRuleSet
SocietyQuery
SocietyProjection
```

The public API should accept owned, serializable values and batch operations.
It must not return mutable stores or expose internal solver caches.

Rule implementations remain stateless. Deterministic state, remainders, and
stream positions live in persisted plugin-owned records or components.

## Design stress tests (not implementation deliverables)

The following profiles explain why the abstraction has its current shape. They
are not implementation tasks, executable fixtures, or CM content requirements.
Historical names, places, eras, doctrine, political outcomes, and narrative
mapping remain outside this Canwu work item. The implementation provides one
neutral composite tutorial, not one tutorial per comparison.

### Profile A: hierarchical mission

- an external sponsor assigns named agents
- agents travel through routes and require local access
- elite sponsorship changes reach but not automatic assent
- translation or interpretation quality changes exposure fidelity
- local policy can tolerate, restrict, or expel agents

### Profile B: clandestine network

- family and locality edges spread participation
- public practice and private tie can diverge
- surveillance produces false positives and false negatives
- suppression can reduce visible activity while underground persistence remains
- state classification can group organizations that are not organizationally
  unified

### Profile C: media-amplified reform

- content spreads before a stable organization exists
- copied representations use information-network capacity
- affiliation variants branch and compete
- a ruler or council changes institutional alignment through a decision
- public conformity changes faster than private assent
- policy protection or persecution affects access, migration, and organization
  survival without instantly changing belief

Any future downstream implementation of these profiles should be representable
without changing the society solver. The current implementation does not need
to build or ship them.

## Single tutorial case

The documentation deliverable contains exactly one neutral case:

**Local community diffusion and institutional response**

The case demonstrates one small settlement over several boundaries:

1. two population cohorts begin with different awareness and private assent;
2. a local organization and two social influence edges create exposure;
3. an institution makes a DecisionTicket-backed policy choice;
4. public alignment changes faster than private assent;
5. an authorized observer receives an estimate that differs from ground truth;
6. extension-validated save/load, fork, and exact replay reproduce the result.

It does not reproduce any of the three historical comparisons, trigger rebellion
or war, introduce historical terminology, or become a multi-case library. The
runnable example and its Chinese/English documentation count as one case.

## Conformance evidence

The implemented domain-extension tests prove:

- every active `(cohort, target)` distribution conserves cohort headcount
- partial transfers preserve integer remainders across save and load
- adding an unrelated target does not change existing results
- policy changes affect incentives and access, not instant private assent
- institutional alignment and population disposition remain separate
- actor projections never expose ground-truth membership without evidence
- payload core-entity references and persisted derived values are recomputed at
  the module load boundary
- pending institutional policy components are validated with the root state at
  the module load boundary
- explicit command authority cannot substitute for engine-issued
  DecisionTicket provenance
- inactive organizations cannot receive or relay organization strength
- EPOCH and negative-time boundaries remain valid simulation times
- domain-extension-validated snapshot load, exact replay, and fork reproduce the same
  state

The crate defines mobilization candidates but no conflict type or conflict
emission path. Political conflict qualification remains a downstream
architectural boundary rather than a society test assertion.

The engine workspace separately covers generic rollback, snapshot commitment
tamper rejection, and plugin manifest validation. Structural performance tests
verify that active signal and narrow observer-projection indexes grow with their
active outputs, while inactive catalog growth does not materialize a dense
`territory * cohort * target * channel` matrix.

## Implementation order

1. Approve this proposal without declaring public API stability.
2. Coordinate with the decision-framework worktree and information design so
   persistence, knowledge, and decision contracts are not implemented twice.
3. Build the published experimental `canwu-society` social diffusion simulation module as
   a domain extension on the existing plugin and domain-record contracts.
4. Implement cohorts, sparse multidimensional disposition distributions, and
   deterministic transition remainders.
5. Implement social influence, organization topology, institutional alignment,
   and orthogonal policy pressure.
6. Use the merged DecisionTicket contract and the existing authorized
   `ViewerContext` boundary; keep the separate information-lifecycle design out
   of this implementation.
7. Add focused generic conformance tests for conservation, determinism,
   actor-relative privacy, rollback, save/load, and exact replay. These tests do
   not reproduce the three historical comparisons.
8. Add the single neutral tutorial case described above.
9. Measure sparse scaling and finish repository verification.
10. Stop before historical content, CM integration, additional tutorial cases,
    public release, or core promotion; each requires a separate explicit scope
    decision.

## Hidden cost

The largest cost is state-space growth: cohorts multiplied by active targets,
disposition profiles, social edges, organizations, institutions, and observer
estimates. Sparse storage, bounded profiles, batch queries, and explicit
performance evidence are architectural requirements, not later optimizations.

The second cost is false generality. The historical comparisons can still share
accidental assumptions. Focused generic tests should therefore vary organization
topology, policy enforcement, public/private divergence, and observer access
independently without recreating those histories.

## Failure modes and stop conditions

Stop and redesign if any of the following occurs:

- Canwu core gains named religious entities, doctrines, rituals, or eras.
- A ruler or policy write directly sets population belief percentages.
- one scalar "acceptance" value is required to explain all behavior
- public practice is treated as proof of private assent
- ground-truth membership is returned by an actor-facing query
- rebellion is emitted directly by the society solver
- a domain scenario requires a scenario-ID branch in shared code
- the solver requires a dense world-sized matrix
- adding an unrelated observer changes authoritative RNG or outcomes
- a snapshot cannot rehydrate the exact plugin contract and remainder state

## Approval boundary

The user approved the bounded implementation scope. This document now records
the implemented experimental domain extension and its remaining stop boundary:
historical content, CM integration, additional tutorials, publication, and core
promotion still require separate approval.

Because the eventual work changes public APIs, actor knowledge, persistence,
replay, and possibly conservation contracts, repository rules require an
independent review before commit. That review also requires explicit user
authorization before a review subagent is started.
