//! 本地代理的分段计时。只记录固定阶段、运行编号与字节数，不记录正文、地址或凭据。
use std::collections::HashSet;
use std::time::Instant;

mod response_shape;

pub(crate) struct RequestTiming {
    scope_id: String,
    request_id: String,
    route: &'static str,
    purpose: &'static str,
    started: Instant,
    last: Instant,
    bytes: usize,
    first_chunk: bool,
    sse: Option<SseMilestones>,
}

impl RequestTiming {
    pub(crate) fn new(scope_id: &str, route: &'static str, purpose: &'static str) -> Self {
        let now = Instant::now();
        Self {
            scope_id: scope_id.to_owned(),
            request_id: uuid::Uuid::new_v4().simple().to_string(),
            route,
            purpose,
            started: now,
            last: now,
            bytes: 0,
            first_chunk: false,
            sse: (route == "responses").then(SseMilestones::default),
        }
    }

    pub(crate) fn mark(&mut self, stage: &str) {
        let now = Instant::now();
        eprintln!(
            "ai_send_timing command=model_http scope_id={} request_id={} route={} purpose={} stage={stage} stage_ms={} total_ms={} received_bytes={}",
            self.scope_id,
            self.request_id,
            self.route,
            self.purpose,
            now.duration_since(self.last).as_millis(),
            now.duration_since(self.started).as_millis(),
            self.bytes,
        );
        self.last = now;
    }

    pub(crate) fn chunk(&mut self, chunk: &[u8]) {
        self.bytes += chunk.len();
        if !self.first_chunk {
            self.first_chunk = true;
            self.mark("first_chunk");
        }
        let stages = self
            .sse
            .as_mut()
            .map(|sse| sse.observe(chunk))
            .unwrap_or_default();
        for stage in stages {
            self.mark(stage);
        }
    }
}

#[derive(Default)]
struct SseMilestones {
    line: Vec<u8>,
    oversized: bool,
    seen: HashSet<&'static str>,
}

impl SseMilestones {
    // 仅旁观数据流；跨 chunk 拼行，上限防止大工具参数/图片扩大诊断内存。原数据原样转发。
    fn observe(&mut self, chunk: &[u8]) -> Vec<&'static str> {
        let mut stages = Vec::new();
        for &byte in chunk {
            if byte != b'\n' {
                if self.line.len() < 65_536 && !self.oversized {
                    self.line.push(byte);
                } else {
                    self.line.clear();
                    self.oversized = true;
                }
                continue;
            }
            if !self.oversized
                && let Some(data) = self.line.strip_prefix(b"data:")
                && let Ok(event) = serde_json::from_slice::<serde_json::Value>(data)
            {
                let stage = match event["type"].as_str() {
                    Some(
                        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta",
                    ) => Some("first_reasoning"),
                    Some("response.function_call_arguments.delta") => Some("first_tool_arguments"),
                    Some("response.output_text.delta") => Some("first_text"),
                    Some("response.completed") => Some("response_completed"),
                    _ => None,
                };
                if let Some(stage) = stage
                    && self.seen.insert(stage)
                {
                    stages.push(stage);
                }
                for stage in response_shape::observe(&event) {
                    if self.seen.insert(stage) {
                        stages.push(stage);
                    }
                }
            }
            self.line.clear();
            self.oversized = false;
        }
        stages
    }
}

#[cfg(test)]
mod tests {
    use super::SseMilestones;

    #[test]
    fn observes_split_sse_without_counting_heartbeats_or_duplicate_deltas() {
        let mut sse = SseMilestones::default();
        assert!(
            sse.observe(b": heartbeat\n\nevent: delta\ndata: {\"type\":\"response.output_")
                .is_empty()
        );
        assert_eq!(
            sse.observe(b"text.delta\",\"delta\":\"hello\"}\r\n\n"),
            ["first_text", "text_delta_nonempty"]
        );
        assert!(
            sse.observe(b"data: {\"type\":\"response.output_text.delta\"}\n")
                .is_empty()
        );
        assert_eq!(
            sse.observe(b"data: {\"type\":\"response.function_call_arguments.delta\"}\n"),
            ["first_tool_arguments"]
        );
    }

    #[test]
    fn oversized_or_invalid_data_does_not_hide_following_events() {
        let mut sse = SseMilestones::default();
        assert!(sse.observe(&vec![b'x'; 70_000]).is_empty());
        assert!(sse.line.len() <= 65_536);
        assert_eq!(
            sse.observe(b"\ndata: invalid\ndata: {\"type\":\"response.completed\"}\n"),
            [
                "response_completed",
                "terminal_status_unknown",
                "terminal_output_unknown"
            ]
        );
    }
}
