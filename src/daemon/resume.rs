use crate::history::{HistoryQuery, HistoryRecord, HistoryService, HistoryStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResumeDecision {
    New,
    ResumeSeed { source_id: String, text: String },
}

pub(super) fn decision_from_latest(record: Option<&HistoryRecord>) -> ResumeDecision {
    match record {
        Some(record)
            if record.status == HistoryStatus::Canceled && !record.asr.text.trim().is_empty() =>
        {
            ResumeDecision::ResumeSeed {
                source_id: record.id.clone(),
                text: record.asr.text.clone(),
            }
        }
        Some(record)
            if record.status == HistoryStatus::Timeout
                && record
                    .error
                    .as_ref()
                    .is_some_and(|error| error.kind == "asr_timeout")
                && !record.asr.text.trim().is_empty() =>
        {
            ResumeDecision::ResumeSeed {
                source_id: record.id.clone(),
                text: record.asr.text.clone(),
            }
        }
        _ => ResumeDecision::New,
    }
}

pub(super) async fn latest_decision(history: HistoryService) -> anyhow::Result<ResumeDecision> {
    let page = tokio::task::spawn_blocking(move || {
        history.page(HistoryQuery {
            limit: 1,
            ..HistoryQuery::default()
        })
    })
    .await
    .map_err(|error| anyhow::anyhow!("join latest resume history lookup: {error}"))??;
    Ok(decision_from_latest(page.records.first()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{
        AsrHistory, AsrSessionHistory, HistoryError, HistoryRecord, PipelineStepHistory,
    };
    use time::OffsetDateTime;

    fn record(id: &str, status: HistoryStatus, asr_text: &str) -> HistoryRecord {
        let started_at = OffsetDateTime::from_unix_timestamp(1_750_000_000).unwrap();
        HistoryRecord {
            version: 1,
            id: id.to_string(),
            started_at,
            ended_at: started_at + time::Duration::seconds(1),
            duration_ms: 1_000,
            status,
            app: None,
            text: asr_text.to_string(),
            text_stats: crate::text_stats::compute(asr_text),
            asr: AsrHistory {
                provider: "test".to_string(),
                text: asr_text.to_string(),
                duration_ms: 1_000,
                audio_ms: 1_000,
                sessions: vec![AsrSessionHistory {
                    text: asr_text.to_string(),
                    started_at,
                    ended_at: started_at + time::Duration::seconds(1),
                    audio_ms: 1_000,
                }],
            },
            pipeline: Vec::<PipelineStepHistory>::new(),
            error: None,
        }
    }

    #[test]
    fn submitted_returns_new() {
        let record = record("submitted", HistoryStatus::Submitted, "text");

        assert_eq!(decision_from_latest(Some(&record)), ResumeDecision::New);
    }

    #[test]
    fn canceled_with_non_empty_asr_text_returns_seed() {
        let record = record("canceled", HistoryStatus::Canceled, "old text");

        assert_eq!(
            decision_from_latest(Some(&record)),
            ResumeDecision::ResumeSeed {
                source_id: "canceled".to_string(),
                text: "old text".to_string(),
            }
        );
    }

    #[test]
    fn canceled_with_empty_asr_text_returns_new() {
        let record = record("canceled-empty", HistoryStatus::Canceled, "  ");

        assert_eq!(decision_from_latest(Some(&record)), ResumeDecision::New);
    }

    #[test]
    fn asr_timeout_with_non_empty_asr_text_returns_seed() {
        let mut record = record("timeout", HistoryStatus::Timeout, "old text");
        record.error = Some(HistoryError {
            kind: "asr_timeout".to_string(),
            msg: "finalize timed out".to_string(),
        });

        assert_eq!(
            decision_from_latest(Some(&record)),
            ResumeDecision::ResumeSeed {
                source_id: "timeout".to_string(),
                text: "old text".to_string(),
            }
        );
    }

    #[test]
    fn timeout_with_other_error_kind_returns_new() {
        let mut record = record("timeout", HistoryStatus::Timeout, "old text");
        record.error = Some(HistoryError {
            kind: "dispatch".to_string(),
            msg: "not recoverable".to_string(),
        });

        assert_eq!(decision_from_latest(Some(&record)), ResumeDecision::New);
    }

    #[test]
    fn error_and_empty_return_new() {
        let error = record("error", HistoryStatus::Error, "old text");
        let empty = record("empty", HistoryStatus::Empty, "old text");

        assert_eq!(decision_from_latest(Some(&error)), ResumeDecision::New);
        assert_eq!(decision_from_latest(Some(&empty)), ResumeDecision::New);
        assert_eq!(decision_from_latest(None), ResumeDecision::New);
    }

    #[tokio::test]
    async fn latest_decision_uses_only_latest_record() {
        let history = HistoryService::with_dir(
            std::env::temp_dir().join(format!("shuohua-resume-test-{}", ulid::Ulid::generate())),
        );
        let older = record("older", HistoryStatus::Canceled, "old text");
        let mut newer = record("newer", HistoryStatus::Submitted, "new text");
        newer.started_at = older.started_at + time::Duration::seconds(1);
        newer.ended_at = newer.started_at + time::Duration::seconds(1);

        history.append(older).unwrap();
        history.append(newer).unwrap();

        assert_eq!(latest_decision(history).await.unwrap(), ResumeDecision::New);
    }
}
