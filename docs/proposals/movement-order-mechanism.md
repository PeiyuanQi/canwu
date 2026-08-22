# Movement Order Mechanism

Status: implemented movement boundary. The runtime uses `OrderMovement` for
all built-in movement subjects; the old `MoveArmy` command format is not a
public compatibility surface.

## Decision

Canwu should generalize the movement mechanism, not treat every `EntityRef` as
a movable object.

The canonical command is `OrderMovement`. Its subject is an `EntityRef`, its
destination is a territory, and its optional cargo manifest contains typed
letter IDs. A client may offer convenience actions named `move_entity` or
`self_move`, but both submit the same command and never mutate positions
directly. Army movement retains army-specific transit events internally, while
person movement adds person transit and letter custody events under this same
boundary.

## Active and passive movement

`OrderMovement` means that a movement intent is admitted through the canonical
command boundary. It does not mean that another actor ordered the subject to
move.

The movement mechanism separates four facts:

1. **Decision origin**: who or what authorized the intent.
2. **Command subject**: the aggregate, person, cargo lot, or domain object the
   intent acts on.
3. **Movement capability**: which operation the origin is allowed to perform
   on that subject.
4. **Physical role**: whether the subject is self-propelled, carried, a cargo
   item, a carrier, or an attached participant.

For a person who voluntarily travels, the admitted authority is actor-bound
and the subject is the same person:

```text
decision_origin = Actor(person)
command_subject = Person(person)
capability      = SelfMove
physical_role   = MovablePrincipal
```

For a commander moving an army:

```text
decision_origin = Actor(commander)
command_subject = Army(army)
capability      = CommandUnit
physical_role   = Carrier or MovablePrincipal
```

For an institution dispatching a cargo lot, the origin and subject differ:

```text
decision_origin = Institution(institution, responsible_actor)
command_subject = Domain(cargo_lot)
capability      = MoveCargo
physical_role   = Cargo
```

Forced relocation, custody transfer, evacuation, and automatic migration are
also admitted movement intents. Their different initiative is derived from
the authority and domain capability; callers must not be allowed to claim
"self-directed" in an unchecked payload field.

Self-directed movement is therefore not a second scheduler or a special state
mutation. It is an ordinary `OrderMovement` whose authority binds the actor to
the subject and whose capability policy permits voluntary movement.

## Person carrying a letter

The concrete use case "a person personally travels to deliver a letter" is a
cross-domain operation with two linked lifecycles:

1. The movement domain admits one `Person` subject as a
   `MovablePrincipal`, with `SelfMove` authority bound to that same person.
2. The information or logistics domain admits the letter as a quantity-one
   cargo/dispatch and records the person as its expected custodian.
3. The movement execution changes the person's location or transit state and
   preserves the custody link; it does not mark the letter delivered.
4. At arrival, the information domain performs the recipient handoff and
   creates the normal delivery/access evidence. A failed trip, interception,
   or lost custody therefore cannot be mistaken for successful delivery.

If origin and destination are the same territory, the travel domain may admit
a zero-leg local movement or a separate intra-territory route policy. It must
still pass through the same authority, custody, and delivery evidence checks;
it must not bypass the command boundary by directly assigning
`Person.current_location`.

## Why not `MoveAnyEntity`

`EntityRef` is an identity union, not a mobility contract. It includes armies,
people, resources, governments, organizations, routes, territories, and
plugin-owned domain records. These kinds do not share location, custody,
quantity, lifecycle, or authority semantics.

The first movement capability matrix is:

| Subject | Default status | Owning layer |
| --- | --- | --- |
| Army | movable | built-in movement slice |
| Person | capability-gated movable | person/travel domain |
| Cargo lot or equipment | capability-gated, quantity-bearing | logistics domain |
| Carrier or convoy | capability-gated movable and carryable | logistics domain |
| Government or office | not a physical movement subject | political domain transitions |
| Territory or route | immobile topology | world/topology owner |
| Organization | capability-gated | owning domain |
| `EntityRef::Domain` | capability-gated | registering plugin |
| `ResourceId` without a domain record | unresolved | no movement authority |

Unknown or unregistered capabilities fail closed. Existence of an
`EntityRef` never grants movement rights.

## Extension contract

The transport extension request is intentionally an intent, not a mutable
position write:

```rust,ignore
pub struct MovementOrderRequest {
    pub subjects: Vec<MovementSubject>,
    pub origin: EndpointRef,
    pub destination: EndpointRef,
    pub mode: MovementMode,
    pub policy_version: String,
    pub expected_position_revision: u64,
}

pub struct MovementSubject {
    pub entity: EntityRef,
    pub role: MovementSubjectRole,
    pub quantity: Option<u64>,
    pub expected_custody: Option<CustodyRef>,
}
```

The richer route-plan types remain extension-owned. The core provides
admission, authority binding, transactionality, deterministic scheduling,
rollback, persistence, and replay; the owning domain provides the capability
descriptor, location/custody state, authority predicate, arrival effect, and
knowledge policy.

An execution must retain enough evidence to prove:

- subject manifest and capability descriptor;
- committed origin and expected position/custody revision;
- route-plan digest, topology version, knowledge cut, and policy version;
- departure/arrival times and deterministic schedule sequence;
- reservations, custody handoffs, reroutes, cancellations, and failures;
- command, cause, correlation, and completion identities.

## Invariants

- A subject has exactly one committed location or explicit in-transit/custody
  state.
- A subject cannot have two active movements unless its capability explicitly
  permits parallel legs.
- The order origin must match the committed position/custody revision.
- All manifest entries are validated atomically before any state write.
- Carrier and cargo custody cannot produce an arrival ordering contradiction.
- Cargo quantities are conserved except for explicit loss or consumption.
- Reroute creates an immutable itinerary revision; retry creates a new attempt
  or operation identity.
- Route estimates remain derived execution inputs and never rewrite an
  information lifecycle deadline.
- Every pending execution has matching order, schedule, state, and causal
  evidence after save/load and exact replay.
- Planning remains actor-relative and cannot silently read ground truth.

## Migration

1. Persist and validate `OrderMovement` as the only public movement command.
2. Bind person self-movement to `Issuer::Actor(person)` and
   `authority.command_subject == EntityRef::Person(person)`.
3. Keep army transit and person transit as separate state records, but use the
   same command admission, scheduler, rollback, persistence, and replay paths.
4. Treat a letter as explicit quantity-one cargo. Ordering changes custody to
   `InTransit`; arrival either delivers it or leaves it held at the destination.
5. Reject the old `MoveArmy` wire shape. Historical format loaders may be
   retired or migrated at an external import boundary, but the live runtime,
   snapshots, journals, and public serde types do not preserve that format.

## Required evidence before promotion

- self-directed person movement binds actor and subject and rejects mismatches;
- commander, institution, delegated, forced, and debug authority matrices;
- non-movable topology and unresolved resource rejection;
- one-active-transit and stale-position checks;
- atomic mixed-manifest rollback;
- custody and quantity conservation through handoff, reroute, cancellation,
  failure, retry, save/load, fork, and exact replay;
- deterministic equal-time scheduling and actor-relative route planning;
- deterministic `OrderMovement` command/event/replay fixtures for army and
  person subjects, including a person-carried letter.
