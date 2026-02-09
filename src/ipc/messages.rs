use serde::{Deserialize, Serialize};

/// Protocol version
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum message size (1MB)
pub const MAX_MESSAGE_SIZE: usize = 1_048_576;

// ── Handshake ────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct VersionHandshake {
    pub version: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VersionResponse {
    pub version: u32,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// ── Commands (Client → Daemon) ───────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Command {
    Create {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        workspace: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        coopfile: Option<String>,
        #[serde(default)]
        detach: bool,
    },
    Attach {
        session: String,
        #[serde(default)]
        pty: u32,
        #[serde(default = "default_cols")]
        cols: u16,
        #[serde(default = "default_rows")]
        rows: u16,
    },
    Shell {
        session: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(default = "default_cols")]
        cols: u16,
        #[serde(default = "default_rows")]
        rows: u16,
    },
    Ls,
    Kill {
        session: String,
        #[serde(default)]
        all: bool,
        #[serde(default)]
        force: bool,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    Serve {
        #[serde(default = "default_port")]
        port: u16,
        #[serde(default = "default_host")]
        host: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    Tunnel {
        session: String,
    },
    Shutdown,
    Detach,
}

fn default_cols() -> u16 {
    120
}
fn default_rows() -> u16 {
    40
}
fn default_port() -> u16 {
    8888
}
fn default_host() -> String {
    "127.0.0.1".to_string()
}

// ── Responses (Daemon → Client) ──────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(flatten)]
    pub data: ResponseData,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ResponseData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pty: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<SessionInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer_sdp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qr_data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub name: String,
    pub workspace: String,
    pub pid: u32,
    pub created: u64,
    pub ptys: Vec<PtyInfo>,
    pub web_clients: u32,
    pub local_clients: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyInfo {
    pub id: u32,
    pub role: PtyRole,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PtyRole {
    Agent,
    Shell,
}

// ── Events (Daemon → Client, in stream mode) ────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DaemonEvent {
    PtyExited { code: i32 },
    PtyRestarting { delay_ms: u64 },
    Detached,
}

// ── Stream Frame Types ───────────────────────────────────────

/// Frame type tags for stream mode
pub const FRAME_PTY_DATA: u8 = 0x00;
pub const FRAME_CONTROL: u8 = 0x01;

// ── Error Codes ──────────────────────────────────────────────

pub const ERR_SESSION_NOT_FOUND: &str = "SESSION_NOT_FOUND";
pub const ERR_SESSION_EXISTS: &str = "SESSION_EXISTS";
pub const ERR_PTY_NOT_FOUND: &str = "PTY_NOT_FOUND";
pub const ERR_INVALID_COMMAND: &str = "INVALID_COMMAND";
pub const ERR_VERSION_MISMATCH: &str = "VERSION_MISMATCH";
pub const ERR_MESSAGE_TOO_LARGE: &str = "MESSAGE_TOO_LARGE";

impl Response {
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            message: None,
            data: ResponseData::default(),
        }
    }

    pub fn ok_with(data: ResponseData) -> Self {
        Self {
            ok: true,
            error: None,
            message: None,
            data,
        }
    }

    pub fn err(code: &str, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(code.to_string()),
            message: Some(message.into()),
            data: ResponseData::default(),
        }
    }

    pub fn err_with(code: &str, message: impl Into<String>, data: ResponseData) -> Self {
        Self {
            ok: false,
            error: Some(code.to_string()),
            message: Some(message.into()),
            data,
        }
    }
}
