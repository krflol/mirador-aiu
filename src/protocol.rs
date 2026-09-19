use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostMessage {
    Hello {
        protocol: u16,
        host_version: String,
        plugin: String,
        config: serde_json::Value,
        cwd: String,
    },
    Resize {
        columns: u16,
        rows: u16,
    },
    Focus {
        focused: bool,
    },
    Key {
        key: String,
        code: String,
        text: Option<String>,
        modifiers: Vec<String>,
    },
    Paste {
        text: String,
    },
    Mouse {
        kind: String,
        button: Option<String>,
        column: u16,
        row: u16,
        modifiers: Vec<String>,
    },
    Tick,
    Shutdown,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Frame {
    pub revision: u64,
    pub title: String,
    pub counter: String,
    pub lines: Vec<Line>,
    pub bindings: Vec<Binding>,
    pub input: InputPolicy,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Line {
    pub spans: Vec<Span>,
}

impl Line {
    pub fn text(text: impl Into<String>, color: &str) -> Self {
        Self {
            spans: vec![Span {
                text: text.into(),
                fg: color.into(),
                bold: false,
            }],
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Span {
    pub text: String,
    pub fg: String,
    pub bold: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Binding {
    pub key: String,
    pub action: String,
    pub primary: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct InputPolicy {
    pub capture: bool,
    pub keys: Vec<String>,
    pub paste: bool,
    pub mouse: bool,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound<'a> {
    Ready {
        protocol: u16,
        title: &'a str,
        refresh_ms: u64,
    },
    Frame {
        #[serde(flatten)]
        frame: &'a Frame,
    },
    Error {
        message: &'a str,
        fatal: bool,
    },
    Watch {
        text: &'a str,
    },
}

pub fn write_message(mut writer: impl Write, message: &Outbound<'_>) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(message)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "outbound frame exceeds protocol limit",
        ));
    }
    writer.write_all(&bytes)?;
    writer.flush()
}

pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<HostMessage>> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unterminated protocol message",
                ))
            };
        }
        let size = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |n| n + 1);
        if bytes.len() + size > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "protocol message too large",
            ));
        }
        let complete = available[size - 1] == b'\n';
        bytes.extend_from_slice(&available[..size]);
        reader.consume(size);
        if complete {
            return serde_json::from_slice(&bytes).map(Some).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid protocol message")
            });
        }
    }
}
