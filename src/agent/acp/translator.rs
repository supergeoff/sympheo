/// Translate an ACP [`SessionUpdate`] notification into a Sympheo [`AgentEvent`].
///
/// The translation is deterministic and pure (no I/O). All unrecognised or
/// future ACP update types map to `AgentEvent::Other` via the mandatory
/// catch-all arm — required because `SessionUpdate` is `#[non_exhaustive]`.
use agent_client_protocol::schema::{
    ContentBlock, Plan, SessionUpdate, ToolCall, ToolCallContent, ToolCallUpdate,
};

use crate::agent::parser::{
    AgentEvent, Location, PlanStep, PlanStepStatus, ToolCallContent as SympheoToolCallContent,
    ToolKind, ToolStatus,
};

/// Convert an ACP `SessionUpdate` to a Sympheo `AgentEvent`.
pub fn translate(update: SessionUpdate) -> AgentEvent {
    match update {
        // agent_message_chunk → Text (spec: §6.3, canonical ACP name)
        SessionUpdate::AgentMessageChunk(chunk) => {
            let text = extract_text(&chunk.content);
            AgentEvent::Text {
                timestamp: 0,
                session_id: String::new(),
                part: crate::agent::parser::TextPart {
                    id: String::new(),
                    message_id: String::new(),
                    session_id: String::new(),
                    part_type: "text".to_string(),
                    text,
                    time: None,
                },
            }
        }
        // agent_thought_chunk → Thinking (spec: canonical ACP name, NOT thinking_chunk)
        SessionUpdate::AgentThoughtChunk(chunk) => {
            let delta = extract_text(&chunk.content);
            AgentEvent::Thinking { delta }
        }
        // ToolCall initiation
        SessionUpdate::ToolCall(tc) => translate_tool_call(tc),
        // ToolCallUpdate (incremental or final)
        SessionUpdate::ToolCallUpdate(tcu) => translate_tool_call_update(tcu),
        // Plan
        SessionUpdate::Plan(plan) => translate_plan(plan),
        // All other variants (AvailableCommandsUpdate, CurrentModeUpdate, ConfigOptionUpdate,
        // SessionInfoUpdate, UserMessageChunk, UsageUpdate, …) → Other.
        // The `#[non_exhaustive]` attribute on SessionUpdate makes this arm mandatory.
        _ => AgentEvent::Other,
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

fn map_plan_status(
    status: agent_client_protocol::schema::PlanEntryStatus,
) -> PlanStepStatus {
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
        ToolCallContent::Diff(_) => SympheoToolCallContent {
            content_type: "diff".to_string(),
            text: None,
        },
        ToolCallContent::Terminal(_) => SympheoToolCallContent {
            content_type: "terminal".to_string(),
            text: None,
        },
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

fn translate_tool_call_update(tcu: ToolCallUpdate) -> AgentEvent {
    let fields = tcu.fields;
    AgentEvent::ToolCallUpdate {
        id: tcu.tool_call_id.0.to_string(),
        status: fields.status.map(map_tool_status),
        content: fields
            .content
            .unwrap_or_default()
            .into_iter()
            .map(map_content)
            .collect(),
        raw_output: fields.raw_output,
    }
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
        ContentChunk, PlanEntry, PlanEntryPriority, PlanEntryStatus,
        ToolCallId, ToolCallUpdate as AcpToolCallUpdate, ToolCallUpdateFields,
    };

    fn text_chunk(s: &str) -> agent_client_protocol::schema::ContentChunk {
        ContentChunk::new(ContentBlock::from(s))
    }

    // --- AgentMessageChunk → Text ---

    #[test]
    fn agent_message_chunk_to_text() {
        let event = translate(SessionUpdate::AgentMessageChunk(text_chunk("hello")));
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
        let event = translate(SessionUpdate::AgentMessageChunk(chunk));
        match event {
            AgentEvent::Text { part, .. } => assert_eq!(part.text, ""),
            _ => panic!("expected Text"),
        }
    }

    // --- AgentThoughtChunk → Thinking ---

    #[test]
    fn agent_thought_chunk_to_thinking() {
        let event = translate(SessionUpdate::AgentThoughtChunk(text_chunk("pondering")));
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
        let event = translate(SessionUpdate::ToolCall(tc));
        match event {
            AgentEvent::ToolCall { id, title, kind, raw_input, locations } => {
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
        let event = translate(SessionUpdate::ToolCall(tc));
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
        let event = translate(SessionUpdate::ToolCallUpdate(tcu));
        match event {
            AgentEvent::ToolCallUpdate { id, status, content, raw_output } => {
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
        let event = translate(SessionUpdate::ToolCallUpdate(tcu));
        match event {
            AgentEvent::ToolCallUpdate { id, status, content, raw_output } => {
                assert_eq!(id, "tc-4");
                assert!(status.is_none());
                assert!(content.is_empty());
                assert!(raw_output.is_none());
            }
            _ => panic!("expected ToolCallUpdate"),
        }
    }

    // --- Plan ---

    #[test]
    fn plan_translation() {
        let plan = Plan::new(vec![
            PlanEntry::new("Step 1", PlanEntryPriority::High, PlanEntryStatus::Pending),
            PlanEntry::new("Step 2", PlanEntryPriority::Medium, PlanEntryStatus::InProgress),
            PlanEntry::new("Step 3", PlanEntryPriority::Low, PlanEntryStatus::Completed),
        ]);
        let event = translate(SessionUpdate::Plan(plan));
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
        let event = translate(SessionUpdate::Plan(Plan::new(vec![])));
        match event {
            AgentEvent::Plan { steps } => assert!(steps.is_empty()),
            _ => panic!("expected Plan"),
        }
    }

    // --- Catch-all → Other ---

    #[test]
    fn available_commands_update_maps_to_other() {
        use agent_client_protocol::schema::AvailableCommandsUpdate;
        let event = translate(SessionUpdate::AvailableCommandsUpdate(
            AvailableCommandsUpdate::new(vec![]),
        ));
        assert!(matches!(event, AgentEvent::Other));
    }

    #[test]
    fn current_mode_update_maps_to_other() {
        use agent_client_protocol::schema::CurrentModeUpdate;
        let event = translate(SessionUpdate::CurrentModeUpdate(
            CurrentModeUpdate::new("build"),
        ));
        assert!(matches!(event, AgentEvent::Other));
    }

    #[test]
    fn user_message_chunk_maps_to_other() {
        let event = translate(SessionUpdate::UserMessageChunk(text_chunk("user msg")));
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
            assert_eq!(map_tool_kind(acp.clone()), expected, "kind={acp:?}");
        }
    }

    // --- Plan status mapping ---

    #[test]
    fn all_plan_statuses_map_correctly() {
        use agent_client_protocol::schema::PlanEntryStatus as AcpStatus;
        assert_eq!(map_plan_status(AcpStatus::Pending), PlanStepStatus::Pending);
        assert_eq!(map_plan_status(AcpStatus::InProgress), PlanStepStatus::InProgress);
        assert_eq!(map_plan_status(AcpStatus::Completed), PlanStepStatus::Completed);
    }
}
