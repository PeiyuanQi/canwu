# canwu-force-supply-reference

This unpublished integration is an independently compilable, replaceable
consumer of `canwu-resource`. It proves that the conserved resource extension
can support a second downstream domain without importing military doctrine into
the engine. Force profiles and requisition policies are compiled from exact
`CompiledEconomyReferenceContentV1` coverage cells; holder commands cannot
author applicability, deltas, cadence, rule revisions, or provenance digests.

The package keeps food, fodder, physical currency, ammunition, spares, fuel,
and authored other resources distinct. Requirements retain their own cadence,
buffer, tolerance, and consequence rules. Resource allocation, transfer, and
arrival states grant no force benefit. A consequence is applied exactly once
only after acknowledgement ingress resolves an exact terminal outcome from the
authoritative retained resource-provider record version. Packet-authored
outcome bodies and self-hashes are never authoritative.

Requisition is recorded as a bounded saga: resource consumption, force-local
consequence, externality intent, external application or rejection, and final
force acknowledgement remain separate save/replay-friendly records. The force
package never mutates population, cooperation, harvest, property, fiscal, wage,
arrears, or credit state. Settled hot work becomes a bounded package-owned
terminal receipt; receipts beyond the configured cap advance a deterministic
continuation chain.

Settled history leaves the root through a content-addressed package archive,
which the economy reference integration reuses. Each archive commit extends
a cumulative lookup index in the resource archive's format, so a boundary
that asks whether a key was archived reads the head directory, one index page
per level, and one blob on a hit, however many commits the run has made.
Preparing a commit reads the archive store, and the restore audit rebuilds
the head's index from the directory chain. A settled commit leaves a
maintenance receipt in the root until the host acknowledges the commit's
retention; the acknowledgement folds the receipt into the root's receipt
chain, so the root keeps only the receipts of unacknowledged commits however
many commits a run makes. An operation whose result finds no room in the
root is skipped with an `operation_skipped` event (`force_operation_skip`
reads it) and records nothing; once an archive commit has made room, the
command rebuilt against the root's current revision settles under the same
operation ID. `force_operation_outcome` reads a settled outcome, hot or
archived.

Reports are detached and holder-relative. They are assembled only from typed
knowledge publications available by the holder's observation cut, so current
force, shortage, saga, outcome, and acknowledgement truth cannot leak through
a delayed report. Snapshot, checkpoint, and replay validation re-resolve the
exact retained resource and economy provider versions cited by active or
archived work.
