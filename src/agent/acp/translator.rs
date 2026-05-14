/// Translate an ACP [`SessionUpdate`] notification into one or more Sympheo [`AgentEvent`]s.
///
/// The translation is deterministic and pure (no I/O).  Most updates produce a single event;
/// a `ToolCallUpdate` that contains diff content produces an `AgentEvent::Diff` followed by
/// the `AgentEvent::ToolCallUpdate`.  All unrecognised or future ACP update types map to
/// `AgentEvent::Other` via the mandatory catch-all arm — required because `SessionUpdate`
/// is `#[non_exhaustive]`.
use agent_client_protocol::schema::{
    ContentBlock, Plan, SessionUpdate, ToolCall, ToolCallContent, ToolCallUpdate,
};

use crate::agent::parser::{
    AgentEvent, Location, PlanStep, PlanStepStatus, ToolCallContent as SympheoToolCallContent,
    ToolKind, ToolStatus,
};

/// Convert an ACP `SessionUpdate` to zero or more Sympheo `AgentEvent`s.
///
/// * `session_id` — the ACP session identifier from the outer `SessionNotification`.
/// * `now` — current Unix timestamp in milliseconds, used to populate `timestamp` fields.
pub fn translate(session_id: &str, update: SessionUpdate, now: i64) -> Vec<AgentEvent> {
    match update {
        // agent_message_chunk → Text (spec: §6.3, canonical ACP name)
        SessionUpdate::AgentMessageChunk(chunk) => {
            let text = extract_text(&chunk.content);
            vec![AgentEvent::Text {
                timestamp: now,
                session_id: session_id.to_string(),
                part: crate::agent::parser::TextPart {
                    id: String::new(),
                    message_id: String::new(),
                    session_id: session_id.to_string(),
                    part_type: "text".to_string(),
                    text,
                    time: None,
                },
            }]
        }
        // agent_thought_chunk → Thinking (spec: canonical ACP name, NOT thinking_chunk)
        SessionUpdate::AgentThoughtChunk(chunk) => {
            let delta = extract_text(&chunk.content);
            vec![AgentEvent::Thinking { delta }]
        }
        // ToolCall initiation
        SessionUpdate::ToolCall(tc) => vec![translate_tool_call(tc)],
        // ToolCallUpdate — may emit a Diff event before the ToolCallUpdate event
        SessionUpdate::ToolCallUpdate(tcu) => translate_tool_call_update(tcu),
        // Plan
        SessionUpdate::Plan(plan) => vec![translate_plan(plan)],
        // All other variants (AvailableCommandsUpdate, CurrentModeUpdate, ConfigOptionUpdate,
        // SessionInfoUpdate, UserMessageChunk, UsageUpdate, …) → Other.
        // The `#[non_exhaustive]` attribute on SessionUpdate makes this arm mandatory.
        _ => vec![AgentEvent::Other],
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn extract_text(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

fn map_tool_kind(kind: agent_client_protocol::schema::ToolKind) -> ToolKind {
    use agent_client_protocol::schema::ToolKind as AcpKind;
    match kind {
        AcpKind::Read => ToolKind::Read,
        AcpKind::Edit => ToolKind::Edit,
        AcpKind::Delete => ToolKind::Delete,
        AcpKind::Move => ToolKind::Move,
        AcpKind::Search => ToolKind::Search,
        AcpKind::Execute => ToolKind::Execute,
        AcpKind::Think => ToolKind::Think,
        AcpKind::Fetch => ToolKind::Fetch,
        // SwitchMode and Other (+ future variants) map to Other
        _ => ToolKind::Other,
    }
}

fn map_tool_status(status: agent_client_protocol::schema::ToolCallStatus) -> ToolStatus {
    use agent_client_protocol::schema::ToolCallStatus as AcpStatus;
    match status {
        AcpStatus::Pending => ToolStatus::Pending,
        AcpStatus::InProgress => ToolStatus::InProgress,
        AcpStatus::Completed => ToolStatus::Completed,
        AcpStatus::Failed => ToolStatus::Failed,
        _ => ToolStatus::Failed,
    }
}

fn map_plan_status(status: agent_client_protocol::schema::PlanEntryStatus) -> PlanStepStatus {
    use agent_client_protocol::schema::PlanEntryStatus as AcpStatus;
    match status {
        AcpStatus::Pending => PlanStepStatus::Pending,
        AcpStatus::InProgress => PlanStepStatus::InProgress,
        AcpStatus::Completed => PlanStepStatus::Completed,
        _ => PlanStepStatus::Pending,
    }
}

fn map_location(loc: agent_client_protocol::schema::ToolCallLocation) -> Location {
    Location {
        path: loc.path.to_string_lossy().into_owned(),
        start_line: loc.line,
        end_line: None,
    }
}

fn map_content(content: ToolCallContent) -> SympheoToolCallContent {
    match content {
        ToolCallContent::Content(c) => {
            let text = match &c.content {
                ContentBlock::Text(t) => Some(t.text.clone()),
                _ => None,
            };
            SympheoToolCallContent {
                content_type: "text".to_string(),
                text,
            }
        }
        ToolCallContent::Terminal(_) => SympheoToolCallContent {
            content_type: "terminal".to_string(),
            text: None,
        },
        // Diff is handled before map_content is called (in translate_tool_call_update).
        // This catch-all covers Diff if it ever reaches here and all future #[non_exhaustive]
        // variants — both require an exhaustive wildcard arm for external crates.
        _ => SympheoToolCallContent {
            content_type: "other".to_string(),
            text: None,
        },
    }
}

fn translate_tool_call(tc: ToolCall) -> AgentEvent {
    AgentEvent::ToolCall {
        id: tc.tool_call_id.0.to_string(),
        title: tc.title,
        kind: map_tool_kind(tc.kind),
        raw_input: tc.raw_input.unwrap_or(serde_json::Value::Null),
        locations: tc.locations.into_iter().map(map_location).collect(),
    }
}

/// Translate a `ToolCallUpdate`, emitting an `AgentEvent::Diff` for each diff content item
/// followed by the `AgentEvent::ToolCallUpdate`.  Most updates produce exactly one event.
fn translate_tool_call_update(tcu: ToolCallUpdate) -> Vec<AgentEvent> {
    let id_str = tcu.tool_call_id.0.to_string();
    let fields = tcu.fields;
    let content_list = fields.content.unwrap_or_default();

    let mut diff_events: Vec<AgentEvent> = Vec::new();
    let mut mapped_content: Vec<SympheoToolCallContent> = Vec::new();

    for item in content_list {
        match item {
            ToolCallContent::Diff(diff) => {
                diff_events.push(AgentEvent::Diff {
                    tool_call_id: id_str.clone(),
                    path: diff.path,
                    old_text: diff.old_text,
                    new_text: diff.new_text,
                });
                mapped_content.push(SympheoToolCallContent {
                    content_type: "diff".to_string(),
                    text: None,
                });
            }
            other => mapped_content.push(map_content(other)),
        }
    }

    let update_event = AgentEvent::ToolCallUpdate {
        id: id_str,
        status: fields.status.map(map_tool_status),
        content: mapped_content,
        raw_output: fields.raw_output,
    };

    let mut events = diff_events;
    events.push(update_event);
    events
}

fn translate_plan(plan: Plan) -> AgentEvent {
    AgentEvent::Plan {
        steps: plan
            .entries
            .into_iter()
            .map(|e| PlanStep {
                title: e.content,
                status: map_plan_status(e.status),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::{
        ContentChunk, Diff as AcpDiff, PlanEntry, PlanEntryPriority, PlanEntryStatus, ToolCallId,
        ToolCallUpdate as AcpToolCallUpdate, ToolCallUpdateFields,
    };

    /// Convenience wrapper: translate a single update and assert exactly one event is returned.
    fn translate_one(update: SessionUpdate) -> AgentEvent {
        let mut v = translate("", update, 0);
        assert_eq!(v.len(), 1, "expected exactly one event");
        v.remove(0)
    }

    fn text_chunk(s: &str) -> agent_client_protocol::schema::ContentChunk {
        ContentChunk::new(ContentBlock::from(s))
    }

    // --- AgentMessageChunk → Text ---

    #[test]
    fn agent_message_chunk_to_text() {
        let event = translate_one(SessionUpdate::AgentMessageChunk(text_chunk("hello")));
        match event {
            AgentEvent::Text { part, .. } => assert_eq!(part.text, "hello"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn agent_message_chunk_non_text_block_gives_empty_text() {
        let chunk = ContentChunk::new(ContentBlock::ResourceLink(
            agent_client_protocol::schema::ResourceLink::new("foo", "file:///foo"),
        ));
        let event = translate_one(SessionUpdate::AgentMessageChunk(chunk));
        match event {
            AgentEvent::Text { part, .. } => assert_eq!(part.text, ""),
            _ => panic!("expected Text"),
        }
    }

    // --- session_id and timestamp propagation ---

    #[test]
    fn session_id_and_timestamp_propagate_to_text() {
        let events = translate(
            "my-session",
            SessionUpdate::AgentMessageChunk(text_chunk("hi")),
            12345,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::Text {
                timestamp,
                session_id,
                part,
            } => {
                assert_eq!(*timestamp, 12345);
                assert_eq!(session_id, "my-session");
                assert_eq!(part.session_id, "my-session");
                assert_eq!(part.text, "hi");
            }
            _ => panic!("expected Text"),
        }
    }

    // --- AgentThoughtChunk → Thinking ---

    #[test]
    fn agent_thought_chunk_to_thinking() {
        let event = translate_one(SessionUpdate::AgentThoughtChunk(text_chunk("pondering")));
        match event {
            AgentEvent::Thinking { delta } => assert_eq!(delta, "pondering"),
            _ => panic!("expected Thinking"),
        }
    }

    // --- ToolCall ---

    #[test]
    fn tool_call_translation() {
        let tc = ToolCall::new(ToolCallId::new("tc-1"), "Read file")
            .kind(agent_client_protocol::schema::ToolKind::Read)
            .raw_input(serde_json::json!({"path": "/tmp/foo"}));
        let event = translate_one(SessionUpdate::ToolCall(tc));
        match event {
            AgentEvent::ToolCall {
                id,
                title,
                kind,
                raw_input,
                locations,
            } => {
                assert_eq!(id, "tc-1");
                assert_eq!(title, "Read file");
                assert_eq!(kind, ToolKind::Read);
                assert_eq!(raw_input["path"], "/tmp/foo");
                assert!(locations.is_empty());
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn tool_call_switch_mode_maps_to_other_kind() {
        let tc = ToolCall::new(ToolCallId::new("tc-2"), "switch")
            .kind(agent_client_protocol::schema::ToolKind::SwitchMode);
        let event = translate_one(SessionUpdate::ToolCall(tc));
        match event {
            AgentEvent::ToolCall { kind, .. } => assert_eq!(kind, ToolKind::Other),
            _ => panic!("expected ToolCall"),
        }
    }

    // --- ToolCallUpdate ---

    #[test]
    fn tool_call_update_with_status() {
        // ToolCallUpdateFields is #[non_exhaustive]; use Default then set the field.
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(agent_client_protocol::schema::ToolCallStatus::Completed);
        let tcu = AcpToolCallUpdate::new(ToolCallId::new("tc-3"), fields);
        let event = translate_one(SessionUpdate::ToolCallUpdate(tcu));
        match event {
            AgentEvent::ToolCallUpdate {
                id,
                status,
                content,
                raw_output,
            } => {
                assert_eq!(id, "tc-3");
                assert_eq!(status, Some(ToolStatus::Completed));
                assert!(content.is_empty());
                assert!(raw_output.is_none());
            }
            _ => panic!("expected ToolCallUpdate"),
        }
    }

    #[test]
    fn tool_call_update_partial_id_only() {
        let fields = ToolCallUpdateFields::default();
        let tcu = AcpToolCallUpdate::new(ToolCallId::new("tc-4"), fields);
        let event = translate_one(SessionUpdate::ToolCallUpdate(tcu));
        match event {
            AgentEvent::ToolCallUpdate {
                id,
                status,
                content,
                raw_output,
            } => {
                assert_eq!(id, "tc-4");
                assert!(status.is_none());
                assert!(content.is_empty());
                assert!(raw_output.is_none());
            }
            _ => panic!("expected ToolCallUpdate"),
        }
    }

    // --- Diff — emitted as a separate event before the ToolCallUpdate ---

    #[test]
    fn tool_call_update_with_diff_emits_diff_and_update_events() {
        let diff = AcpDiff::new("/path/file.rs", "new content").old_text("old content");
        let mut fields = ToolCallUpdateFields::default();
        fields.content = Some(vec![ToolCallContent::Diff(diff)]);
        let tcu = AcpToolCallUpdate::new(ToolCallId::new("tc-diff"), fields);

        let events = translate("", SessionUpdate::ToolCallUpdate(tcu), 0);
        assert_eq!(
            events.len(),
            2,
            "expected AgentEvent::Diff + AgentEvent::ToolCallUpdate"
        );

        match &events[0] {
            AgentEvent::Diff {
                tool_call_id,
                path,
                old_text,
                new_text,
            } => {
                assert_eq!(tool_call_id, "tc-diff");
                assert_eq!(path.to_str().unwrap(), "/path/file.rs");
                assert_eq!(old_text.as_deref(), Some("old content"));
                assert_eq!(new_text, "new content");
            }
            _ => panic!("expected Diff as first event, got {:?}", events[0]),
        }

        match &events[1] {
            AgentEvent::ToolCallUpdate { id, content, .. } => {
                assert_eq!(id, "tc-diff");
                assert_eq!(content.len(), 1);
                assert_eq!(content[0].content_type, "diff");
            }
            _ => panic!("expected ToolCallUpdate as second event"),
        }
    }

    #[test]
    fn tool_call_update_diff_without_old_text() {
        let diff = AcpDiff::new("/new_file.rs", "fn main() {}");
        let mut fields = ToolCallUpdateFields::default();
        fields.content = Some(vec![ToolCallContent::Diff(diff)]);
        let tcu = AcpToolCallUpdate::new(ToolCallId::new("tc-new"), fields);

        let events = translate("", SessionUpdate::ToolCallUpdate(tcu), 0);
        assert_eq!(events.len(), 2);
        match &events[0] {
            AgentEvent::Diff {
                old_text, new_text, ..
            } => {
                assert!(old_text.is_none());
                assert_eq!(new_text, "fn main() {}");
            }
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn tool_call_update_no_diff_produces_single_event() {
        let fields = ToolCallUpdateFields::default();
        let tcu = AcpToolCallUpdate::new(ToolCallId::new("tc-nodiff"), fields);
        let events = translate("", SessionUpdate::ToolCallUpdate(tcu), 0);
        assert_eq!(
            events.len(),
            1,
            "no diff content → single ToolCallUpdate event"
        );
        assert!(matches!(events[0], AgentEvent::ToolCallUpdate { .. }));
    }

    // --- Plan ---

    #[test]
    fn plan_translation() {
        let plan = Plan::new(vec![
            PlanEntry::new("Step 1", PlanEntryPriority::High, PlanEntryStatus::Pending),
            PlanEntry::new(
                "Step 2",
                PlanEntryPriority::Medium,
                PlanEntryStatus::InProgress,
            ),
            PlanEntry::new("Step 3", PlanEntryPriority::Low, PlanEntryStatus::Completed),
        ]);
        let event = translate_one(SessionUpdate::Plan(plan));
        match event {
            AgentEvent::Plan { steps } => {
                assert_eq!(steps.len(), 3);
                assert_eq!(steps[0].title, "Step 1");
                assert_eq!(steps[0].status, PlanStepStatus::Pending);
                assert_eq!(steps[1].status, PlanStepStatus::InProgress);
                assert_eq!(steps[2].status, PlanStepStatus::Completed);
            }
            _ => panic!("expected Plan"),
        }
    }

    #[test]
    fn empty_plan() {
        let event = translate_one(SessionUpdate::Plan(Plan::new(vec![])));
        match event {
            AgentEvent::Plan { steps } => assert!(steps.is_empty()),
            _ => panic!("expected Plan"),
        }
    }

    // --- Catch-all → Other ---

    #[test]
    fn available_commands_update_maps_to_other() {
        use agent_client_protocol::schema::AvailableCommandsUpdate;
        let event = translate_one(SessionUpdate::AvailableCommandsUpdate(
            AvailableCommandsUpdate::new(vec![]),
        ));
        assert!(matches!(event, AgentEvent::Other));
    }

    #[test]
    fn current_mode_update_maps_to_other() {
        use agent_client_protocol::schema::CurrentModeUpdate;
        let event = translate_one(SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(
            "build",
        )));
        assert!(matches!(event, AgentEvent::Other));
    }

    #[test]
    fn user_message_chunk_maps_to_other() {
        let event = translate_one(SessionUpdate::UserMessageChunk(text_chunk("user msg")));
        assert!(matches!(event, AgentEvent::Other));
    }

    // --- Tool kind mapping ---

    #[test]
    fn all_tool_kinds_map_correctly() {
        use agent_client_protocol::schema::ToolKind as AcpKind;
        let cases = [
            (AcpKind::Read, ToolKind::Read),
            (AcpKind::Edit, ToolKind::Edit),
            (AcpKind::Delete, ToolKind::Delete),
            (AcpKind::Move, ToolKind::Move),
            (AcpKind::Search, ToolKind::Search),
            (AcpKind::Execute, ToolKind::Execute),
            (AcpKind::Think, ToolKind::Think),
            (AcpKind::Fetch, ToolKind::Fetch),
            (AcpKind::SwitchMode, ToolKind::Other),
            (AcpKind::Other, ToolKind::Other),
        ];
        for (acp, expected) in cases {
            // AcpKind is Copy — no .clone() needed
            assert_eq!(map_tool_kind(acp), expected, "kind={acp:?}");
        }
    }

    // --- Plan status mapping ---

    #[test]
    fn all_plan_statuses_map_correctly() {
        use agent_client_protocol::schema::PlanEntryStatus as AcpStatus;
        assert_eq!(map_plan_status(AcpStatus::Pending), PlanStepStatus::Pending);
        assert_eq!(
            map_plan_status(AcpStatus::InProgress),
            PlanStepStatus::InProgress
        );
        assert_eq!(
            map_plan_status(AcpStatus::Completed),
            PlanStepStatus::Completed
        );
    }
}
