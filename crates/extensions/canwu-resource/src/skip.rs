//! The trace a resource packet leaves when it settles nothing.
//!
//! A packet the resource plugin cannot settle for an expected reason never
//! fails its boundary. It changes nothing in the resource root, its boundary
//! commits, and the plugin emits one [`RESOURCE_INGRESS_SKIPPED_EVENT`] that
//! names the packet, the operation key it carried, and why it was skipped.
//! A host application reads that event to learn what to send again. Work
//! another plugin scheduled is never handed to the host: when it does not
//! fit the root, the plugin defers it to a later instant itself, and the
//! event says until when.

use crate::{PLUGIN_NAME, ResourceOperationKey};
use canwu_api::{IngressId, SimEvent, SimTime};
use serde::{Deserialize, Serialize};

/// Event the resource plugin emits for every admitted packet that settles
/// nothing: one event per packet, in the boundary that admitted it.
///
/// Its summary has the form
/// `resource ingress {ingress} skipped ({reason}) [{operation key}] {detail}`,
/// or, for deferred work,
/// `resource ingress {ingress} deferred to minute {minute} ({reason}) [{operation key}] {detail}`,
/// where the operation key is empty for a packet that could not be decoded.
/// [`ResourceIngressSkipV1::from_event`] reads it back. Its affected entity is
/// the holder of a tracked command or of an allocation pass, when that holder
/// is a registered entity, so the holder can see that its request settled
/// nothing; the event of any other packet names no entity. The detail of the
/// event of a production output batch or an external participant step names
/// the holder of the participant the work belongs to: the run never checked
/// that holder against its entities, and an event naming an entity the run
/// does not hold would fail its boundary.
///
/// A resend of a request whose outcome is already recorded, in the root or
/// in the archive, also settles nothing. It leaves no event: the outcome under
/// its key is its own.
pub const RESOURCE_INGRESS_SKIPPED_EVENT: &str = "canwu.resource.ingress_skipped.v1";

/// Why a resource packet settled nothing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceIngressSkipReasonV1 {
    /// A different request holds the packet's operation key, in the root or
    /// in the archive. The outcome under the key is that request's; this
    /// packet's request never settles under the key.
    OperationKeyTaken,
    /// The packet names an operation key under
    /// [`crate::RESOURCE_COMPLETION_OPERATION_KEY_PREFIX`] on a request that
    /// is not a completion operation.
    ReservedOperationKey,
    /// The packet does not decode, or does not pass the checks its ingress
    /// makes at settlement: its binding to a tracked command, its provider
    /// and exact source, its certificate binding, its boundary time, the
    /// current version of the evidence it cites, or a credit of a production
    /// output batch that the root refuses. The event's detail names the
    /// check. Nothing was recorded, so a corrected request may use the same
    /// operation key.
    FailedSettlementChecks,
    /// What the operation would leave does not fit the archive-candidate or
    /// hot-outcome budget of the root, and an archive commit of every hot
    /// candidate would make room. Nothing was recorded; send the same request
    /// again once a commit has applied.
    ArchiveBacklog,
    /// What the operation would leave does not fit even beside an empty
    /// backlog: the room is reserved by admitted completion leases, or the
    /// budget is smaller than the operation. Room returns when leases
    /// complete, are released, or expire, or with a larger budget; an
    /// archive commit does not help until they do. For work an admitted
    /// lease certifies, such as a production output batch, the room is held
    /// by finished participants whose reports wait to publish; once they
    /// publish, an archive commit makes room. The detail says when the work
    /// would not fit even with an empty backlog and every reservation free:
    /// work a plugin scheduled is then skipped, not deferred.
    ReservedCapacity,
    /// What the operation would leave passes another limit of the root, in
    /// practice its state byte cap. Work a plugin scheduled waits for room
    /// until its deferral delay has reached 1,024 minutes, and is skipped if
    /// it still does not fit after that wait.
    RootLimit,
}

impl ResourceIngressSkipReasonV1 {
    const ALL: [Self; 6] = [
        Self::OperationKeyTaken,
        Self::ReservedOperationKey,
        Self::FailedSettlementChecks,
        Self::ArchiveBacklog,
        Self::ReservedCapacity,
        Self::RootLimit,
    ];

    /// The stable code the skip event's summary carries.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::OperationKeyTaken => "operation_key_taken",
            Self::ReservedOperationKey => "reserved_operation_key",
            Self::FailedSettlementChecks => "failed_settlement_checks",
            Self::ArchiveBacklog => "archive_backlog",
            Self::ReservedCapacity => "reserved_capacity",
            Self::RootLimit => "root_limit",
        }
    }

    /// The reason a code names, or `None` for an unknown code.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.code() == code)
    }

    /// Whether the packet was skipped for room in the root, so that the same
    /// request settles once room returns.
    #[must_use]
    pub const fn is_capacity(self) -> bool {
        matches!(
            self,
            Self::ArchiveBacklog | Self::ReservedCapacity | Self::RootLimit
        )
    }
}

/// One resource packet that settled nothing, as its
/// [`RESOURCE_INGRESS_SKIPPED_EVENT`] names it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceIngressSkipV1 {
    /// The admitted packet. Its record in the ingress log holds the payload,
    /// and the command that produced it when it is a tracked command.
    pub ingress: IngressId,
    pub reason: ResourceIngressSkipReasonV1,
    /// The operation key the packet carried; for a production output batch,
    /// the key of the credit that decided the skip. `None` when the packet
    /// could not be decoded.
    pub operation_key: Option<ResourceOperationKey>,
    /// For work another plugin scheduled that did not fit the root: the
    /// instant the resource plugin settles it again. The plugin queued that
    /// retry itself; nothing is asked of the host application but room.
    pub deferred_until: Option<SimTime>,
}

impl ResourceIngressSkipV1 {
    /// Reads a skip back from its event, or returns `None` for any other
    /// event.
    #[must_use]
    pub fn from_event(event: &SimEvent) -> Option<Self> {
        if event.kind.plugin_identity() != Some((PLUGIN_NAME, RESOURCE_INGRESS_SKIPPED_EVENT)) {
            return None;
        }
        let rest = event.summary.strip_prefix("resource ingress ")?;
        let (ingress, rest) = rest.split_once(' ')?;
        let ingress = IngressId::new(ingress.parse().ok()?);
        let (deferred_until, rest) = if let Some(rest) = rest.strip_prefix("skipped (") {
            (None, rest)
        } else {
            let rest = rest.strip_prefix("deferred to minute ")?;
            let (minute, rest) = rest.split_once(" (")?;
            (Some(SimTime::from_minutes(minute.parse().ok()?)), rest)
        };
        let (code, rest) = rest.split_once(") [")?;
        let (operation_key, _detail) = rest.split_once(']')?;
        Some(Self {
            ingress,
            reason: ResourceIngressSkipReasonV1::from_code(code)?,
            operation_key: if operation_key.is_empty() {
                None
            } else {
                Some(ResourceOperationKey::new(operation_key).ok()?)
            },
            deferred_until,
        })
    }

    /// The event summary of this skip, with a human-readable `detail`.
    pub(crate) fn summary(&self, detail: &str) -> String {
        let disposition = self.deferred_until.map_or_else(
            || "skipped".to_owned(),
            |until| format!("deferred to minute {}", until.as_minutes()),
        );
        format!(
            "resource ingress {} {disposition} ({}) [{}] {detail}",
            self.ingress,
            self.reason.code(),
            self.operation_key
                .as_ref()
                .map_or("", ResourceOperationKey::as_str),
        )
    }
}
