use crate::{error::Error, logs::schema::Message};
use axum::{
    body::StreamBody,
    response::{IntoResponse, Response},
    Json,
};
use futures::stream;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use serde_json::json;
use tracing::warn;

pub struct LogsResponse {
    pub response_type: LogsResponseType,
    pub reverse: bool,
}

pub enum LogsResponseType {
    Raw(Vec<String>),
    Processed(ProcessedLogs),
}

pub struct ProcessedLogs {
    pub messages: Vec<Message>,
    pub logs_type: ProcessedLogsType,
}

impl ProcessedLogs {
    pub fn parse_raw(lines: Vec<String>, logs_type: ProcessedLogsType) -> Self {
        let messages = lines
            .into_par_iter()
            .filter_map(|line| match Message::parse_from_raw_irc(line) {
                Ok(msg) => Some(msg),
                Err(err) => {
                    warn!("Could not parse message: {err:#}");
                    None
                }
            })
            .collect();

        Self {
            messages,
            logs_type,
        }
    }
}

pub enum ProcessedLogsType {
    Text,
    Json,
}

impl IntoResponse for LogsResponse {
    fn into_response(self) -> Response {
        match self.response_type {
            LogsResponseType::Raw(mut lines) => {
                if self.reverse {
                    lines.reverse();
                }

                let lines = lines
                    .into_iter()
                    .flat_map(|line| vec![Ok::<_, Error>(line), Ok("\n".to_owned())]);

                let stream = stream::iter(lines);
                StreamBody::new(stream).into_response()
            }
            LogsResponseType::Processed(processed_logs) => {
                let mut messages = processed_logs.messages;
                if self.reverse {
                    messages.reverse();
                }

                match processed_logs.logs_type {
                    ProcessedLogsType::Text => messages
                        .into_iter()
                        .map(|message| message.to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                        .into_response(),
                    ProcessedLogsType::Json => Json(json!({
                        "messages": messages,
                    }))
                    .into_response(),
                }
            }
        }
    }
}
