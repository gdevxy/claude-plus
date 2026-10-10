mod record;
pub mod text;
mod usage;

pub use record::*;
pub use usage::TokenCounts;

#[cfg(test)]
mod tests {
    use super::*;

    const ASSISTANT: &str = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-10-01T10:00:00.000Z","isSidechain":false,"cwd":"/w","requestId":"req_1","uuid":"u1","attributionSkill":"superpowers:brainstorming","effort":"high","message":{"id":"msg_1","model":"claude-opus-5-5","content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"/w/a.rs"}}],"usage":{"input_tokens":10,"output_tokens":1000,"cache_read_input_tokens":20000,"cache_creation_input_tokens":5000,"cache_creation":{"ephemeral_1h_input_tokens":5000,"ephemeral_5m_input_tokens":0}}}}"#;

    #[test]
    fn parses_assistant_record() {
        let parsed = parse_line(ASSISTANT).unwrap();
        let Record::Assistant(a) = parsed.record else {
            panic!("expected assistant")
        };
        assert_eq!(a.dedup_key.as_deref(), Some("req_1"));
        assert_eq!(a.model, "claude-opus-5-5");
        assert_eq!(a.usage.cache_write_1h, 5000);
        assert_eq!(a.common.session_id.as_deref(), Some("s1"));
        assert_eq!(
            a.common.timestamp.unwrap().to_rfc3339(),
            "2026-10-01T10:00:00+00:00"
        );
        assert_eq!(a.skill.as_deref(), Some("superpowers:brainstorming"));
        assert_eq!(a.effort.as_deref(), Some("high"));
        assert_eq!(
            a.tool_uses,
            vec![ToolUse {
                id: "toolu_1".into(),
                name: "Read".into(),
                input_chars: 7
            }]
        );
        assert!(a.is_billable());
    }

    #[test]
    fn dedup_key_falls_back_to_message_id_then_uuid() {
        let no_request = ASSISTANT.replace(r#""requestId":"req_1","#, "");
        let Record::Assistant(a) = parse_line(&no_request).unwrap().record else {
            panic!()
        };
        assert_eq!(a.dedup_key.as_deref(), Some("msg_1"));
        let no_ids = no_request.replace(r#""id":"msg_1","#, "");
        let Record::Assistant(a) = parse_line(&no_ids).unwrap().record else {
            panic!()
        };
        assert_eq!(a.dedup_key.as_deref(), Some("u1"));
    }

    #[test]
    fn synthetic_assistant_is_not_billable() {
        let line = ASSISTANT.replace("claude-opus-5-5", "<synthetic>");
        let Record::Assistant(a) = parse_line(&line).unwrap().record else {
            panic!()
        };
        assert!(!a.is_billable());
    }

    #[test]
    fn parses_user_tool_results_and_prompts() {
        let tool = r#"{"type":"user","sessionId":"s1","timestamp":"2026-10-01T10:00:01Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"hello world","is_error":false}]}}"#;
        let Record::User(u) = parse_line(tool).unwrap().record else {
            panic!()
        };
        assert!(!u.is_prompt);
        assert_eq!(
            u.tool_results,
            vec![ToolResult {
                tool_use_id: "toolu_1".into(),
                chars: 11,
                images: 0,
                is_error: false
            }]
        );

        let prompt =
            r#"{"type":"user","sessionId":"s1","message":{"role":"user","content":"fix the bug"}}"#;
        let Record::User(u) = parse_line(prompt).unwrap().record else {
            panic!()
        };
        assert!(u.is_prompt);
        assert_eq!(u.text_chars, 11);

        let meta = r#"{"type":"user","isMeta":true,"sessionId":"s1","message":{"role":"user","content":"caveat"}}"#;
        let Record::User(u) = parse_line(meta).unwrap().record else {
            panic!()
        };
        assert!(!u.is_prompt);
    }

    #[test]
    fn system_and_tooling_messages_are_not_prompts() {
        let user = |content: &str, extra: &str| {
            let line =
                serde_json::json!({"type":"user","message":{"role":"user","content":content}});
            let mut line = line;
            if !extra.is_empty() {
                line[extra] = true.into();
            }
            let Record::User(u) = parse_value(line).unwrap().record else {
                panic!()
            };
            u.is_prompt
        };
        for prefix in [
            "<task-notification>x",
            "<command-name>x",
            "<local-command-stdout>x",
            "<system-reminder>x",
            "[Request interrupted by user]",
        ] {
            assert!(!user(prefix, ""), "{prefix}");
            assert!(!user(&format!("  \n{prefix}"), ""), "padded {prefix}");
        }
        assert!(!user("hello", "isCompactSummary"));
        assert!(!user("hello", "isVisibleInTranscriptOnly"));
        assert!(!user("hello", "isMeta"));
        assert!(user("fix the bug", ""));
    }

    #[test]
    fn tool_result_images_are_counted_not_measured() {
        let line = serde_json::json!({"type":"user","message":{"role":"user","content":[{
        "type":"tool_result","tool_use_id":"t","content":[
            {"type":"text","text":"abc"},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":"A".repeat(10000)}}
        ]}]}});
        let Record::User(u) = parse_value(line).unwrap().record else {
            panic!()
        };
        assert_eq!((u.tool_results[0].chars, u.tool_results[0].images), (3, 1));
    }

    #[test]
    fn overflowing_numbers_saturate() {
        let attachment =
            r#"{"type":"attachment","attachment":[{"$chars":18446744073709551615},{"$chars":1}]}"#;
        let Record::Attachment(a) = parse_line(attachment).unwrap().record else {
            panic!()
        };
        assert_eq!(a.chars, u64::MAX);
        let assistant = r#"{"type":"assistant","message":{"model":"m","usage":{"input_tokens":18446744073709551615,"cache_read_input_tokens":18446744073709551615}}}"#;
        let Record::Assistant(a) = parse_line(assistant).unwrap().record else {
            panic!()
        };
        assert_eq!(a.usage.prompt_total(), u64::MAX);
    }

    #[test]
    fn parses_attachments() {
        let line = r#"{"type":"attachment","sessionId":"s1","attachment":{"type":"skill_listing","content":"- a: b","names":["a"],"skillCount":1}}"#;
        let Record::Attachment(a) = parse_line(line).unwrap().record else {
            panic!()
        };
        assert_eq!(a.kind, "skill_listing");
        assert_eq!(a.names, vec!["a".to_string()]);
        assert_eq!(a.chars, "skill_listing".len() as u64 + 6 + 1);

        let instructions = r#"{"type":"attachment","sessionId":"s1","attachment":{"type":"instructions","files":[{"path":"/h/CLAUDE.md","type":"User","content":"rules"}]}}"#;
        let Record::Attachment(a) = parse_line(instructions).unwrap().record else {
            panic!()
        };
        assert_eq!(a.names, vec!["/h/CLAUDE.md".to_string()]);
    }

    #[test]
    fn parses_compaction_cost_state_and_titles() {
        let compact = r#"{"type":"system","subtype":"compact_boundary","sessionId":"s1","compactMetadata":{"trigger":"auto","preTokens":367175,"postTokens":19259}}"#;
        let Record::CompactBoundary(c) = parse_line(compact).unwrap().record else {
            panic!()
        };
        assert_eq!(
            (c.trigger.as_deref(), c.pre_tokens, c.post_tokens),
            (Some("auto"), Some(367175), Some(19259))
        );

        let cost = r#"{"type":"cost-state","sessionId":"s1","totalCostUSD":0.77,"modelUsage":{"claude-sonnet-5":{"inputTokens":3,"outputTokens":40,"thinkingTokens":9,"cacheReadInputTokens":500,"cacheCreationInputTokens":60,"costUSD":0.77},"claude-haiku-4-5":{"costUSD":0.01},"broken":7}}"#;
        let Record::CostState(c) = parse_line(cost).unwrap().record else {
            panic!()
        };
        assert_eq!(c.total_cost_usd, 0.77);
        assert_eq!(
            c.model_usage.get("claude-sonnet-5"),
            Some(&ModelUsage {
                input: 3,
                output: 40,
                cache_read: 500,
                cache_write: 60,
                cost_usd: 0.77
            })
        );
        assert_eq!(c.model_usage["claude-haiku-4-5"].output, 0);
        assert!(!c.model_usage.contains_key("broken"));

        let title = r#"{"type":"custom-title","sessionId":"s1","customTitle":"Refactor"}"#;
        let Record::Title(t) = parse_line(title).unwrap().record else {
            panic!()
        };
        assert_eq!((t.title.as_str(), t.is_custom), ("Refactor", true));
    }

    #[test]
    fn unknown_kinds_are_other() {
        let parsed = parse_line(r#"{"type":"queue-operation","operation":"enqueue"}"#).unwrap();
        assert_eq!(parsed.record.kind(), "queue-operation");
        assert!(matches!(parsed.record, Record::Other { .. }));
    }

    #[test]
    fn garbage_lines_error_without_panic() {
        for line in [
            "",
            "not json",
            "[1,2]",
            "42",
            "{\"type\":",
            "{\"type\":17}",
            r#"{"type":"attachment","attachment":[{"$chars":18446744073709551615},{"$chars":1}]}"#,
            r#"{"type":"assistant","message":{"usage":{"input_tokens":18446744073709551615,"cache_read_input_tokens":18446744073709551615}}}"#,
            "{\"message\":{\"usage\":\"x\"},\"type\":\"assistant\"}",
        ] {
            let _ = parse_line(line);
        }
        assert!(matches!(parse_line("[1]"), Err(ParseError::NotAnObject)));
        assert!(matches!(
            parse_line("nope"),
            Err(ParseError::InvalidJson(_))
        ));
    }
}
