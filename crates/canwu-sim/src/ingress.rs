use crate::{CommandRequest, PayloadSchema, SystemCadence};
use canwu_core::{EntityRef, IngressId};
use canwu_event::CauseRef;
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Reverse;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressClass {
    Command,
    Communication,
    Acknowledgement,
    Information,
    ScheduledSystem,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginIngressDescriptor {
    pub name: String,
    pub description: String,
    pub class: IngressClass,
    pub payload_schema: PayloadSchema,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PluginIngressRequest {
    pub plugin: String,
    pub packet_type: String,
    pub due_at: SimTime,
    pub priority: i32,
    pub payload: Value,
    pub affected_entities: Vec<EntityRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<CauseRef>,
}

impl PluginIngressRequest {
    #[must_use]
    pub fn new(
        plugin: impl Into<String>,
        packet_type: impl Into<String>,
        due_at: SimTime,
        payload: Value,
    ) -> Self {
        Self {
            plugin: plugin.into(),
            packet_type: packet_type.into(),
            due_at,
            priority: 0,
            payload,
            affected_entities: Vec::new(),
            cause: None,
        }
    }

    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub fn with_entity(mut self, entity: EntityRef) -> Self {
        self.affected_entities.push(entity);
        self
    }

    #[must_use]
    pub fn caused_by(mut self, cause: CauseRef) -> Self {
        self.cause = Some(cause);
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IngressPayload {
    Command {
        request: Box<CommandRequest>,
    },
    Plugin {
        plugin: String,
        packet_type: String,
        payload: Value,
        affected_entities: Vec<EntityRef>,
    },
    Calendar {
        cadences: Vec<SystemCadence>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IngressRecord {
    pub id: IngressId,
    pub issued_at: SimTime,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub eligible_boundary_count: u64,
    pub due_at: SimTime,
    pub class: IngressClass,
    pub priority: i32,
    pub payload: IngressPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<CauseRef>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IngressReceipt {
    pub ingress_id: IngressId,
    pub issued_at: SimTime,
    pub due_at: SimTime,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct IngressQueueKey {
    pub due_at: SimTime,
    pub class: IngressClass,
    pub priority: Reverse<i32>,
    pub issued_at: SimTime,
    pub id: IngressId,
}

impl IngressQueueKey {
    #[must_use]
    pub(crate) const fn from_record(record: &IngressRecord) -> Self {
        Self {
            due_at: record.due_at,
            class: record.class,
            priority: Reverse(record.priority),
            issued_at: record.issued_at,
            id: record.id,
        }
    }
}
