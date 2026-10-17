//! Production roots: the primary root and the scope roots, how a boundary
//! finds the root a packet belongs to, and the boundary activity record that
//! tells the later production systems which roots the lifecycle system
//! settled.
//!
//! Every production root is a whole [`ProductionState`] under its own record
//! of the production runtime kind: the primary root
//! `canwu.production:runtime` and one root per scope,
//! `canwu.production:runtime:scope:{scope}`. A root holds every site,
//! facility, work order, execution, facility project, completion lease,
//! outcome, report line, and archive chain of its scope, and nothing of any
//! other root, so the roots settle independently: a boundary decodes,
//! validates, re-encodes, and journals only the roots its packets name.

use crate::PLUGIN_NAMESPACE;
use crate::model::{
    ProductionRuntimeRecord, ProductionScopeId, ProductionState, invalid,
    production_root_reference, production_root_scope,
};
use canwu_api::{
    BoundaryContext, BoundaryId, CanwuError, DomainRecord, DomainRecordKind, DomainRecordRef,
    DomainRecordType, DomainValueKindClass, IngressId, SimulationView, TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The production root a packet or a host call names: `None` for the primary
/// root, `Some(scope)` for a scope root.
pub(crate) type RootKey = Option<ProductionScopeId>;

/// Record id of the production boundary activity record.
pub const PRODUCTION_BOUNDARY_ACTIVITY_ID: &str = "canwu.production:boundary-activity";

/// What the production lifecycle system settled in one boundary, for the
/// systems of the same boundary's later phases. It is rewritten by every
/// boundary in which the lifecycle system writes a root, and a later system
/// reads it only when `boundary` is its own boundary, so it never carries a
/// decision from one boundary into another.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionBoundaryActivityV1 {
    pub boundary: BoundaryId,
    /// Every root the lifecycle system wrote, in scope order.
    pub roots: Vec<ProductionRootActivityV1>,
}

/// One root the lifecycle system wrote in a boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionRootActivityV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ProductionScopeId>,
    /// The first packet of the boundary that changed this root's production
    /// lifecycle: an applied operation or batch item, a settled output
    /// acknowledgement, or an accepted completion step. Incident evaluation
    /// runs for a root only when it is set and cites it as its draw evidence;
    /// output dispatch and report publication run when it is set or the
    /// root's wake was admitted. A root changed only by recorded rejections,
    /// continuation witnesses, or archive maintenance has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<IngressId>,
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProductionBoundaryActivityRecord;

impl DomainRecordType for ProductionBoundaryActivityRecord {
    type Payload = ProductionBoundaryActivityV1;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
    const NAME: &'static str = "boundary-activity";
}

#[must_use]
pub fn production_boundary_activity_reference()
-> TypedDomainRecordRef<ProductionBoundaryActivityRecord> {
    TypedDomainRecordRef::new(PRODUCTION_BOUNDARY_ACTIVITY_ID)
}

/// The kind of every production root record.
pub(crate) fn production_root_kind() -> DomainRecordKind {
    DomainRecordKind::for_type::<ProductionRuntimeRecord>()
}

/// Decodes and validates the root `scope` names, as the view sees it. A
/// record whose payload names another scope than its record is refused.
pub(crate) fn load_root(
    view: &SimulationView<'_>,
    scope: Option<&ProductionScopeId>,
) -> Result<Option<(DomainRecord, ProductionState)>, CanwuError> {
    let Some(record) = view.typed_domain_record(&production_root_reference(scope))? else {
        return Ok(None);
    };
    let state = record.decode_payload::<ProductionRuntimeRecord>()?;
    if state.scope.as_ref() != scope {
        return Err(invalid(
            "production root names another scope than its record",
        ));
    }
    state.validate()?;
    Ok(Some((record.clone(), state)))
}

/// The root a production record reference names, if it names one.
pub(crate) fn root_of_record(reference: &DomainRecordRef) -> Option<RootKey> {
    production_root_scope(reference)
}

/// Reads the optional `scope` member of a packet payload. A member that is
/// present but is not a scope identifier is an error, which the lifecycle
/// system records as a refusal.
pub(crate) fn payload_scope(value: Option<&Value>) -> Result<RootKey, CanwuError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value::<ProductionScopeId>(value.clone())
            .map(Some)
            .map_err(|error| invalid(format!("production packet scope is invalid: {error}"))),
    }
}

/// The payload without its `scope` member, for packet types whose payload is
/// another type with the scope beside it.
pub(crate) fn without_scope(payload: &Value) -> Value {
    let mut payload = payload.clone();
    if let Some(object) = payload.as_object_mut() {
        object.remove("scope");
    }
    payload
}

/// The payload with `scope` beside its members when the root is a scope
/// root.
pub(crate) fn with_scope(mut payload: Value, scope: Option<&ProductionScopeId>) -> Value {
    if let (Some(scope), Some(object)) = (scope, payload.as_object_mut()) {
        object.insert("scope".to_owned(), Value::String(scope.to_string()));
    }
    payload
}

/// The activity record of `context`'s own boundary, if the lifecycle system
/// wrote any root in it.
pub(crate) fn boundary_activity(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<Option<ProductionBoundaryActivityV1>, CanwuError> {
    let reference = production_boundary_activity_reference();
    let Some(record) = view.typed_domain_record(&reference)? else {
        return Ok(None);
    };
    // After the domain commit, only a version this boundary wrote is
    // trusted: a record installed with a scenario or a snapshot, whatever
    // boundary it names, never is. Before it (invariant validation), a
    // planted record could only make the system validate a root again.
    if context_is_committed(context) {
        let written_now = view
            .current_domain_record_version(reference.as_untyped())?
            .is_some_and(|version| {
                matches!(
                    version.established_by,
                    canwu_api::DomainRecordVersionSource::BoundaryChange { boundary, .. }
                        if boundary == context.boundary_id
                )
            });
        if !written_now {
            return Ok(None);
        }
    }
    let activity = record.decode_payload::<ProductionBoundaryActivityRecord>()?;
    Ok((activity.boundary == context.boundary_id).then_some(activity))
}

/// Whether the boundary's domain commit has run before the system `context`
/// belongs to, so the records of the boundary have their provenance.
fn context_is_committed(context: &BoundaryContext) -> bool {
    context.phase > canwu_api::BoundaryPhase::AtomicDomainCommit
}

/// The roots the lifecycle system wrote in this boundary, with the trigger
/// of each.
pub(crate) fn active_roots(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BTreeMap<RootKey, Option<IngressId>>, CanwuError> {
    Ok(boundary_activity(view, context)?
        .map(|activity| {
            activity
                .roots
                .into_iter()
                .map(|root| (root.scope, root.trigger))
                .collect()
        })
        .unwrap_or_default())
}
