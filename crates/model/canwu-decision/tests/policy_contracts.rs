use canwu_core::{DecisionRequestId, DecisionTicketId, EntityRef, PersonId};
use canwu_decision::{
    DecisionAction, DecisionArchiveBlob, DecisionArchiveProvider, DecisionArchiveRecord,
    DecisionArchiveStore, DecisionArchiveStoreOutcome, DecisionAttemptOutcome,
    DecisionAttemptRecord, DecisionContext, DecisionError, DecisionErrorCode, DecisionHistoryKey,
    DecisionHistoryLocation, DecisionHistoryQueryBudget, DecisionOption, DecisionPolicy,
    DecisionState, DecisionTicket, DecisionTicketState, ExternalDecisionResponse, ExternalPolicy,
    HumanDecisionResponse, LlmModelIdentity, MAX_DECISION_ARCHIVE_BUCKET_PAGE_BYTES,
    MAX_DECISION_ARCHIVE_BUCKET_PAGE_ENTRIES, QueuedExternalPolicy, QueuedHumanPolicy,
    QueuedLlmPolicy, decision_history_page_key,
};
use canwu_time::SimTime;
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Default)]
struct DecisionStore(RefCell<BTreeMap<String, DecisionArchiveBlob>>);

impl DecisionArchiveProvider for DecisionStore {
    fn load_decision_archive(
        &self,
        locator: &str,
    ) -> Result<Option<DecisionArchiveBlob>, DecisionError> {
        Ok(self.0.borrow().get(locator).cloned())
    }
}

impl DecisionArchiveStore for DecisionStore {
    fn store_decision_archive(
        &self,
        blob: &DecisionArchiveBlob,
    ) -> Result<DecisionArchiveStoreOutcome, DecisionError> {
        let locator = blob.content_id()?;
        let mut stored = self.0.borrow_mut();
        if let Some(existing) = stored.get(&locator) {
            return if existing == blob {
                Ok(DecisionArchiveStoreOutcome::AlreadyStored)
            } else {
                Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "conflicting decision archive content",
                ))
            };
        }
        stored.insert(locator, blob.clone());
        Ok(DecisionArchiveStoreOutcome::Stored)
    }
}

fn ticket(version: u64) -> DecisionTicket {
    DecisionTicket {
        id: DecisionTicketId::new(7),
        definition: "diplomacy.answer-aid-request".to_owned(),
        decision_maker: EntityRef::Person(PersonId::new(3)),
        assigned_controller: "neighbor-controller".to_owned(),
        summary: "Choose a response to the neighboring warlord".to_owned(),
        context: DecisionContext::new("diplomacy.aid-request.v1", json!({"known": true})),
        options: vec![DecisionOption {
            description: "Send a field army".to_owned(),
            action: DecisionAction::Command {
                command: json!({"private_command": "must-not-cross-policy-boundary"}),
            },
            metadata: json!({"public_hint": "supports ally"}),
            ..DecisionOption::new("send-aid", "Send aid")
        }],
        opened_at: SimTime::EPOCH,
        updated_at: SimTime::EPOCH,
        deadline: None,
        version,
        state: DecisionTicketState::Open,
    }
}

#[test]
fn decision_attempt_index_rebuilds_from_the_persisted_journal() {
    let request_id = DecisionRequestId::new(41);
    let attempt = DecisionAttemptRecord {
        request_id,
        request_commitment: "a".repeat(64),
        at: SimTime::EPOCH,
        revision_before: 7,
        expected_revision: 7,
        outcome: DecisionAttemptOutcome::Accepted {
            trace_id: None,
            command_request_id: None,
        },
    };
    let mut state = DecisionState::default();
    state
        .append_attempt(attempt.clone())
        .expect("unique attempt should append");
    let encoded = serde_json::to_string(&state).expect("decision state should serialize");
    assert!(!encoded.contains("attempts_by_request"));

    let restored: DecisionState =
        serde_json::from_str(&encoded).expect("decision state should deserialize");
    assert_eq!(restored.attempt(request_id), Some(&attempt));
    restored.validate().expect("rebuilt index should validate");
}

#[test]
fn hot_history_accumulator_is_restart_stable_and_reversible() {
    let mut state = DecisionState::default();
    let empty_root = state
        .hot_history_commitment()
        .expect("empty hot-history commitment");
    let request_id = DecisionRequestId::new(42);
    state
        .append_attempt(DecisionAttemptRecord {
            request_id,
            request_commitment: "d".repeat(64),
            at: SimTime::EPOCH,
            revision_before: 4,
            expected_revision: 4,
            outcome: DecisionAttemptOutcome::Rejected {
                code: canwu_decision::DecisionAttemptErrorCode::InvalidDecision,
                message: "restart-stable accumulator".to_owned(),
            },
        })
        .expect("append attempt");
    let populated_root = state
        .hot_history_commitment()
        .expect("populated hot-history commitment");
    assert_ne!(populated_root, empty_root);

    let encoded = serde_json::to_vec(&state).expect("serialize decision state");
    let restarted: DecisionState =
        serde_json::from_slice(&encoded).expect("restart decision state");
    assert_eq!(
        restarted
            .hot_history_commitment()
            .expect("restarted commitment"),
        populated_root
    );

    let key = DecisionHistoryKey::Attempt(request_id);
    let prepared = restarted
        .prepare_decision_archive(std::slice::from_ref(&key))
        .expect("prepare archive");
    let store = DecisionStore::default();
    for blob in &prepared.blobs {
        store.store_decision_archive(blob).expect("store blob");
    }
    let archived = restarted
        .commit_decision_archive(&prepared, &store)
        .expect("archive hot attempt");
    assert_eq!(
        archived
            .hot_history_commitment()
            .expect("commitment after hot release"),
        empty_root
    );
}

#[test]
fn decision_locator_primary_bucket_is_split_into_bounded_segments() {
    let mut selected_bucket = None;
    let mut selected = Vec::new();
    for ordinal in 1_u64..=250_000 {
        let key = DecisionHistoryKey::Attempt(DecisionRequestId::new(ordinal));
        let page_key = decision_history_page_key(&key).expect("hash decision key");
        match selected_bucket {
            None => {
                selected_bucket = Some(page_key.bucket);
                selected.push((key, page_key));
            }
            Some(bucket) if bucket == page_key.bucket => selected.push((key, page_key)),
            Some(_) => {}
        }
        let distinct_segments = selected
            .iter()
            .map(|(_, page_key)| page_key.segment)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if selected.len() >= 24 && distinct_segments >= 8 {
            break;
        }
    }
    assert!(
        selected.len() >= 24,
        "fixture should find one busy primary bucket"
    );

    let mut state = DecisionState::default();
    let mut keys = Vec::new();
    for (key, _) in &selected {
        let DecisionHistoryKey::Attempt(request_id) = key else {
            unreachable!("fixture creates attempt keys")
        };
        state
            .append_attempt(DecisionAttemptRecord {
                request_id: *request_id,
                request_commitment: format!("{:064x}", request_id.get()),
                at: SimTime::EPOCH,
                revision_before: request_id.get(),
                expected_revision: request_id.get(),
                outcome: DecisionAttemptOutcome::Rejected {
                    code: canwu_decision::DecisionAttemptErrorCode::InvalidDecision,
                    message: "segmented locator fixture".to_owned(),
                },
            })
            .expect("append segmented attempt");
        keys.push(key.clone());
    }
    let prepared = state
        .prepare_decision_archive(&keys)
        .expect("prepare segmented archive");
    let store = DecisionStore::default();
    for blob in &prepared.blobs {
        store.store_decision_archive(blob).expect("store blob");
    }
    let archived = state
        .commit_decision_archive(&prepared, &store)
        .expect("commit segmented archive");
    let page_keys = selected
        .iter()
        .map(|(_, page_key)| *page_key)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(page_keys.len() >= 8);
    for page_key in page_keys {
        let page = archived
            .decision_archive_bucket_page(page_key)
            .expect("encode locator page")
            .expect("locator page");
        assert!(page.receipts.len() <= MAX_DECISION_ARCHIVE_BUCKET_PAGE_ENTRIES);
        assert!(
            serde_json::to_vec(&page)
                .expect("encode locator page")
                .len()
                <= MAX_DECISION_ARCHIVE_BUCKET_PAGE_BYTES
        );
    }
}

#[test]
fn terminal_decision_history_moves_out_of_hot_state_after_verified_storage() {
    let request_id = DecisionRequestId::new(77);
    let attempt = DecisionAttemptRecord {
        request_id,
        request_commitment: "b".repeat(64),
        at: SimTime::EPOCH,
        revision_before: 3,
        expected_revision: 3,
        outcome: DecisionAttemptOutcome::Rejected {
            code: canwu_decision::DecisionAttemptErrorCode::InvalidDecision,
            message: "Rejected deterministically".to_owned(),
        },
    };
    let mut state = DecisionState::default();
    state
        .append_attempt(attempt.clone())
        .expect("append attempt");
    let key = DecisionHistoryKey::Attempt(request_id);
    let prepared = state
        .prepare_decision_archive(std::slice::from_ref(&key))
        .expect("prepare archive");
    let store = DecisionStore::default();
    for blob in &prepared.blobs {
        store.store_decision_archive(blob).expect("store blob");
    }
    let archived = state
        .commit_decision_archive(&prepared, &store)
        .expect("commit archive");
    assert!(archived.attempt(request_id).is_none());
    assert!(matches!(
        archived.decision_locator(&key),
        DecisionHistoryLocation::Archived { .. }
    ));
    assert_eq!(archived.decision_hot_state().attempt_count, 0);
    assert_eq!(
        archived
            .load_decision_history(&key, &store)
            .expect("load archive"),
        Some(DecisionArchiveRecord::Attempt { attempt })
    );
}

#[test]
fn archived_decision_history_is_owned_paginated_and_generation_bound() {
    let mut state = DecisionState::default();
    let mut keys = Vec::new();
    for ordinal in [77_u64, 78, 79] {
        let request_id = DecisionRequestId::new(ordinal);
        state
            .append_attempt(DecisionAttemptRecord {
                request_id,
                request_commitment: "c".repeat(64),
                at: SimTime::EPOCH,
                revision_before: 3,
                expected_revision: 3,
                outcome: DecisionAttemptOutcome::Rejected {
                    code: canwu_decision::DecisionAttemptErrorCode::InvalidDecision,
                    message: format!("rejected-{ordinal}"),
                },
            })
            .expect("append attempt");
        keys.push(DecisionHistoryKey::Attempt(request_id));
    }
    let prepared = state
        .prepare_decision_archive(&keys)
        .expect("prepare archive");
    let store = DecisionStore::default();
    for blob in &prepared.blobs {
        store.store_decision_archive(blob).expect("store blob");
    }
    let archived = state
        .commit_decision_archive(&prepared, &store)
        .expect("commit archive");
    let budget = DecisionHistoryQueryBudget {
        max_results: 2,
        max_provider_calls: 2,
        max_decoded_bytes: 1_000_000,
    };
    let first = archived
        .archived_decision_history_page(None, budget, &store)
        .expect("first history page");
    assert_eq!(first.records.len(), 2);
    assert_eq!(first.provider_calls, 2);
    let cursor = first.next_cursor.expect("continuation cursor");
    let second = archived
        .archived_decision_history_page(Some(&cursor), budget, &store)
        .expect("second history page");
    assert_eq!(second.records.len(), 1);
    assert!(second.next_cursor.is_none());

    let mut stale_cursor = cursor;
    stale_cursor.archive_root = "0".repeat(64);
    assert_eq!(
        archived
            .archived_decision_history_page(Some(&stale_cursor), budget, &store)
            .expect_err("stale cursor must fail")
            .code,
        DecisionErrorCode::DecisionHistoryUnavailable
    );
}

#[test]
fn external_request_exposes_options_but_not_authoritative_actions() {
    let policy = QueuedExternalPolicy::new("diplomacy-service", "1");
    let request = policy.external_request(&ticket(2));
    let serialized = serde_json::to_string(&request).expect("external decision request");

    assert_eq!(request.ticket_version, 2);
    assert_eq!(request.options[0].id, "send-aid");
    assert!(serialized.contains("public_hint"));
    assert!(!serialized.contains("private_command"));
    assert!(!serialized.contains("must-not-cross-policy-boundary"));
}

#[test]
fn asynchronous_policies_reject_stale_ticket_versions() {
    let current = ticket(2);

    let mut human = QueuedHumanPolicy::new("human-seat", "1");
    human
        .submit(
            current.id,
            HumanDecisionResponse {
                ticket_version: 1,
                option_id: "send-aid".to_owned(),
                operator_id: "operator-9".to_owned(),
            },
        )
        .expect("first human response should be accepted");
    assert_eq!(
        human
            .decide(&current)
            .expect_err("stale human response")
            .code,
        DecisionErrorCode::VersionConflict
    );

    let stale_external = ExternalDecisionResponse {
        ticket_version: 1,
        option_id: "send-aid".to_owned(),
        provider: "policy-service".to_owned(),
        request_id: "request-1".to_owned(),
        metadata: BTreeMap::new(),
    };
    let mut external = QueuedExternalPolicy::new("external-service", "1");
    external
        .submit(current.id, stale_external.clone())
        .expect("first external response should be accepted");
    assert_eq!(
        external
            .decide(&current)
            .expect_err("stale external response")
            .code,
        DecisionErrorCode::VersionConflict
    );

    let mut llm = QueuedLlmPolicy::new(
        "llm-adapter",
        "1",
        LlmModelIdentity {
            provider: "policy-service".to_owned(),
            model: "decision-model".to_owned(),
            prompt_contract: "select-existing-option.v1".to_owned(),
        },
    );
    llm.submit(current.id, stale_external)
        .expect("first LLM response should be accepted");
    assert_eq!(
        llm.decide(&current).expect_err("stale LLM response").code,
        DecisionErrorCode::VersionConflict
    );

    let mut current_llm = QueuedLlmPolicy::new(
        "llm-adapter",
        "1",
        LlmModelIdentity {
            provider: "policy-service".to_owned(),
            model: "decision-model".to_owned(),
            prompt_contract: "select-existing-option.v1".to_owned(),
        },
    );
    current_llm
        .submit(
            current.id,
            ExternalDecisionResponse {
                ticket_version: current.version,
                option_id: "send-aid".to_owned(),
                provider: "policy-service".to_owned(),
                request_id: "request-2".to_owned(),
                metadata: BTreeMap::new(),
            },
        )
        .expect("the first current LLM response should be accepted");
    let decision = current_llm.decide(&current).expect("current LLM response");
    let evidence = decision.external.expect("LLM trace evidence");
    assert_eq!(evidence.model.as_deref(), Some("decision-model"));
    assert_eq!(
        evidence.prompt_contract.as_deref(),
        Some("select-existing-option.v1")
    );
}

#[test]
fn asynchronous_policies_reject_duplicate_responses() {
    let current = ticket(2);
    let response = HumanDecisionResponse {
        ticket_version: current.version,
        option_id: "send-aid".to_owned(),
        operator_id: "operator-9".to_owned(),
    };
    let mut human = QueuedHumanPolicy::new("human-seat", "1");
    human
        .submit(current.id, response.clone())
        .expect("first response should be accepted");
    assert_eq!(
        human
            .submit(current.id, response)
            .expect_err("duplicate response should be rejected")
            .code,
        DecisionErrorCode::DuplicateResponse
    );
}

#[test]
fn asynchronous_policies_allow_a_newer_response_to_replace_a_stale_one() {
    let current = ticket(2);
    let mut human = QueuedHumanPolicy::new("human-seat", "1");
    human
        .submit(
            current.id,
            HumanDecisionResponse {
                ticket_version: 1,
                option_id: "send-aid".to_owned(),
                operator_id: "operator-old".to_owned(),
            },
        )
        .expect("stale response should be queued for later reconciliation");
    human
        .submit(
            current.id,
            HumanDecisionResponse {
                ticket_version: current.version,
                option_id: "send-aid".to_owned(),
                operator_id: "operator-current".to_owned(),
            },
        )
        .expect("newer response should replace the stale response");

    let decision = human
        .decide(&current)
        .expect("current response should be authoritative");
    assert_eq!(
        decision.external.and_then(|evidence| evidence.request_id),
        Some("operator-current".to_owned())
    );
}
