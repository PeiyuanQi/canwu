use crate::model::{
    AdoptionRecord, ApplicationSpec, AssetBinding, AttemptObservation, CapabilityQualification,
    ClaimAssessment, ExperimentAttempt, ImplementationRecord, MetricSchema, ProductionRun,
    REFERENCE_EVALUATOR_V1, TechnicalClaim, TechnicalProgram, TechniqueRevision, TechniqueSpec,
    TechnologyExecutionIntent, TechnologyIntentRequest, TechnologyIntentState, TechnologyLimitsV1,
    TechnologyOperation, TechnologyOperationStatus, TransmissionOpportunity,
    attach_payload_continuation,
};
use crate::plugin::{
    APPLY_SYSTEM, AdmittedOperation, CAPACITY_REJECTION_EVENT, FINALIZE_SYSTEM, INPUT_HASH_DOMAIN,
    OperationOrigin, TECHNOLOGY_COMMAND_INGRESS, operation_draft, reduce_new_operations,
};
use crate::schema::technology_record_kinds;
use canwu_api::{
    BoundaryId, BoundaryPhase, BoundaryRecord, Canwu, CanwuError, CauseRef, Command,
    CommandAttemptId, CommandId, DomainRecord, DomainRecordClass, DomainRecordKind,
    DomainRecordLifecycle, DomainRecordOperation, DomainRecordRef, DomainRecordType,
    DomainRecordVersionRef, DomainRecordVersionSource, EntityRef, ErrorCode, EventId, EvidenceRef,
    IngressId, IngressPayload, KnowledgeHolderRef, KnowledgeSubjectTarget, RandomDrawId,
    SimulationView, StateVisibility, canonical_hash,
};
use std::collections::{BTreeMap, BTreeSet};

const QUERY_PAGE: usize = 512;

pub(crate) trait TechnologyEvidenceAccess {
    fn technology_domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<Option<DomainRecord>, CanwuError>;

    fn technology_domain_record_version_exists(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<bool, CanwuError>;

    fn technology_evidence_exists(&self, reference: &EvidenceRef) -> Result<bool, CanwuError>;

    fn technology_evidence_time(
        &self,
        reference: &EvidenceRef,
    ) -> Result<Option<canwu_api::SimTime>, CanwuError>;
}

impl TechnologyEvidenceAccess for SimulationView<'_> {
    fn technology_domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<Option<DomainRecord>, CanwuError> {
        self.domain_record_version(reference)
    }

    fn technology_domain_record_version_exists(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<bool, CanwuError> {
        self.domain_record_version_evidence_exists(reference)
    }

    fn technology_evidence_exists(&self, reference: &EvidenceRef) -> Result<bool, CanwuError> {
        self.evidence_exists(reference)
    }

    fn technology_evidence_time(
        &self,
        reference: &EvidenceRef,
    ) -> Result<Option<canwu_api::SimTime>, CanwuError> {
        self.evidence_time(reference)
    }
}

impl TechnologyEvidenceAccess for Canwu {
    fn technology_domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<Option<DomainRecord>, CanwuError> {
        Ok(self.domain_record_version(reference))
    }

    fn technology_domain_record_version_exists(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<bool, CanwuError> {
        Ok(self.domain_record_version_evidence_exists(reference))
    }

    fn technology_evidence_exists(&self, reference: &EvidenceRef) -> Result<bool, CanwuError> {
        Ok(self.evidence_exists(reference))
    }

    fn technology_evidence_time(
        &self,
        reference: &EvidenceRef,
    ) -> Result<Option<canwu_api::SimTime>, CanwuError> {
        Ok(self.evidence_time(reference))
    }
}

#[derive(Clone, Debug, Default)]
pub struct TechnologyRecordSet {
    pub records: BTreeMap<DomainRecordRef, DomainRecord>,
    exact_records: BTreeMap<DomainRecordVersionRef, DomainRecord>,
}

impl TechnologyRecordSet {
    pub fn load(view: &SimulationView<'_>) -> Result<Self, CanwuError> {
        let mut records = BTreeMap::new();
        for kind in technology_record_kinds() {
            let mut after = None;
            loop {
                let page = view.domain_records_of_kind_after(&kind, after.as_ref(), QUERY_PAGE)?;
                if page.is_empty() {
                    break;
                }
                after = page.last().map(|record| record.reference.clone());
                let full = page.len() == QUERY_PAGE;
                records.extend(
                    page.into_iter()
                        .map(|record| (record.reference.clone(), record)),
                );
                if !full {
                    break;
                }
            }
        }
        let mut set = Self {
            records,
            exact_records: BTreeMap::new(),
        };
        let references = exact_versions(&set)?;
        set.hydrate_view(view, references)?;
        Ok(set)
    }

    pub fn load_host(canwu: &Canwu) -> Result<Self, CanwuError> {
        let mut records = BTreeMap::new();
        let revision = canwu.revision();
        for kind in technology_record_kinds() {
            let mut after = None;
            loop {
                let page =
                    canwu.domain_record_page(&kind, after.as_ref(), QUERY_PAGE, Some(revision))?;
                let has_more = page.next.is_some();
                after = page.next;
                records.extend(
                    page.records
                        .into_iter()
                        .map(|record| (record.reference.clone(), record)),
                );
                if !has_more {
                    break;
                }
            }
        }
        let mut set = Self {
            records,
            exact_records: BTreeMap::new(),
        };
        let mut pending = exact_versions(&set)?;
        while let Some(reference) = pending.pop_first() {
            if set.exact_records.contains_key(&reference) {
                continue;
            }
            let record = canwu.domain_record_version(&reference).ok_or_else(|| {
                invalid(format!(
                    "exact technology record body {reference:?} is unavailable"
                ))
            })?;
            pending.extend(exact_versions_in_record(&record)?);
            set.exact_records.insert(reference, record);
            enforce_exact_record_bound(&set)?;
        }
        Ok(set)
    }

    pub fn insert(&mut self, record: DomainRecord) {
        self.records.insert(record.reference.clone(), record);
    }

    pub(crate) fn hydrate_view(
        &mut self,
        view: &SimulationView<'_>,
        references: impl IntoIterator<Item = DomainRecordVersionRef>,
    ) -> Result<(), CanwuError> {
        self.hydrate(view, references)
    }

    pub(crate) fn hydrate(
        &mut self,
        access: &(impl TechnologyEvidenceAccess + ?Sized),
        references: impl IntoIterator<Item = DomainRecordVersionRef>,
    ) -> Result<(), CanwuError> {
        let mut pending = references.into_iter().collect::<BTreeSet<_>>();
        while let Some(reference) = pending.pop_first() {
            if self.exact_records.contains_key(&reference) {
                continue;
            }
            let record = access
                .technology_domain_record_version(&reference)?
                .ok_or_else(|| {
                    invalid(format!(
                        "exact technology record body {reference:?} is unavailable"
                    ))
                })?;
            pending.extend(exact_versions_in_record(&record)?);
            self.exact_records.insert(reference, record);
            enforce_exact_record_bound(self)?;
        }
        Ok(())
    }

    pub fn validate(&self, now: canwu_api::SimTime) -> Result<(), CanwuError> {
        let limits = TechnologyLimitsV1::canonical();
        if self.records.len() > limits.max_total_records {
            return Err(invalid("technology records exceed the shared total cap"));
        }
        let mut counts = BTreeMap::<DomainRecordKind, usize>::new();
        for record in self.records.values() {
            let encoded =
                serde_json::to_vec(&record.payload).map_err(|error| invalid_encoding(&error))?;
            if encoded.len() > limits.max_payload_bytes
                || record.references.len() > limits.max_references
            {
                return Err(invalid(format!(
                    "technology record {} exceeds canonical size limits",
                    record.reference
                )));
            }
            let count = counts.entry(record.reference.kind.clone()).or_default();
            *count += 1;
            if *count > limits.max_records_per_kind {
                return Err(invalid(format!(
                    "technology record kind {} exceeds its lifetime cap",
                    record.reference.kind
                )));
            }
            validate_payload_continuation(record)?;
        }
        for record in self.exact_records.values() {
            validate_payload_continuation(record)?;
        }
        self.validate_metrics()?;
        self.validate_revisions()?;
        self.validate_programs(now)?;
        self.validate_intents(now)?;
        self.validate_attempts(now)?;
        self.validate_observations_and_claims(now)?;
        self.validate_capabilities(now)?;
        self.validate_runtime_records(now)?;
        Ok(())
    }

    fn validate_metrics(&self) -> Result<(), CanwuError> {
        for (_, value) in self.decoded::<MetricSchema>()? {
            canonical_text(&value.label, "metric label")?;
            canonical_text(&value.unit, "metric unit")?;
            if value.scale == 0 || value.minimum > value.maximum {
                return Err(invalid("metric schema has an invalid scale or range"));
            }
        }
        for (_, value) in self.decoded::<TechniqueSpec>()? {
            canonical_text(&value.label, "technique label")?;
            canonical_text(&value.function, "technique function")?;
            bounded(&value.requirements, "technique requirements")?;
            bounded(&value.qualification_rules, "qualification rules")?;
            for group in &value.requirements {
                validate_group(self, group)?;
            }
            for rule in &value.qualification_rules {
                canonical_text(&rule.operation, "qualification operation")?;
                if rule.minimum_successful_attempts == 0
                    || rule.minimum_reliability_per_mille > 1_000
                {
                    return Err(invalid("qualification rule is out of range"));
                }
            }
        }
        for (_, value) in self.decoded::<ApplicationSpec>()? {
            self.require_version(&value.technique)?;
            bounded(&value.viability, "application viability")?;
            for group in &value.viability {
                validate_group(self, group)?;
            }
        }
        Ok(())
    }

    fn validate_revisions(&self) -> Result<(), CanwuError> {
        let revisions = self.decoded::<TechniqueRevision>()?;
        for (reference, value) in &revisions {
            self.require_version(&value.spec)?;
            if value.evaluator != REFERENCE_EVALUATOR_V1 {
                return Err(invalid("technique revision uses an unknown evaluator"));
            }
            bounded(&value.parents, "revision parents")?;
            bounded(&value.parameters, "revision parameters")?;
            bounded(&value.discovery_evidence, "revision discovery evidence")?;
            for parent in &value.parents {
                self.require_version(&parent.parent)?;
                if parent.parent.record == *reference {
                    return Err(invalid("technique revision cannot parent itself"));
                }
            }
            validate_metric_values(self, &value.parameters)?;
            match (&value.produced_by, &value.execution_intent) {
                (Some(program_ref), Some(_)) => {
                    if value.discovery_evidence.is_empty() {
                        return Err(invalid(
                            "runtime technique revision requires discovery evidence",
                        ));
                    }
                    let program = self.decode_version::<TechnicalProgram>(program_ref)?;
                    if program.status != crate::model::ProgramStatus::Active
                        || !matches!(
                            program.mode,
                            crate::model::ProgramMode::Investigation
                                | crate::model::ProgramMode::Adaptation
                                | crate::model::ProgramMode::ReverseEngineering
                        )
                    {
                        return Err(invalid(
                            "runtime technique revision requires an active invention program",
                        ));
                    }
                    if let Some(prior) = &program.revision
                        && !value.parents.iter().any(|parent| parent.parent == *prior)
                    {
                        return Err(invalid(
                            "adapted technique revision must name its program revision as a parent",
                        ));
                    }
                }
                (Some(_), None) => {
                    return Err(invalid(
                        "runtime technique revision requires an exact execution intent",
                    ));
                }
                (None, Some(_)) => {
                    return Err(invalid(
                        "catalog technique revision cannot carry an execution intent",
                    ));
                }
                (None, None) if !value.discovery_evidence.is_empty() => {
                    return Err(invalid(
                        "catalog technique revision cannot carry runtime discovery evidence",
                    ));
                }
                (None, None) => {}
            }
        }
        for reference in revisions.keys() {
            let mut frontier = vec![(reference.clone(), 0usize, BTreeSet::new())];
            while let Some((current, depth, mut path)) = frontier.pop() {
                if depth > TechnologyLimitsV1::canonical().max_ancestry_depth {
                    return Err(invalid("technique revision ancestry exceeds its bound"));
                }
                if !path.insert(current.clone()) {
                    return Err(invalid("technique revision ancestry contains a cycle"));
                }
                if let Some(payload) = revisions.get(&current) {
                    frontier.extend(
                        payload
                            .parents
                            .iter()
                            .map(|parent| (parent.parent.record.clone(), depth + 1, path.clone())),
                    );
                }
            }
        }
        Ok(())
    }

    fn validate_programs(&self, now: canwu_api::SimTime) -> Result<(), CanwuError> {
        for (_, value) in self.decoded::<TechnicalProgram>()? {
            if value.started_at > now || value.due_at.is_some_and(|due| due < value.started_at) {
                return Err(invalid("technical program has an invalid time window"));
            }
            if let Some(revision) = &value.revision {
                self.require_version(revision)?;
            }
            bounded(&value.requirements, "provider requirements")?;
            for requirement in &value.requirements {
                canonical_text(&requirement.provider, "provider")?;
                canonical_text(&requirement.capability, "provider capability")?;
                canonical_text(&requirement.unit, "provider unit")?;
                if requirement.quantity == 0 {
                    return Err(invalid("provider quantity must be nonzero"));
                }
            }
        }
        Ok(())
    }

    fn validate_intents(&self, _now: canwu_api::SimTime) -> Result<(), CanwuError> {
        for (_, value) in self.decoded::<TechnologyExecutionIntent>()? {
            canonical_text(&value.provider, "technology provider")?;
            if value
                .expires_at
                .is_some_and(|until| until < value.not_before)
            {
                return Err(invalid(
                    "technology execution intent has an invalid time window",
                ));
            }
            let program = self.decode_version::<TechnicalProgram>(&value.program)?;
            if program.sponsor != value.authorized_by {
                return Err(invalid(
                    "technology execution intent is not owned by its exact program sponsor",
                ));
            }
            match &value.request {
                TechnologyIntentRequest::Experiment {
                    result_id,
                    revision,
                    operation,
                    site,
                    required_assets,
                    ..
                } => {
                    canonical_text(result_id, "technology result id")?;
                    canonical_text(operation, "technology operation")?;
                    bounded(required_assets, "experiment intent required assets")?;
                    self.require_version(revision)?;
                    if program.revision.as_ref() != Some(revision) || program.site != *site {
                        return Err(invalid(
                            "experiment intent does not match its exact program revision and site",
                        ));
                    }
                    for asset in required_assets {
                        self.require_version(asset)?;
                    }
                }
                TechnologyIntentRequest::Production {
                    result_id,
                    revision,
                    application,
                    site,
                    required_assets,
                    ..
                } => {
                    canonical_text(result_id, "technology result id")?;
                    bounded(required_assets, "production intent required assets")?;
                    self.require_version(revision)?;
                    if let Some(application) = application {
                        self.require_version(application)?;
                    }
                    if program.revision.as_ref() != Some(revision) || program.site != *site {
                        return Err(invalid(
                            "production intent does not match its exact program revision and site",
                        ));
                    }
                    for asset in required_assets {
                        self.require_version(asset)?;
                    }
                }
                TechnologyIntentRequest::Invention {
                    result_id,
                    spec,
                    site,
                    ..
                } => {
                    canonical_text(result_id, "technology result id")?;
                    self.require_version(spec)?;
                    if program.site != *site
                        || !matches!(
                            program.mode,
                            crate::model::ProgramMode::Investigation
                                | crate::model::ProgramMode::Adaptation
                                | crate::model::ProgramMode::ReverseEngineering
                        )
                    {
                        return Err(invalid(
                            "invention intent does not match its program site and mode",
                        ));
                    }
                }
            }
            if let TechnologyIntentState::Consumed {
                ingress,
                operation,
                result,
            } = &value.state
            {
                self.require_version(operation)?;
                self.require_version(result)?;
                if !matches!(ingress, canwu_api::EvidenceRef::Ingress(_)) {
                    return Err(invalid(
                        "consumed technology intent must cite its provider ingress",
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_attempts(&self, now: canwu_api::SimTime) -> Result<(), CanwuError> {
        let metrics = self.decoded::<MetricSchema>()?;
        for (_, value) in self.decoded::<ExperimentAttempt>()? {
            let intent =
                self.decode_version::<TechnologyExecutionIntent>(&value.execution_intent)?;
            let program = self.decode_version::<TechnicalProgram>(&value.program)?;
            let revision = self.decode_version::<TechniqueRevision>(&value.revision)?;
            if value.started_at > value.ended_at
                || value.ended_at > now
                || value.started_at < intent.not_before
                || intent
                    .expires_at
                    .is_some_and(|expires_at| value.ended_at > expires_at)
                || value.evaluation.evaluator != REFERENCE_EVALUATOR_V1
            {
                return Err(invalid("experiment attempt has invalid time or evaluator"));
            }
            canonical_text(&value.operation, "attempt operation")?;
            if program.status != crate::model::ProgramStatus::Active
                || program.revision.as_ref() != Some(&value.revision)
                || program.site != value.site
                || value.started_at < program.started_at
                || program.due_at.is_some_and(|due| value.ended_at > due)
            {
                return Err(invalid(
                    "experiment attempt is not bound to its active program, revision, site, and time window",
                ));
            }
            validate_metric_values(self, &value.inputs)?;
            validate_metric_values(self, &value.environment)?;
            validate_metric_values(self, &value.outputs)?;
            bounded(&value.assets, "attempt assets")?;
            validate_evaluation(&value.evaluation)?;
            for asset in &value.assets {
                self.require_version(asset)?;
            }
            let spec = self.decode_version::<TechniqueSpec>(&revision.spec)?;
            let mut context = crate::model::MetricContext::default();
            context.values.extend(
                value
                    .inputs
                    .iter()
                    .chain(&value.environment)
                    .chain(&value.outputs)
                    .map(|metric| (metric.metric.record.clone(), metric.value)),
            );
            let expected = crate::evaluate_attempt(&revision, &spec, &metrics, &context)?;
            if expected != value.evaluation {
                return Err(invalid(
                    "experiment attempt evaluation does not match its evidence",
                ));
            }
        }
        Ok(())
    }

    fn validate_observations_and_claims(&self, now: canwu_api::SimTime) -> Result<(), CanwuError> {
        for (_, value) in self.decoded::<AttemptObservation>()? {
            let attempt = self.decode_version::<ExperimentAttempt>(&value.attempt)?;
            if value.uncertainty_per_mille > 1_000
                || value.observed_at < attempt.ended_at
                || value.observed_at > now
            {
                return Err(invalid("attempt observation is out of range"));
            }
            canonical_text(&value.method, "observation method")?;
            validate_metric_values(self, &value.values)?;
        }
        for (_, value) in self.decoded::<TechnicalClaim>()? {
            canonical_text(&value.proposition, "claim proposition")?;
            if value.asserted_at > now {
                return Err(invalid("technical claim is asserted in the future"));
            }
            bounded(&value.scope, "claim scope")?;
            bounded(&value.source_evidence, "claim source evidence")?;
            bounded(&value.relations, "claim relations")?;
            for relation in &value.relations {
                canonical_text(&relation.relation, "claim relation")?;
                self.require_version(&relation.claim)?;
            }
        }
        for (_, value) in self.decoded::<ClaimAssessment>()? {
            let claim = self.decode_version::<TechnicalClaim>(&value.claim)?;
            if value.confidence_per_mille > 1_000
                || value.as_of < claim.asserted_at
                || value.as_of > now
            {
                return Err(invalid("claim assessment is out of range"));
            }
            canonical_text(&value.method, "claim assessment method")?;
            bounded(
                &value.supporting_evidence,
                "claim assessment supporting evidence",
            )?;
            bounded(
                &value.contradicting_evidence,
                "claim assessment contradicting evidence",
            )?;
        }
        Ok(())
    }

    fn validate_capabilities(&self, now: canwu_api::SimTime) -> Result<(), CanwuError> {
        for (_, value) in self.decoded::<CapabilityQualification>()? {
            self.require_version(&value.revision)?;
            bounded(&value.attempts, "capability attempts")?;
            if value.reliability_per_mille > 1_000
                || value.last_practiced_at > now
                || value.valid_from < value.last_practiced_at
                || value.valid_from > now
                || value
                    .valid_until
                    .is_some_and(|until| until < value.valid_from || (value.active && until < now))
            {
                return Err(invalid("capability qualification is out of range"));
            }
            if value.attempts.is_empty()
                || value.attempts.iter().collect::<BTreeSet<_>>().len() != value.attempts.len()
            {
                return Err(invalid(
                    "capability requires unique exact experiment attempts",
                ));
            }
            let mut successful = 0usize;
            let mut operators = BTreeSet::new();
            let mut subject_participated = false;
            let mut last_practiced = None;
            let claimed_operator = value
                .operator
                .clone()
                .unwrap_or_else(|| holder_entity(&value.holder));
            for attempt_ref in &value.attempts {
                let attempt = self.decode_version::<ExperimentAttempt>(attempt_ref)?;
                if attempt.revision != value.revision
                    || attempt.operation != value.operation
                    || attempt.site != value.site
                    || attempt.ended_at > now
                {
                    return Err(invalid(
                        "capability attempt does not match its revision, operation, site, or time",
                    ));
                }
                let operator = holder_entity(&attempt.operator);
                subject_participated |= operator == claimed_operator;
                operators.insert(operator);
                successful += usize::from(attempt.evaluation.passed);
                last_practiced = Some(last_practiced.map_or(attempt.ended_at, |current| {
                    std::cmp::max(current, attempt.ended_at)
                }));
            }
            if !subject_participated || last_practiced != Some(value.last_practiced_at) {
                return Err(invalid(
                    "capability holder/operator and last-practiced time do not match its attempts",
                ));
            }
            let revision = self.decode_version::<TechniqueRevision>(&value.revision)?;
            let spec = self.decode_version::<TechniqueSpec>(&revision.spec)?;
            let rule = spec
                .qualification_rules
                .iter()
                .find(|rule| rule.operation == value.operation)
                .ok_or_else(|| invalid("capability operation has no qualification rule"))?;
            let reliability = u16::try_from(successful * 1_000 / value.attempts.len())
                .map_err(|_| invalid("capability reliability exceeds its integer range"))?;
            if value.reliability_per_mille != reliability
                || (value.active
                    && (successful < usize::from(rule.minimum_successful_attempts)
                        || reliability < rule.minimum_reliability_per_mille
                        || (rule.independent_reproduction_required && operators.len() < 2)))
            {
                return Err(invalid("capability evidence does not satisfy its rule"));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn validate_runtime_records(&self, now: canwu_api::SimTime) -> Result<(), CanwuError> {
        for (_, value) in self.decoded::<AssetBinding>()? {
            if value.condition_per_mille > 1_000 {
                return Err(invalid("asset condition exceeds 1000 permille"));
            }
            bounded(&value.capabilities, "asset capabilities")?;
            for capability in &value.capabilities {
                canonical_text(capability, "asset capability")?;
            }
        }
        for (_, value) in self.decoded::<ProductionRun>()? {
            let intent =
                self.decode_version::<TechnologyExecutionIntent>(&value.execution_intent)?;
            let revision = self.decode_version::<TechniqueRevision>(&value.revision)?;
            if let Some(application) = &value.application {
                let application = self.decode_version::<ApplicationSpec>(application)?;
                if application.technique != revision.spec {
                    return Err(invalid(
                        "production application does not match its technique revision",
                    ));
                }
            }
            bounded(&value.assets, "production assets")?;
            for asset in &value.assets {
                self.require_version(asset)?;
            }
            if value.started_at > value.ended_at
                || value.started_at < intent.not_before
                || intent
                    .expires_at
                    .is_some_and(|expires_at| value.ended_at > expires_at)
                || value.ended_at > now
            {
                return Err(invalid(
                    "production run has an invalid or future time window",
                ));
            }
            validate_metric_values(self, &value.inputs)?;
            validate_metric_values(self, &value.outputs)?;
            validate_evaluation(&value.evaluation)?;
            let spec = self.decode_version::<TechniqueSpec>(&revision.spec)?;
            let metrics = self.decoded::<MetricSchema>()?;
            let context = crate::model::MetricContext {
                values: value
                    .inputs
                    .iter()
                    .chain(&value.outputs)
                    .map(|metric| (metric.metric.record.clone(), metric.value))
                    .collect(),
            };
            let expected = crate::evaluate_attempt(&revision, &spec, &metrics, &context)?;
            if value.evaluation != expected || value.successful != expected.passed {
                return Err(invalid(
                    "production result does not match its exact revision evidence",
                ));
            }
        }
        for (_, value) in self.decoded::<ImplementationRecord>()? {
            self.require_version(&value.revision)?;
            self.require_version(&value.qualification)?;
            bounded(&value.assets, "implementation assets")?;
            if value.reliability_per_mille > 1_000 || value.installed_at > now {
                return Err(invalid(
                    "implementation reliability or installation time is out of range",
                ));
            }
            let qualification =
                self.decode_version::<CapabilityQualification>(&value.qualification)?;
            if qualification.revision != value.revision
                || qualification.site != value.site
                || qualification.holder != value.owner
                || !qualification.active
                || qualification.valid_from > value.installed_at
                || qualification.valid_until.is_some_and(|until| {
                    until < value.installed_at || (value.active && until < now)
                })
            {
                return Err(invalid(
                    "implementation lacks a matching active local qualification",
                ));
            }
            for asset in &value.assets {
                let asset = self.decode_version::<AssetBinding>(asset)?;
                if !asset.active || asset.site != value.site || asset.owner != value.owner {
                    return Err(invalid(
                        "implementation asset is inactive, off-site, or owned by another holder",
                    ));
                }
            }
        }
        for (_, value) in self.decoded::<AdoptionRecord>()? {
            bounded(&value.implementations, "adoption implementations")?;
            bounded(&value.viability_evidence, "adoption viability evidence")?;
            validate_evaluation(&value.viability)?;
            let application = self.decode_version::<ApplicationSpec>(&value.application)?;
            let requires_viability = matches!(
                value.status,
                crate::model::AdoptionStatus::Trial | crate::model::AdoptionStatus::Committed
            );
            if value.implementations.is_empty()
                || value.viability_evidence.is_empty()
                || (requires_viability && !value.viability.passed)
            {
                return Err(invalid(
                    "active adoption requires a viable evidenced implementation portfolio",
                ));
            }
            let mut installed_capacity = 0u64;
            for implementation in &value.implementations {
                let installed = self.decode_version::<ImplementationRecord>(implementation)?;
                let revision = self.decode_version::<TechniqueRevision>(&installed.revision)?;
                if installed.site != value.site
                    || installed.owner != value.adopter
                    || (requires_viability && !installed.active)
                    || revision.spec != application.technique
                {
                    return Err(invalid(
                        "adoption implementation does not match its holder, site, state, or technique",
                    ));
                }
                installed_capacity = installed_capacity
                    .checked_add(installed.capacity)
                    .ok_or_else(|| invalid("adoption implementation capacity overflowed"))?;
            }
            if requires_viability && value.scale > installed_capacity {
                return Err(invalid(
                    "active adoption scale exceeds its exact implementation capacity",
                ));
            }
            validate_metric_values(self, &value.viability_metrics)?;
            let evidenced = self.evidenced_metrics(&value.viability_evidence)?;
            if value
                .viability_metrics
                .iter()
                .any(|metric| evidenced.get(&metric.metric.record).copied() != Some(metric.value))
            {
                return Err(invalid(
                    "adoption viability metric lacks matching provider evidence",
                ));
            }
            let metrics = self.decoded::<MetricSchema>()?;
            let context = crate::model::MetricContext {
                values: value
                    .viability_metrics
                    .iter()
                    .map(|metric| (metric.metric.record.clone(), metric.value))
                    .collect(),
            };
            if crate::evaluate_application(&application, &metrics, &context)? != value.viability {
                return Err(invalid("adoption viability does not match its evidence"));
            }
        }
        for (_, value) in self.decoded::<TransmissionOpportunity>()? {
            bounded(&value.evidence, "transmission evidence")?;
            if value.opened_at > now {
                return Err(invalid("transmission opportunity opens in the future"));
            }
            if let Some(revision) = &value.revision {
                self.require_version(revision)?;
            }
            if value.source.is_none()
                && !matches!(
                    value.mode,
                    crate::model::TransmissionMode::IndependentInvestigation
                )
            {
                return Err(invalid("non-independent transmission requires a source"));
            }
            let requires_practice_capability = matches!(
                value.mode,
                crate::model::TransmissionMode::Demonstration
                    | crate::model::TransmissionMode::Apprenticeship
                    | crate::model::TransmissionMode::PersonnelTransfer
            );
            if requires_practice_capability && value.source_capability.is_none() {
                return Err(invalid(
                    "practice transmission requires an exact source capability",
                ));
            }
            if let Some(source_capability) = &value.source_capability {
                self.validate_transmission_source(&value, source_capability)?;
            }
            if let Some(program_ref) = &value.resulting_program {
                let program = self.decode_version::<TechnicalProgram>(program_ref)?;
                if program.sponsor != value.destination
                    || program.site != value.destination_site
                    || program.revision != value.revision
                    || program.started_at < value.opened_at
                    || !matches!(
                        program.mode,
                        crate::model::ProgramMode::Training
                            | crate::model::ProgramMode::Investigation
                            | crate::model::ProgramMode::Adaptation
                            | crate::model::ProgramMode::ReverseEngineering
                    )
                {
                    return Err(invalid(
                        "transmission outcome is not a matching destination program",
                    ));
                }
            }
        }
        for (reference, value) in self.decoded::<TechnologyOperation>()? {
            bounded(
                &value.canonical_input_hashes,
                "operation canonical input hashes",
            )?;
            bounded(&value.causes, "operation causes")?;
            if value.id != reference.id
                || value.canonical_input_hashes.is_empty()
                || !value
                    .canonical_input_hashes
                    .windows(2)
                    .all(|pair| pair[0] < pair[1])
                || value.causes.is_empty()
                || !value.causes.windows(2).all(|pair| pair[0] < pair[1])
                || value.causes.len() != value.canonical_input_hashes.len()
            {
                return Err(invalid(
                    "technology operation identity or input evidence is invalid",
                ));
            }
            let expected_hash = if value.canonical_input_hashes.len() == 1 {
                value.canonical_input_hashes[0].clone()
            } else {
                canwu_api::canonical_hash(
                    "canwu.technology.operation-conflict.v1",
                    &value.canonical_input_hashes,
                )?
            };
            if value.canonical_input_hash != expected_hash {
                return Err(invalid("technology operation input summary is invalid"));
            }
            match value.status {
                TechnologyOperationStatus::Applied
                    if value.result.is_some() && value.rejection_code.is_none() => {}
                TechnologyOperationStatus::Rejected
                    if value.result.is_none()
                        && value
                            .rejection_code
                            .as_deref()
                            .is_some_and(valid_rejection_code) => {}
                _ => {
                    return Err(invalid(
                        "technology operation terminal state is inconsistent",
                    ));
                }
            }
            if value.canonical_input_hashes.len() > 1
                && (value.status != TechnologyOperationStatus::Rejected
                    || value.rejection_code.as_deref() != Some("idempotency_conflict"))
            {
                return Err(invalid("technology operation conflict is not terminal"));
            }
            if value
                .result
                .as_ref()
                .is_some_and(|result| !self.records.contains_key(result))
            {
                return Err(invalid("technology operation result is unavailable"));
            }
        }
        Ok(())
    }

    fn validate_transmission_source(
        &self,
        transmission: &crate::model::TransmissionOpportunityPayload,
        reference: &DomainRecordVersionRef,
    ) -> Result<(), CanwuError> {
        let source = transmission
            .source
            .as_ref()
            .ok_or_else(|| invalid("source capability requires a transmission source"))?;
        let source_site = transmission
            .source_site
            .as_ref()
            .ok_or_else(|| invalid("source capability requires an exact source site"))?;
        let revision = transmission
            .revision
            .as_ref()
            .ok_or_else(|| invalid("source capability requires an exact technique revision"))?;
        let record = self.require_version(reference)?;
        if record
            .reference
            .kind
            .matches_type::<CapabilityQualification>()
        {
            let qualification = record.decode_payload::<CapabilityQualification>()?;
            return validate_source_qualification(
                &qualification,
                source,
                source_site,
                revision,
                transmission.opened_at,
            );
        }
        if record.reference.kind.matches_type::<ImplementationRecord>() {
            let implementation = record.decode_payload::<ImplementationRecord>()?;
            if implementation.owner != *source
                || implementation.site != *source_site
                || implementation.revision != *revision
                || implementation.installed_at > transmission.opened_at
                || !implementation.active
            {
                return Err(invalid(
                    "transmission source implementation did not provide the exact active practice capability",
                ));
            }
            let qualification =
                self.decode_version::<CapabilityQualification>(&implementation.qualification)?;
            return validate_source_qualification(
                &qualification,
                source,
                source_site,
                revision,
                transmission.opened_at,
            );
        }
        Err(invalid(
            "transmission source capability is neither a qualification nor an implementation",
        ))
    }

    fn evidenced_metrics(
        &self,
        references: &[DomainRecordVersionRef],
    ) -> Result<BTreeMap<DomainRecordRef, i64>, CanwuError> {
        let mut values = BTreeMap::new();
        for reference in references {
            let record = self.require_version(reference)?;
            let metrics = if record.reference.kind.matches_type::<AttemptObservation>() {
                record.decode_payload::<AttemptObservation>()?.values
            } else if record.reference.kind.matches_type::<ProductionRun>() {
                record.decode_payload::<ProductionRun>()?.outputs
            } else {
                return Err(invalid(
                    "viability evidence must be an observation or production run",
                ));
            };
            for metric in metrics {
                if values.insert(metric.metric.record, metric.value).is_some() {
                    return Err(invalid(
                        "viability evidence repeats a metric across exact records",
                    ));
                }
            }
        }
        Ok(values)
    }

    pub fn decoded<T: DomainRecordType>(
        &self,
    ) -> Result<BTreeMap<DomainRecordRef, T::Payload>, CanwuError>
    where
        T::Payload: serde::de::DeserializeOwned,
    {
        let kind = DomainRecordKind::for_type::<T>();
        self.records
            .iter()
            .filter(|(reference, _)| reference.kind == kind)
            .map(|(reference, record)| {
                record
                    .decode_payload::<T>()
                    .map(|payload| (reference.clone(), payload))
            })
            .collect()
    }

    pub(crate) fn validate_current_transmission_source(
        &self,
        transmission: &crate::model::TransmissionOpportunityPayload,
    ) -> Result<(), CanwuError> {
        let Some(reference) = &transmission.source_capability else {
            return Ok(());
        };
        let current = self.records.get(&reference.record).ok_or_else(|| {
            invalid("transmission source capability has no current domain record")
        })?;
        if current.version != reference.version || !current.is_active() {
            return Err(invalid(
                "new transmission must cite the source capability version current when it opens",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_current_implementation_dependencies(
        &self,
        implementation: &crate::model::ImplementationPayload,
    ) -> Result<(), CanwuError> {
        let qualification = self
            .records
            .get(&implementation.qualification.record)
            .filter(|record| record.version == implementation.qualification.version)
            .ok_or_else(|| {
                invalid(
                    "new or reactivated implementation must cite the current qualification version",
                )
            })?
            .decode_payload::<CapabilityQualification>()?;
        if !qualification.active {
            return Err(invalid(
                "new or reactivated implementation requires a currently active qualification",
            ));
        }
        for reference in &implementation.assets {
            let asset = self
                .records
                .get(&reference.record)
                .filter(|record| record.version == reference.version)
                .ok_or_else(|| {
                    invalid("new or reactivated implementation must cite current asset versions")
                })?
                .decode_payload::<AssetBinding>()?;
            if !asset.active {
                return Err(invalid(
                    "new or reactivated implementation requires currently active assets",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn validate_current_adoption_dependencies(
        &self,
        adoption: &crate::model::AdoptionPayload,
    ) -> Result<(), CanwuError> {
        if !matches!(
            adoption.status,
            crate::model::AdoptionStatus::Trial | crate::model::AdoptionStatus::Committed
        ) {
            return Ok(());
        }
        for reference in &adoption.implementations {
            let implementation = self
                .records
                .get(&reference.record)
                .filter(|record| record.version == reference.version)
                .ok_or_else(|| {
                    invalid("new or recommitted adoption must cite current implementation versions")
                })?
                .decode_payload::<ImplementationRecord>()?;
            if !implementation.active {
                return Err(invalid(
                    "new or recommitted adoption requires currently active implementations",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn validate_temporal_evidence(
        &self,
        access: &(impl TechnologyEvidenceAccess + ?Sized),
    ) -> Result<(), CanwuError> {
        for (_, observation) in self.decoded::<AttemptObservation>()? {
            for reference in observation
                .values
                .iter()
                .map(|value| EvidenceRef::DomainRecordVersion(value.metric.clone()))
                .chain(std::iter::once(EvidenceRef::DomainRecordVersion(
                    observation.attempt,
                )))
            {
                require_evidence_at(access, &reference, observation.observed_at, "observation")?;
            }
        }
        for (_, claim) in self.decoded::<TechnicalClaim>()? {
            for reference in &claim.source_evidence {
                require_evidence_at(access, reference, claim.asserted_at, "technical claim")?;
            }
            for relation in &claim.relations {
                require_evidence_at(
                    access,
                    &EvidenceRef::DomainRecordVersion(relation.claim.clone()),
                    claim.asserted_at,
                    "technical claim relation",
                )?;
            }
        }
        for (_, assessment) in self.decoded::<ClaimAssessment>()? {
            require_evidence_at(
                access,
                &EvidenceRef::DomainRecordVersion(assessment.claim.clone()),
                assessment.as_of,
                "claim assessment",
            )?;
            for reference in assessment
                .supporting_evidence
                .iter()
                .chain(&assessment.contradicting_evidence)
            {
                require_evidence_at(access, reference, assessment.as_of, "claim assessment")?;
            }
        }
        Ok(())
    }

    fn decode_version<T: DomainRecordType>(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<T::Payload, CanwuError>
    where
        T::Payload: serde::de::DeserializeOwned,
    {
        let record = self.require_version(reference)?;
        record.decode_payload::<T>()
    }

    fn require_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<&DomainRecord, CanwuError> {
        self.exact_records
            .get(reference)
            .ok_or_else(|| invalid(format!("record version {reference:?} is unavailable")))
    }
}

fn require_evidence_at(
    access: &(impl TechnologyEvidenceAccess + ?Sized),
    reference: &EvidenceRef,
    cut: canwu_api::SimTime,
    label: &str,
) -> Result<(), CanwuError> {
    if access
        .technology_evidence_time(reference)?
        .is_none_or(|available_at| available_at > cut)
    {
        return Err(invalid(format!(
            "{label} cites evidence unavailable at its semantic time cut"
        )));
    }
    Ok(())
}

fn validate_source_qualification(
    qualification: &crate::model::CapabilityQualificationPayload,
    source: &KnowledgeHolderRef,
    source_site: &EntityRef,
    revision: &DomainRecordVersionRef,
    opened_at: canwu_api::SimTime,
) -> Result<(), CanwuError> {
    if qualification.holder != *source
        || qualification.site != *source_site
        || qualification.revision != *revision
        || !qualification.active
        || qualification.valid_from > opened_at
        || qualification.last_practiced_at > opened_at
        || qualification
            .valid_until
            .is_some_and(|until| until < opened_at)
    {
        return Err(invalid(
            "transmission source qualification was not valid when the opportunity opened",
        ));
    }
    Ok(())
}

struct ProvenanceIndexes<'a> {
    canwu: &'a Canwu,
    ingress: BTreeMap<IngressId, &'a canwu_api::IngressRecord>,
    commands: BTreeMap<CommandId, &'a canwu_api::CommandRecord>,
    events: BTreeMap<EventId, &'a canwu_api::SimEvent>,
    boundaries: BTreeMap<BoundaryId, (usize, &'a BoundaryRecord)>,
    boundary_cuts: BTreeMap<BoundaryId, usize>,
    command_cuts: BTreeMap<CommandId, usize>,
    command_attempt_cuts: BTreeMap<CommandAttemptId, usize>,
    ingress_cuts: BTreeMap<IngressId, usize>,
    event_cuts: BTreeMap<EventId, usize>,
    random_draw_cuts: BTreeMap<RandomDrawId, usize>,
    record_versions: BTreeMap<(DomainRecordRef, u64), DomainRecordVersionRef>,
}

impl<'a> ProvenanceIndexes<'a> {
    fn new(canwu: &'a Canwu) -> Result<Self, CanwuError> {
        let mut indexes = Self {
            canwu,
            ingress: canwu
                .ingress_log()
                .iter()
                .map(|record| (record.id, record))
                .collect(),
            commands: canwu
                .commands()
                .iter()
                .map(|record| (record.id, record))
                .collect(),
            events: canwu
                .events()
                .iter()
                .map(|record| (record.id, record))
                .collect(),
            boundaries: BTreeMap::new(),
            boundary_cuts: BTreeMap::new(),
            command_cuts: BTreeMap::new(),
            command_attempt_cuts: BTreeMap::new(),
            ingress_cuts: BTreeMap::new(),
            event_cuts: BTreeMap::new(),
            random_draw_cuts: BTreeMap::new(),
            record_versions: BTreeMap::new(),
        };
        for (cut, boundary) in canwu.boundaries().iter().enumerate() {
            indexes.boundaries.insert(boundary.id, (cut, boundary));
            indexes.boundary_cuts.insert(boundary.id, cut);
            for id in &boundary.admitted_commands {
                indexes.command_cuts.entry(*id).or_insert(cut);
            }
            for id in &boundary.admitted_attempts {
                indexes.command_attempt_cuts.entry(*id).or_insert(cut);
            }
            for id in &boundary.admitted_ingress {
                indexes.ingress_cuts.entry(*id).or_insert(cut);
            }
            for generation in &boundary.generated_ingress {
                let visible_cut = if generation.phase < BoundaryPhase::DomainDeltaProposal {
                    cut
                } else {
                    cut.saturating_add(1)
                };
                indexes
                    .ingress_cuts
                    .entry(generation.ingress)
                    .or_insert(visible_cut);
            }
            for id in &boundary.admitted_events {
                indexes.event_cuts.entry(*id).or_insert(cut);
            }
            for emission in &boundary.emissions {
                indexes
                    .event_cuts
                    .entry(emission.event)
                    .or_insert_with(|| cut.saturating_add(1));
            }
            for id in &boundary.random_draws {
                indexes
                    .random_draw_cuts
                    .entry(*id)
                    .or_insert_with(|| cut.saturating_add(1));
            }
            for (change_index, change) in boundary.record_changes.iter().enumerate() {
                let version = DomainRecordVersionRef {
                    record: change.current.reference.clone(),
                    version: change.current.version,
                    established_by: DomainRecordVersionSource::BoundaryChange {
                        boundary: boundary.id,
                        change_index: u64::try_from(change_index)
                            .map_err(|_| invalid("technology change index overflowed"))?,
                    },
                };
                indexes.record_versions.insert(
                    (change.current.reference.clone(), change.current.version),
                    version,
                );
            }
        }
        Ok(indexes)
    }
}

struct RestoreEvidenceAccess<'a> {
    indexes: &'a ProvenanceIndexes<'a>,
    cut: usize,
}

impl RestoreEvidenceAccess<'_> {
    fn exact_source_visible(&self, reference: &DomainRecordVersionRef) -> bool {
        exact_source_visible_at(reference, &self.indexes.boundary_cuts, self.cut)
    }

    fn evidence_cut_visible<T: Ord>(&self, cuts: &BTreeMap<T, usize>, id: &T) -> bool {
        evidence_id_visible_at(cuts.get(id).copied(), self.cut)
    }
}

impl TechnologyEvidenceAccess for RestoreEvidenceAccess<'_> {
    fn technology_domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<Option<DomainRecord>, CanwuError> {
        if !self.exact_source_visible(reference) {
            return Ok(None);
        }
        Ok(self.indexes.canwu.domain_record_version(reference))
    }

    fn technology_domain_record_version_exists(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Result<bool, CanwuError> {
        Ok(self.exact_source_visible(reference)
            && self
                .indexes
                .canwu
                .domain_record_version_evidence_exists(reference))
    }

    fn technology_evidence_exists(&self, reference: &EvidenceRef) -> Result<bool, CanwuError> {
        let visible = match reference {
            EvidenceRef::Command(id) => self.evidence_cut_visible(&self.indexes.command_cuts, id),
            EvidenceRef::CommandAttempt(id) => {
                self.evidence_cut_visible(&self.indexes.command_attempt_cuts, id)
            }
            EvidenceRef::Event(id) => self.evidence_cut_visible(&self.indexes.event_cuts, id),
            EvidenceRef::Ingress(id) => self.evidence_cut_visible(&self.indexes.ingress_cuts, id),
            EvidenceRef::Boundary(id) => self
                .indexes
                .boundary_cuts
                .get(id)
                .is_some_and(|established| *established < self.cut),
            EvidenceRef::RandomDraw(id) => {
                self.evidence_cut_visible(&self.indexes.random_draw_cuts, id)
            }
            EvidenceRef::DomainRecordVersion(reference) => {
                return self.technology_domain_record_version_exists(reference);
            }
        };
        Ok(visible && self.indexes.canwu.evidence_exists(reference))
    }

    fn technology_evidence_time(
        &self,
        reference: &EvidenceRef,
    ) -> Result<Option<canwu_api::SimTime>, CanwuError> {
        if !self.technology_evidence_exists(reference)? {
            return Ok(None);
        }
        Ok(self.indexes.canwu.evidence_time(reference))
    }
}

fn exact_source_visible_at(
    reference: &DomainRecordVersionRef,
    boundary_cuts: &BTreeMap<BoundaryId, usize>,
    cut: usize,
) -> bool {
    match &reference.established_by {
        DomainRecordVersionSource::InitialScenario => true,
        DomainRecordVersionSource::BoundaryChange { boundary, .. } => boundary_cuts
            .get(boundary)
            .is_some_and(|established| *established < cut),
    }
}

const fn evidence_id_visible_at(visible_cut: Option<usize>, cut: usize) -> bool {
    matches!(visible_cut, Some(visible) if visible <= cut)
}

fn validate_operation_provenance(
    indexes: &ProvenanceIndexes<'_>,
    records: &TechnologyRecordSet,
) -> Result<(), CanwuError> {
    let mut state = initial_technology_state(records, indexes.canwu.boundaries());
    for (cut, boundary) in indexes.canwu.boundaries().iter().enumerate() {
        let access = RestoreEvidenceAccess { indexes, cut };
        let references = exact_versions(&state)?;
        state.hydrate(&access, references)?;
        for reference in evidence_refs(&state)? {
            if !access.technology_evidence_exists(&reference)? {
                return Err(invalid(format!(
                    "technology evidence {reference:?} is unavailable at restoration cut {cut}"
                )));
            }
        }
        let operations = operations_at_boundary(indexes, boundary)?;
        let mut new_operations = BTreeMap::new();
        for (id, operation) in operations {
            let reference =
                canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(&id).into_untyped();
            if let Some(existing) = state.records.get(&reference) {
                let existing = existing.decode_payload::<TechnologyOperation>()?;
                if !operation
                    .input_hashes
                    .iter()
                    .all(|hash| existing.canonical_input_hashes.binary_search(hash).is_ok())
                {
                    return Err(invalid(
                        "technology operation ID was reused with different persisted input",
                    ));
                }
                continue;
            }
            new_operations.insert(id, operation);
        }
        if validate_capacity_rejection_boundary(indexes, boundary, &state, &new_operations)? {
            continue;
        }
        let reduced = reduce_new_operations(&access, &mut state, new_operations, boundary.at)?;
        validate_apply_changes(boundary, &reduced)?;
        for change in boundary
            .record_changes
            .iter()
            .filter(|change| change.plugin == crate::PLUGIN_NAME)
        {
            state
                .records
                .insert(change.current.reference.clone(), change.current.clone());
        }
    }
    Ok(())
}

fn validate_capacity_rejection_boundary(
    indexes: &ProvenanceIndexes<'_>,
    boundary: &BoundaryRecord,
    state: &TechnologyRecordSet,
    operations: &BTreeMap<String, AdmittedOperation>,
) -> Result<bool, CanwuError> {
    if operations.is_empty() {
        return Ok(false);
    }
    let limits = TechnologyLimitsV1::canonical();
    let maximum_mutations = operations.values().try_fold(0usize, |total, operation| {
        total.checked_add(2 + usize::from(operation.execution_intent().is_some()))
    });
    let capacity_exhausted = state
        .records
        .len()
        .checked_add(operations.len())
        .is_none_or(|count| count > limits.max_total_records);
    let budget_exhausted =
        maximum_mutations.is_none_or(|maximum| maximum > limits.max_mutations_per_boundary);
    if !capacity_exhausted && !budget_exhausted {
        return Ok(false);
    }
    if boundary
        .record_changes
        .iter()
        .any(|change| change.plugin == crate::PLUGIN_NAME)
    {
        return Err(invalid(
            "capacity-rejected technology boundary also mutated technology records",
        ));
    }
    let emissions = boundary
        .emissions
        .iter()
        .filter(|emission| {
            emission.plugin == crate::PLUGIN_NAME
                && emission.system == APPLY_SYSTEM
                && matches!(emission.kind, canwu_api::BoundaryEmissionKind::Explicit)
        })
        .collect::<Vec<_>>();
    if emissions.len() != operations.len()
        || emissions.iter().any(|emission| {
            indexes.events.get(&emission.event).is_none_or(|event| {
                event.timestamp != boundary.at
                    || !matches!(
                        &event.kind,
                        canwu_api::EventKind::Plugin { plugin, event_type }
                            if plugin == crate::PLUGIN_NAME
                                && event_type == CAPACITY_REJECTION_EVENT
                    )
            })
        })
    {
        return Err(invalid(
            "technology capacity rejection events do not match admitted operations",
        ));
    }
    Ok(true)
}

fn initial_technology_state(
    records: &TechnologyRecordSet,
    boundaries: &[BoundaryRecord],
) -> TechnologyRecordSet {
    let mut state = records.clone();
    state.exact_records.clear();
    for boundary in boundaries.iter().rev() {
        for change in boundary
            .record_changes
            .iter()
            .rev()
            .filter(|change| change.plugin == crate::PLUGIN_NAME)
        {
            if let Some(previous) = &change.previous {
                state
                    .records
                    .insert(previous.reference.clone(), previous.clone());
            } else {
                state.records.remove(&change.current.reference);
            }
        }
    }
    state
}

fn operations_at_boundary(
    indexes: &ProvenanceIndexes<'_>,
    boundary: &BoundaryRecord,
) -> Result<BTreeMap<String, AdmittedOperation>, CanwuError> {
    let mut operations = BTreeMap::<String, AdmittedOperation>::new();
    for ingress_id in &boundary.admitted_ingress {
        let ingress = indexes
            .ingress
            .get(ingress_id)
            .copied()
            .ok_or_else(|| invalid("technology operation ingress is unavailable"))?;
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin != crate::PLUGIN_NAME {
            continue;
        }
        let (id, input_hash, cause, change, origin) = if packet_type == TECHNOLOGY_COMMAND_INGRESS {
            let envelope: crate::model::TechnologyCommandEnvelope =
                decode_json(payload, "technology command ingress")?;
            let Some(CauseRef::Command(command_id)) = ingress.cause else {
                return Err(invalid("technology command ingress has no command cause"));
            };
            let command = indexes
                .commands
                .get(&command_id)
                .copied()
                .ok_or_else(|| invalid("technology command cause is unavailable"))?;
            if !matches!(
                &command.envelope.command,
                Command::Plugin { plugin, command, payload }
                    if plugin == crate::PLUGIN_NAME
                        && command == crate::TECHNOLOGY_COMMAND
                        && decode_json::<crate::model::TechnologyCommandEnvelope>(
                            payload,
                            "technology command evidence",
                        )
                        .is_ok_and(|value| value == envelope)
            ) {
                return Err(invalid(
                    "technology command cause body does not match admitted ingress",
                ));
            }
            let hash = canonical_hash(INPUT_HASH_DOMAIN, &envelope)?;
            (
                envelope.id,
                hash,
                EvidenceRef::Command(command_id),
                envelope.change,
                OperationOrigin::Command,
            )
        } else if packet_type == crate::TECHNOLOGY_RESULT_INGRESS {
            let envelope: crate::model::TechnologyResultEnvelope =
                decode_json(payload, "technology result ingress")?;
            let hash = canonical_hash(INPUT_HASH_DOMAIN, &envelope)?;
            (
                envelope.id,
                hash,
                EvidenceRef::Ingress(*ingress_id),
                envelope.change,
                OperationOrigin::Result {
                    provider: envelope.provider,
                    execution_intent: envelope.execution_intent,
                    ingress: EvidenceRef::Ingress(*ingress_id),
                },
            )
        } else {
            continue;
        };
        if let Some(prior) = operations.get_mut(&id) {
            prior.input_hashes.insert(input_hash.clone());
            let entry = prior.causes.entry(input_hash).or_insert(cause.clone());
            if cause < *entry {
                *entry = cause;
            }
            continue;
        }
        operations.insert(
            id.clone(),
            AdmittedOperation {
                id,
                input_hashes: BTreeSet::from([input_hash.clone()]),
                causes: BTreeMap::from([(input_hash, cause)]),
                change,
                origin,
            },
        );
    }
    Ok(operations)
}

fn validate_apply_changes(
    boundary: &BoundaryRecord,
    reduced: &[crate::plugin::ReducedOperation],
) -> Result<(), CanwuError> {
    let actual = boundary
        .record_changes
        .iter()
        .filter(|change| change.plugin == crate::PLUGIN_NAME && change.system == APPLY_SYSTEM)
        .collect::<Vec<_>>();
    validate_reduced_changes(&actual, reduced)
}

fn validate_reduced_changes(
    actual: &[&canwu_api::DomainRecordChange],
    reduced: &[crate::plugin::ReducedOperation],
) -> Result<(), CanwuError> {
    let expected_count = reduced
        .iter()
        .map(|operation| 1 + usize::from(operation.candidate.is_some()))
        .sum::<usize>();
    if actual.len() != expected_count {
        return Err(invalid(
            "technology boundary record changes do not match replayed operation count",
        ));
    }
    let mut actual = actual
        .iter()
        .map(|change| (change.current.reference.clone(), *change))
        .collect::<BTreeMap<_, _>>();
    if actual.len() != expected_count {
        return Err(invalid(
            "technology boundary repeats a phase-7 mutation target",
        ));
    }
    for operation in reduced {
        if let Some(candidate) = &operation.candidate {
            let change = actual
                .remove(&candidate.reference)
                .ok_or_else(|| invalid("replayed technology result mutation is unavailable"))?;
            let expected_operation = if operation.previous.is_some() {
                DomainRecordOperation::Updated
            } else {
                DomainRecordOperation::Created
            };
            if change.operation != expected_operation
                || change.previous != operation.previous
                || change.current != *candidate
                || change.visibility != StateVisibility::SameBoundary
                || change.summary != "Apply authority-checked technology record change"
            {
                return Err(invalid(format!(
                    "technology result mutation does not match replayed phase-7 decision: expected operation {expected_operation:?}, previous {:?}, current {:?}; found operation {:?}, previous {:?}, current {:?}",
                    operation.previous,
                    candidate,
                    change.operation,
                    change.previous,
                    change.current,
                )));
            }
        }
        let draft = operation_draft(&operation.outcome)?;
        let expected = DomainRecord {
            reference: draft.reference,
            owner: crate::PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        };
        let change = actual
            .remove(&expected.reference)
            .ok_or_else(|| invalid("replayed technology operation mutation is unavailable"))?;
        if change.operation != DomainRecordOperation::Created
            || change.previous.is_some()
            || change.current != expected
            || change.visibility != StateVisibility::SameBoundary
            || change.summary != "Record terminal technology operation outcome"
        {
            return Err(invalid(format!(
                "technology operation outcome does not match replayed phase-7 decision: expected {expected:?}; found {:?}",
                change.current,
            )));
        }
    }
    if !actual.is_empty() {
        return Err(invalid(
            "technology boundary contains an unrecognized phase-7 mutation",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn validate_consumed_intents(
    indexes: &ProvenanceIndexes<'_>,
    records: &TechnologyRecordSet,
) -> Result<(), CanwuError> {
    let operations = records.decoded::<TechnologyOperation>()?;
    let mut consumed = BTreeMap::new();
    for (reference, intent) in records.decoded::<TechnologyExecutionIntent>()? {
        let current_ref = current_record_version(indexes, &reference)?;
        let TechnologyIntentState::Consumed {
            ingress,
            operation,
            result,
        } = &intent.state
        else {
            continue;
        };
        let EvidenceRef::Ingress(ingress_id) = ingress else {
            return Err(invalid(
                "consumed technology intent has a non-ingress cause",
            ));
        };
        if !same_boundary_source(&current_ref, operation)
            || !same_boundary_source(&current_ref, result)
        {
            return Err(invalid(
                "technology intent consumption, operation, and result are not atomic",
            ));
        }
        let operation_record = indexes
            .canwu
            .domain_record_version(operation)
            .ok_or_else(|| invalid("consuming technology operation is unavailable"))?;
        let outcome = operation_record.decode_payload::<TechnologyOperation>()?;
        let result_record = indexes
            .canwu
            .domain_record_version(result)
            .ok_or_else(|| invalid("consumed technology result is unavailable"))?;
        let ingress_record = indexes
            .ingress
            .get(ingress_id)
            .copied()
            .ok_or_else(|| invalid("consumed technology provider ingress is unavailable"))?;
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress_record.payload
        else {
            return Err(invalid("consumed technology cause is not plugin ingress"));
        };
        let envelope: crate::model::TechnologyResultEnvelope =
            decode_json(payload, "consumed technology result ingress")?;
        let execution_intent = envelope.execution_intent.as_ref().ok_or_else(|| {
            invalid("consumed technology result ingress has no exact execution intent")
        })?;
        let pending_record = indexes
            .canwu
            .domain_record_version(execution_intent)
            .ok_or_else(|| invalid("pending technology intent version is unavailable"))?;
        let mut expected_pending = intent.clone();
        expected_pending.state = TechnologyIntentState::Pending;
        validate_consumed_operation_cause(&outcome, ingress)?;
        validate_finalizer_change(indexes, &current_ref, execution_intent, &pending_record)?;
        if plugin != crate::PLUGIN_NAME
            || packet_type != crate::TECHNOLOGY_RESULT_INGRESS
            || envelope.id != outcome.id
            || envelope.provider != intent.provider
            || execution_intent.record != current_ref.record
            || pending_record.decode_payload::<TechnologyExecutionIntent>()? != expected_pending
            || outcome.status != TechnologyOperationStatus::Applied
            || outcome.execution_intent.as_ref() != Some(execution_intent)
            || outcome.provider.as_ref() != Some(&intent.provider)
            || outcome.result.as_ref() != Some(&result.record)
            || result_record.reference != result.record
            || change_id(&envelope.change) != result.record.id
            || change_value(&envelope.change).reference(change_id(&envelope.change))
                != result.record
            || decode_runtime_payload(&result_record)?.as_ref()
                != Some(change_value(&envelope.change))
        {
            return Err(invalid(
                "consumed technology intent does not reproduce its ingress, operation, and result",
            ));
        }
        validate_intent_result_payload(execution_intent, &intent, &result_record)?;
        if consumed
            .insert(
                execution_intent.clone(),
                (operation.clone(), result.clone()),
            )
            .is_some()
        {
            return Err(invalid(
                "technology execution intent was consumed more than once",
            ));
        }
    }
    for (reference, outcome) in operations {
        if outcome.status == TechnologyOperationStatus::Applied
            && let Some(intent) = &outcome.execution_intent
        {
            let Some((operation, result)) = consumed.get(intent) else {
                return Err(invalid(
                    "applied provider operation did not consume its exact intent",
                ));
            };
            if operation.record != reference || outcome.result.as_ref() != Some(&result.record) {
                return Err(invalid(
                    "consumed technology intent points to another operation or result",
                ));
            }
        }
    }
    Ok(())
}

fn validate_consumed_operation_cause(
    outcome: &crate::model::TechnologyOperationPayload,
    ingress: &EvidenceRef,
) -> Result<(), CanwuError> {
    if outcome.causes == [ingress.clone()] {
        Ok(())
    } else {
        Err(invalid(
            "consuming technology operation cause is not the exact provider ingress",
        ))
    }
}

fn validate_finalizer_change(
    indexes: &ProvenanceIndexes<'_>,
    consumed: &DomainRecordVersionRef,
    pending: &DomainRecordVersionRef,
    pending_record: &DomainRecord,
) -> Result<(), CanwuError> {
    let DomainRecordVersionSource::BoundaryChange {
        boundary,
        change_index,
    } = &consumed.established_by
    else {
        return Err(invalid(
            "consumed technology intent was not established by a boundary finalizer",
        ));
    };
    let boundary = indexes
        .boundaries
        .get(boundary)
        .map(|(_, record)| *record)
        .ok_or_else(|| invalid("technology intent finalizer boundary is unavailable"))?;
    let index = usize::try_from(*change_index)
        .map_err(|_| invalid("technology intent finalizer index overflowed"))?;
    let change = boundary
        .record_changes
        .get(index)
        .ok_or_else(|| invalid("technology intent finalizer change is unavailable"))?;
    if change.plugin != crate::PLUGIN_NAME
        || change.system != FINALIZE_SYSTEM
        || change.operation != DomainRecordOperation::Updated
        || change.current.reference != consumed.record
        || change.current.version != consumed.version
        || change.previous.as_ref() != Some(pending_record)
        || pending.record != consumed.record
        || pending.version.checked_add(1) != Some(consumed.version)
    {
        return Err(invalid(
            "technology intent consumption was not written by the exact phase-12 finalizer",
        ));
    }
    Ok(())
}

fn validate_intent_result_payload(
    intent_ref: &DomainRecordVersionRef,
    intent: &crate::model::TechnologyExecutionIntentPayload,
    result: &DomainRecord,
) -> Result<(), CanwuError> {
    let payload = decode_runtime_payload(result)?
        .ok_or_else(|| invalid("technology intent consumed a non-runtime result"))?;
    let matches = match (&intent.request, payload) {
        (
            TechnologyIntentRequest::Experiment {
                result_id,
                revision,
                operation,
                site,
                operator,
                required_assets,
            },
            crate::model::TechnologyRecordPayload::ExperimentAttempt(value),
        ) => {
            result.reference.id == *result_id
                && value.execution_intent == *intent_ref
                && value.program == intent.program
                && value.revision == *revision
                && value.operation == *operation
                && value.site == *site
                && operator
                    .as_ref()
                    .is_none_or(|expected| expected == &value.operator)
                && exact_set(required_assets) == exact_set(&value.assets)
        }
        (
            TechnologyIntentRequest::Production {
                result_id,
                revision,
                application,
                site,
                operator,
                required_assets,
            },
            crate::model::TechnologyRecordPayload::ProductionRun(value),
        ) => {
            result.reference.id == *result_id
                && value.execution_intent == *intent_ref
                && value.revision == *revision
                && value.application == *application
                && value.site == *site
                && operator
                    .as_ref()
                    .is_none_or(|expected| expected == &value.operator)
                && exact_set(required_assets) == exact_set(&value.assets)
        }
        (
            TechnologyIntentRequest::Invention {
                result_id,
                spec,
                parent,
                ..
            },
            crate::model::TechnologyRecordPayload::TechniqueRevision(value),
        ) => {
            result.reference.id == *result_id
                && value.execution_intent.as_ref() == Some(intent_ref)
                && value.produced_by.as_ref() == Some(&intent.program)
                && value.spec == *spec
                && parent.as_ref().is_none_or(|expected| {
                    value
                        .parents
                        .iter()
                        .any(|relation| &relation.parent == expected)
                })
        }
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(invalid(
            "consumed technology result does not match its authorized request",
        ))
    }
}

fn exact_set(values: &[DomainRecordVersionRef]) -> BTreeSet<&DomainRecordVersionRef> {
    values.iter().collect()
}

fn current_record_version(
    indexes: &ProvenanceIndexes<'_>,
    reference: &DomainRecordRef,
) -> Result<DomainRecordVersionRef, CanwuError> {
    let record = indexes
        .canwu
        .domain_record(reference)
        .ok_or_else(|| invalid("current technology record is unavailable"))?;
    if let Some(version) = indexes
        .record_versions
        .get(&(reference.clone(), record.version))
    {
        return Ok(version.clone());
    }
    Ok(DomainRecordVersionRef {
        record: reference.clone(),
        version: record.version,
        established_by: DomainRecordVersionSource::InitialScenario,
    })
}

fn same_boundary_source(left: &DomainRecordVersionRef, right: &DomainRecordVersionRef) -> bool {
    matches!(
        (&left.established_by, &right.established_by),
        (
            DomainRecordVersionSource::BoundaryChange { boundary: left, .. },
            DomainRecordVersionSource::BoundaryChange { boundary: right, .. },
        ) if left == right
    )
}

fn change_id(change: &crate::model::TechnologyRecordChange) -> &str {
    match change {
        crate::model::TechnologyRecordChange::Create { id, .. }
        | crate::model::TechnologyRecordChange::Update { id, .. } => id,
    }
}

fn change_value(
    change: &crate::model::TechnologyRecordChange,
) -> &crate::model::TechnologyRecordPayload {
    match change {
        crate::model::TechnologyRecordChange::Create { value, .. }
        | crate::model::TechnologyRecordChange::Update { value, .. } => value,
    }
}

fn decode_json<T: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
    label: &str,
) -> Result<T, CanwuError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        invalid(format!(
            "{label} could not be decoded during restoration: {error}"
        ))
    })
}

pub fn validate_technology_runtime(canwu: &Canwu) -> Result<(), CanwuError> {
    let records = TechnologyRecordSet::load_host(canwu)?;
    records.validate(canwu.time())?;
    records.validate_temporal_evidence(canwu)?;
    let indexes = ProvenanceIndexes::new(canwu)?;
    validate_operation_provenance(&indexes, &records)?;
    validate_consumed_intents(&indexes, &records)?;
    for reference in exact_versions(&records)? {
        if !canwu.domain_record_version_evidence_exists(&reference) {
            return Err(invalid(format!(
                "exact technology evidence {reference:?} is unavailable"
            )));
        }
    }
    for reference in evidence_refs(&records)? {
        if !canwu.evidence_exists(&reference) {
            return Err(invalid(format!(
                "technology evidence {reference:?} is unavailable"
            )));
        }
    }
    validate_technology_knowledge(canwu)?;
    Ok(())
}

#[derive(serde::Deserialize)]
struct PublishedTechnologyKnowledge {
    record_version: u64,
    record: crate::model::TechnologyRecordPayload,
}

fn validate_technology_knowledge(canwu: &Canwu) -> Result<(), CanwuError> {
    validate_technology_knowledge_records(canwu.knowledge(), |version| {
        canwu.domain_record_version(version)
    })
}

fn validate_technology_knowledge_records(
    knowledge: &canwu_api::KnowledgeSnapshot,
    mut resolve_version: impl FnMut(&DomainRecordVersionRef) -> Option<DomainRecord>,
) -> Result<(), CanwuError> {
    let mut total = 0usize;
    for (holder, records) in &knowledge.records {
        for record in records
            .values()
            .filter(|record| record.schema.kind.namespace == crate::PLUGIN_NAMESPACE)
        {
            total = total
                .checked_add(1)
                .ok_or_else(|| invalid("technology knowledge count overflowed"))?;
            if total > TechnologyLimitsV1::canonical().max_knowledge_records {
                return Err(invalid("technology knowledge exceeds its shared total cap"));
            }
            let [subject] = record.subjects.as_slice() else {
                return Err(invalid(
                    "technology knowledge must bind exactly one record subject",
                ));
            };
            let KnowledgeSubjectTarget::DomainRecord(subject_record) = &subject.target else {
                return Err(invalid(
                    "technology knowledge subject is not a domain record",
                ));
            };
            if subject.role != "record"
                || record.schema.version != 1
                || record.holder != *holder
                || record.confidence_per_mille != 1_000
                || record.as_of != Some(record.learned_at)
                || record.origin.method != "technology_record_evidence_v1"
                || !record.supersedes.is_empty()
                || !record.contradicts.is_empty()
            {
                return Err(invalid(
                    "technology knowledge metadata does not match its publication contract",
                ));
            }
            let [EvidenceRef::DomainRecordVersion(version)] = record.origin.evidence.as_slice()
            else {
                return Err(invalid(
                    "technology knowledge must cite one exact record version",
                ));
            };
            let published: PublishedTechnologyKnowledge =
                decode_json(&record.payload, "technology knowledge payload")?;
            if version.record != *subject_record
                || published.record_version != version.version
                || published.record.reference(subject_record.id.clone()) != *subject_record
                || published.record.knowledge_holder() != Some(holder)
                || expected_knowledge_schema(&published.record)
                    != Some(record.schema.kind.name.as_str())
            {
                return Err(invalid(
                    "technology knowledge holder, schema, subject, or version binding is invalid",
                ));
            }
            let exact = resolve_version(version)
                .ok_or_else(|| invalid("technology knowledge exact record body is unavailable"))?;
            let exact_payload = decode_runtime_payload(&exact)?.ok_or_else(|| {
                invalid("technology knowledge cites a non-publishable record kind")
            })?;
            if exact_payload != published.record {
                return Err(invalid(
                    "technology knowledge payload does not match its exact record evidence",
                ));
            }
        }
    }
    Ok(())
}

fn expected_knowledge_schema(
    value: &crate::model::TechnologyRecordPayload,
) -> Option<&'static str> {
    match value {
        crate::model::TechnologyRecordPayload::TechnicalClaim(_) => {
            Some(crate::schema::CLAIM_KNOWLEDGE)
        }
        crate::model::TechnologyRecordPayload::AttemptObservation(_) => {
            Some(crate::schema::ATTEMPT_KNOWLEDGE)
        }
        crate::model::TechnologyRecordPayload::Capability(_) => {
            Some(crate::schema::CAPABILITY_KNOWLEDGE)
        }
        crate::model::TechnologyRecordPayload::Implementation(_) => {
            Some(crate::schema::IMPLEMENTATION_KNOWLEDGE)
        }
        crate::model::TechnologyRecordPayload::Adoption(_) => {
            Some(crate::schema::ADOPTION_KNOWLEDGE)
        }
        crate::model::TechnologyRecordPayload::TechniqueRevision(_)
        | crate::model::TechnologyRecordPayload::TechnicalProgram(_)
        | crate::model::TechnologyRecordPayload::ExecutionIntent(_)
        | crate::model::TechnologyRecordPayload::ExperimentAttempt(_)
        | crate::model::TechnologyRecordPayload::ClaimAssessment(_)
        | crate::model::TechnologyRecordPayload::AssetBinding(_)
        | crate::model::TechnologyRecordPayload::ProductionRun(_)
        | crate::model::TechnologyRecordPayload::Transmission(_) => None,
    }
}

pub fn from_technology_snapshot_json(
    json: &str,
    plugins: &[&dyn canwu_api::SimulationPlugin],
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::from_snapshot_json_with_plugins(json, plugins)?;
    validate_technology_runtime(&canwu)?;
    Ok(canwu)
}

pub fn from_technology_checkpoint_journal(
    bundle: canwu_api::CheckpointJournal,
    plugins: &[&dyn canwu_api::SimulationPlugin],
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::from_checkpoint_journal_with_plugins(bundle, plugins)?;
    validate_technology_runtime(&canwu)?;
    Ok(canwu)
}

pub fn replay_technology_from_journal(
    scenario: canwu_api::Scenario,
    plugins: &[&dyn canwu_api::SimulationPlugin],
    journal: &canwu_api::ReplayJournal,
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::replay_from_journal(scenario, plugins, journal)?;
    validate_technology_runtime(&canwu)?;
    Ok(canwu)
}

fn exact_versions(
    records: &TechnologyRecordSet,
) -> Result<BTreeSet<DomainRecordVersionRef>, CanwuError> {
    let mut values = BTreeSet::new();
    for record in records.records.values() {
        values.extend(exact_versions_in_record(record)?);
    }
    Ok(values)
}

fn evidence_refs(
    records: &TechnologyRecordSet,
) -> Result<BTreeSet<canwu_api::EvidenceRef>, CanwuError> {
    let mut values = BTreeSet::new();
    for record in records.records.values() {
        if record.reference.kind.matches_type::<TechnologyOperation>() {
            values.extend(record.decode_payload::<TechnologyOperation>()?.causes);
        } else if record.reference.kind.matches_type::<TechniqueRevision>() {
            values.extend(
                record
                    .decode_payload::<TechniqueRevision>()?
                    .discovery_evidence,
            );
        } else if let Some(payload) = decode_runtime_payload(record)? {
            values.extend(payload.evidence_refs());
        }
    }
    Ok(values)
}

fn exact_versions_in_record(
    record: &DomainRecord,
) -> Result<Vec<DomainRecordVersionRef>, CanwuError> {
    let kind = &record.reference.kind;
    let mut values = if kind.matches_type::<TechniqueSpec>() {
        record
            .decode_payload::<TechniqueSpec>()?
            .requirements
            .into_iter()
            .flat_map(|group| group.any_of)
            .map(|threshold| threshold.metric)
            .collect()
    } else if kind.matches_type::<TechniqueRevision>() {
        let payload = record.decode_payload::<TechniqueRevision>()?;
        std::iter::once(payload.spec)
            .chain(payload.parents.into_iter().map(|parent| parent.parent))
            .chain(payload.parameters.into_iter().map(|value| value.metric))
            .chain(payload.produced_by)
            .chain(payload.execution_intent)
            .collect()
    } else if kind.matches_type::<ApplicationSpec>() {
        let payload = record.decode_payload::<ApplicationSpec>()?;
        std::iter::once(payload.technique)
            .chain(
                payload
                    .viability
                    .into_iter()
                    .flat_map(|group| group.any_of)
                    .map(|threshold| threshold.metric),
            )
            .collect()
    } else if kind.matches_type::<TechnologyOperation>() {
        let payload = record.decode_payload::<TechnologyOperation>()?;
        payload.execution_intent.into_iter().collect()
    } else if let Some(payload) = decode_runtime_payload(record)? {
        payload.exact_versions()
    } else {
        Vec::new()
    };
    values.sort();
    values.dedup();
    Ok(values)
}

fn validate_payload_continuation(record: &DomainRecord) -> Result<(), CanwuError> {
    let mut expected = record.payload.clone();
    attach_payload_continuation(&mut expected, exact_versions_in_record(record)?)?;
    if expected != record.payload {
        return Err(invalid(format!(
            "technology record {} has a noncanonical payload continuation",
            record.reference
        )));
    }
    Ok(())
}

fn enforce_exact_record_bound(records: &TechnologyRecordSet) -> Result<(), CanwuError> {
    let maximum = TechnologyLimitsV1::canonical()
        .max_records_per_kind
        .checked_mul(technology_record_kinds().len())
        .ok_or_else(|| invalid("technology exact-record bound overflow"))?;
    if records.exact_records.len() > maximum {
        return Err(invalid(
            "technology exact-record working set exceeds its bound",
        ));
    }
    Ok(())
}

pub(crate) fn decode_runtime_payload(
    record: &DomainRecord,
) -> Result<Option<crate::model::TechnologyRecordPayload>, CanwuError> {
    use crate::model::TechnologyRecordPayload as Payload;
    let kind = &record.reference.kind;
    let payload = if kind.matches_type::<TechnicalProgram>() {
        Payload::TechnicalProgram(record.decode_payload::<TechnicalProgram>()?)
    } else if kind.matches_type::<TechnologyExecutionIntent>() {
        Payload::ExecutionIntent(record.decode_payload::<TechnologyExecutionIntent>()?)
    } else if kind.matches_type::<ExperimentAttempt>() {
        Payload::ExperimentAttempt(record.decode_payload::<ExperimentAttempt>()?)
    } else if kind.matches_type::<AttemptObservation>() {
        Payload::AttemptObservation(record.decode_payload::<AttemptObservation>()?)
    } else if kind.matches_type::<TechnicalClaim>() {
        Payload::TechnicalClaim(record.decode_payload::<TechnicalClaim>()?)
    } else if kind.matches_type::<ClaimAssessment>() {
        Payload::ClaimAssessment(record.decode_payload::<ClaimAssessment>()?)
    } else if kind.matches_type::<CapabilityQualification>() {
        Payload::Capability(record.decode_payload::<CapabilityQualification>()?)
    } else if kind.matches_type::<AssetBinding>() {
        Payload::AssetBinding(record.decode_payload::<AssetBinding>()?)
    } else if kind.matches_type::<ProductionRun>() {
        Payload::ProductionRun(record.decode_payload::<ProductionRun>()?)
    } else if kind.matches_type::<ImplementationRecord>() {
        Payload::Implementation(record.decode_payload::<ImplementationRecord>()?)
    } else if kind.matches_type::<AdoptionRecord>() {
        Payload::Adoption(record.decode_payload::<AdoptionRecord>()?)
    } else if kind.matches_type::<TransmissionOpportunity>() {
        Payload::Transmission(record.decode_payload::<TransmissionOpportunity>()?)
    } else if kind.matches_type::<TechniqueRevision>() {
        Payload::TechniqueRevision(record.decode_payload::<TechniqueRevision>()?)
    } else {
        return Ok(None);
    };
    Ok(Some(payload))
}

fn validate_group(
    set: &TechnologyRecordSet,
    group: &crate::model::RequirementGroup,
) -> Result<(), CanwuError> {
    canonical_text(&group.id, "requirement group")?;
    if group.any_of.is_empty() {
        return Err(invalid("requirement group has no alternatives"));
    }
    bounded(&group.any_of, "requirement alternatives")?;
    for threshold in &group.any_of {
        canonical_text(&threshold.id, "threshold")?;
        let metric = set.decode_version::<MetricSchema>(&threshold.metric)?;
        if threshold.value < metric.minimum || threshold.value > metric.maximum {
            return Err(invalid("requirement threshold is outside its metric range"));
        }
    }
    Ok(())
}

fn validate_metric_values(
    set: &TechnologyRecordSet,
    values: &[crate::model::MetricValue],
) -> Result<(), CanwuError> {
    bounded(values, "metric values")?;
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value.metric.record.clone()) {
            return Err(invalid("metric values contain a duplicate metric"));
        }
        let metric = set.decode_version::<MetricSchema>(&value.metric)?;
        if value.value < metric.minimum || value.value > metric.maximum {
            return Err(invalid("metric value is outside its schema range"));
        }
    }
    Ok(())
}

fn validate_evaluation(value: &crate::model::EvaluationResult) -> Result<(), CanwuError> {
    bounded(&value.satisfied_groups, "satisfied evaluation groups")?;
    bounded(&value.failed_groups, "failed evaluation groups")?;
    for group in value.satisfied_groups.iter().chain(&value.failed_groups) {
        canonical_text(group, "evaluation group")?;
    }
    Ok(())
}

fn bounded<T>(values: &[T], label: &str) -> Result<(), CanwuError> {
    if values.len() > TechnologyLimitsV1::canonical().max_collection_entries {
        return Err(invalid(format!("{label} exceeds its canonical bound")));
    }
    Ok(())
}

fn canonical_text(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(invalid(format!("{label} is not bounded canonical text")));
    }
    Ok(())
}

fn valid_rejection_code(value: &str) -> bool {
    matches!(
        value,
        "invalid_domain_record"
            | "domain_record_not_found"
            | "domain_record_version_conflict"
            | "duplicate_domain_record"
            | "idempotency_conflict"
            | "technology_operation_rejected"
    )
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

fn invalid_encoding(error: &serde_json::Error) -> CanwuError {
    invalid(format!("technology record could not be encoded: {error}"))
}

fn holder_entity(holder: &KnowledgeHolderRef) -> EntityRef {
    match holder {
        KnowledgeHolderRef::Person(person) => EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        MetricSchema, ProgramMode, ProgramStatus, TechnicalProgramPayload,
        TechnologyOperationPayload, TechnologyRecordPayload,
    };
    use crate::plugin::{ReducedOperation, operation_draft};
    use canwu_api::{
        BoundaryId, DomainRecordChange, DomainRecordDraft,
        PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadRequiredEvidenceContinuationV1,
        PersonId, TerritoryId, TypedDomainRecordRef,
    };

    fn record_from_draft(draft: DomainRecordDraft) -> DomainRecord {
        DomainRecord {
            reference: draft.reference,
            owner: crate::PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        }
    }

    fn operation_change(outcome: &TechnologyOperationPayload) -> DomainRecordChange {
        DomainRecordChange {
            plugin: crate::PLUGIN_NAME.to_owned(),
            system: APPLY_SYSTEM.to_owned(),
            operation: DomainRecordOperation::Created,
            previous: None,
            current: record_from_draft(operation_draft(outcome).expect("operation draft")),
            visibility: StateVisibility::SameBoundary,
            summary: "Record terminal technology operation outcome".to_owned(),
        }
    }

    #[test]
    fn payload_continuation_is_recomputed_from_exact_dependencies() {
        let revision = DomainRecordVersionRef {
            record: TypedDomainRecordRef::<TechniqueRevision>::new("revision").into_untyped(),
            version: 2,
            established_by: DomainRecordVersionSource::BoundaryChange {
                boundary: BoundaryId::new(7),
                change_index: 3,
            },
        };
        let draft = TechnologyRecordPayload::TechnicalProgram(TechnicalProgramPayload {
            sponsor: KnowledgeHolderRef::Person(PersonId::new(1)),
            site: EntityRef::Territory(TerritoryId::new(1)),
            revision: Some(revision),
            mode: ProgramMode::Investigation,
            status: ProgramStatus::Active,
            requirements: vec![],
            started_at: canwu_api::SimTime::EPOCH,
            due_at: None,
        })
        .draft("program")
        .expect("program draft");
        let mut record = record_from_draft(draft);
        validate_payload_continuation(&record).expect("generated continuation should validate");
        record.payload[PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD] =
            serde_json::to_value(PayloadRequiredEvidenceContinuationV1::completed())
                .expect("completed continuation");
        assert!(validate_payload_continuation(&record).is_err());
    }

    #[test]
    fn replayed_changes_reject_terminal_status_code_target_and_cause_tampering() {
        let result_ref = TypedDomainRecordRef::<MetricSchema>::new("result").into_untyped();
        let candidate = DomainRecord {
            reference: result_ref.clone(),
            owner: crate::PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: serde_json::json!({"label":"result"}),
            references: Vec::new(),
        };
        let outcome = TechnologyOperationPayload {
            id: "operation".to_owned(),
            canonical_input_hash: "a".repeat(64),
            canonical_input_hashes: vec!["a".repeat(64)],
            causes: vec![EvidenceRef::Ingress(IngressId::new(1))],
            provider: Some("provider".to_owned()),
            execution_intent: None,
            status: TechnologyOperationStatus::Applied,
            result: Some(result_ref),
            rejection_code: None,
        };
        let reduced = vec![ReducedOperation {
            outcome: outcome.clone(),
            mutation: None,
            candidate: Some(candidate.clone()),
            previous: None,
        }];
        let result_change = DomainRecordChange {
            plugin: crate::PLUGIN_NAME.to_owned(),
            system: APPLY_SYSTEM.to_owned(),
            operation: DomainRecordOperation::Created,
            previous: None,
            current: candidate,
            visibility: StateVisibility::SameBoundary,
            summary: "Apply authority-checked technology record change".to_owned(),
        };
        let operation_record_change = operation_change(&outcome);
        assert!(
            validate_reduced_changes(&[&result_change, &operation_record_change], &reduced).is_ok()
        );

        let mut status = operation_record_change.clone();
        status.current.payload["status"] = serde_json::json!("rejected");
        status.current.payload["result"] = serde_json::Value::Null;
        status.current.payload["rejection_code"] = serde_json::json!("invalid_domain_record");
        assert!(validate_reduced_changes(&[&result_change, &status], &reduced).is_err());

        let mut target = result_change.clone();
        target.current.reference.id = "other-result".to_owned();
        assert!(validate_reduced_changes(&[&target, &operation_record_change], &reduced).is_err());

        let mut cause = operation_record_change;
        cause.current.payload["causes"] = serde_json::json!([{
            "type": "ingress",
            "value": 2
        }]);
        assert!(validate_reduced_changes(&[&result_change, &cause], &reduced).is_err());

        let mut rejected = outcome;
        rejected.status = TechnologyOperationStatus::Rejected;
        rejected.result = None;
        rejected.rejection_code = Some("invalid_domain_record".to_owned());
        let rejected_reduced = vec![ReducedOperation {
            outcome: rejected.clone(),
            mutation: None,
            candidate: None,
            previous: None,
        }];
        let mut wrong_code = operation_change(&rejected);
        wrong_code.current.payload["rejection_code"] = serde_json::json!("domain_record_not_found");
        assert!(validate_reduced_changes(&[&wrong_code], &rejected_reduced).is_err());
    }

    #[test]
    fn consumed_operation_requires_the_exact_provider_ingress_cause() {
        let outcome = TechnologyOperationPayload {
            id: "operation".to_owned(),
            canonical_input_hash: "b".repeat(64),
            canonical_input_hashes: vec!["b".repeat(64)],
            causes: vec![EvidenceRef::Ingress(IngressId::new(7))],
            provider: Some("provider".to_owned()),
            execution_intent: None,
            status: TechnologyOperationStatus::Applied,
            result: Some(TypedDomainRecordRef::<MetricSchema>::new("result").into_untyped()),
            rejection_code: None,
        };
        assert!(
            validate_consumed_operation_cause(&outcome, &EvidenceRef::Ingress(IngressId::new(7)))
                .is_ok()
        );
        assert!(
            validate_consumed_operation_cause(&outcome, &EvidenceRef::Ingress(IngressId::new(8)))
                .is_err()
        );
    }

    #[test]
    fn restore_visibility_rejects_future_versions_and_evidence_at_the_phase_7_cut() {
        let boundary_cuts = BTreeMap::from([
            (BoundaryId::new(1), 0usize),
            (BoundaryId::new(2), 1usize),
            (BoundaryId::new(3), 2usize),
        ]);
        let version = |boundary| DomainRecordVersionRef {
            record: TypedDomainRecordRef::<MetricSchema>::new("metric").into_untyped(),
            version: boundary,
            established_by: DomainRecordVersionSource::BoundaryChange {
                boundary: BoundaryId::new(boundary),
                change_index: 0,
            },
        };
        let initial = DomainRecordVersionRef {
            record: TypedDomainRecordRef::<MetricSchema>::new("initial").into_untyped(),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        };

        assert!(exact_source_visible_at(&initial, &boundary_cuts, 1));
        assert!(exact_source_visible_at(&version(1), &boundary_cuts, 1));
        assert!(
            !exact_source_visible_at(&version(2), &boundary_cuts, 1),
            "a phase-7 reducer cannot consume a version established later in its own boundary"
        );
        assert!(
            !exact_source_visible_at(&version(3), &boundary_cuts, 1),
            "future-boundary versions must remain invisible"
        );

        assert!(
            evidence_id_visible_at(Some(1), 1),
            "commands and ingress admitted at the current boundary are visible"
        );
        assert!(
            !evidence_id_visible_at(Some(2), 1),
            "future commands and ingress are invisible"
        );
        assert!(
            boundary_cuts
                .get(&BoundaryId::new(2))
                .is_none_or(|established| *established >= 1),
            "the current boundary is not evidence until settlement completes"
        );
    }

    #[test]
    fn restoration_deeply_validates_technology_knowledge_bindings() {
        let holder = KnowledgeHolderRef::Person(canwu_api::PersonId::new(1));
        let value = crate::model::TechnologyRecordPayload::TechnicalClaim(
            crate::model::TechnicalClaimPayload {
                asserted_by: holder.clone(),
                proposition: "exactly bound claim".to_owned(),
                scope: Vec::new(),
                source_evidence: Vec::new(),
                relations: Vec::new(),
                asserted_at: canwu_api::SimTime::EPOCH,
            },
        );
        let exact = record_from_draft(value.draft("claim").expect("claim draft"));
        let version = DomainRecordVersionRef {
            record: exact.reference.clone(),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        };
        let knowledge_id = canwu_api::KnowledgeRecordId::new(1);
        let knowledge_record = canwu_api::KnowledgeRecord {
            id: knowledge_id,
            holder: holder.clone(),
            schema: canwu_api::KnowledgeSchemaId::new(
                canwu_api::KnowledgeRecordKind::new(
                    crate::PLUGIN_NAMESPACE,
                    crate::schema::CLAIM_KNOWLEDGE,
                ),
                1,
            ),
            subjects: vec![canwu_api::KnowledgeSubject {
                role: "record".to_owned(),
                target: KnowledgeSubjectTarget::DomainRecord(exact.reference.clone()),
            }],
            payload: serde_json::json!({
                "record_version": 1,
                "record": value,
            }),
            as_of: Some(canwu_api::SimTime::EPOCH),
            learned_at: canwu_api::SimTime::EPOCH,
            confidence_per_mille: 1_000,
            origin: canwu_api::KnowledgeOrigin {
                method: "technology_record_evidence_v1".to_owned(),
                evidence: vec![EvidenceRef::DomainRecordVersion(version.clone())],
            },
            supersedes: Vec::new(),
            contradicts: Vec::new(),
        };
        let mut knowledge = canwu_api::KnowledgeSnapshot::default();
        knowledge
            .records
            .entry(holder)
            .or_default()
            .insert(knowledge_id, knowledge_record);
        assert!(
            validate_technology_knowledge_records(&knowledge, |reference| {
                (reference == &version).then(|| exact.clone())
            })
            .is_ok()
        );

        knowledge
            .records
            .values_mut()
            .next()
            .expect("holder")
            .values_mut()
            .next()
            .expect("record")
            .payload["record_version"] = serde_json::json!(2);
        assert!(
            validate_technology_knowledge_records(&knowledge, |reference| {
                (reference == &version).then(|| exact.clone())
            })
            .is_err(),
            "a schema-valid but wrongly bound knowledge payload must fail restoration"
        );
    }
}
