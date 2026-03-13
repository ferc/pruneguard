//! Wire protocol for communication between the pruneguard Rust core and
//! the optional `pruneguard-tsgo` semantic helper binary.
//!
//! Communication happens over stdio using length-prefixed binary framing:
//!
//! ```text
//! [4 bytes: u32 LE payload size][1 byte: message type][JSON payload...]
//! ```
//!
//! Message types:
//! - 0x00: Error
//! - 0x01: Query (stdin, Rust -> helper)
//! - 0x02: Response (stdout, helper -> Rust)
//! - 0x03: Ready (stdout, helper -> Rust, sent once after startup)
//! - 0x04: Shutdown (stdin, Rust -> helper)

use serde::{Deserialize, Serialize};

/// Protocol version. Bumped on breaking changes.
pub const PROTOCOL_VERSION: u32 = 1;

/// Header size: 4 bytes for payload length + 1 byte for message type.
pub const HEADER_SIZE: usize = 5;

/// Maximum payload size (16 MiB).
pub const MAX_PAYLOAD_SIZE: u32 = 16 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// Discriminant byte for message types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    Error = 0x00,
    Query = 0x01,
    Response = 0x02,
    Ready = 0x03,
    Shutdown = 0x04,
}

impl MessageType {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x00 => Some(Self::Error),
            0x01 => Some(Self::Query),
            0x02 => Some(Self::Response),
            0x03 => Some(Self::Ready),
            0x04 => Some(Self::Shutdown),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Handshake
// ---------------------------------------------------------------------------

/// Sent by the Rust core on stdin immediately after spawning the helper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeRequest {
    /// Protocol version the core expects.
    pub version: u32,
    /// Absolute paths to tsconfig.json / jsconfig.json files.
    pub tsconfig_paths: Vec<String>,
    /// Root directory of the project.
    pub project_root: String,
}

/// Sent by the helper on stdout after successful initialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyMessage {
    /// Protocol version the helper speaks.
    pub version: u32,
    /// Number of TypeScript projects loaded.
    pub projects_loaded: usize,
    /// Total files indexed across all projects.
    pub files_indexed: usize,
    /// Milliseconds taken to initialize.
    pub init_ms: u64,
}

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

/// The kind of semantic query to perform.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    /// Find all references to a specific export across the project.
    FindExportReferences,
    /// Find all references to a specific member (method, property, enum variant).
    FindMemberReferences,
    /// Check if an export is used within the same file (local consumption).
    FindSameFileExportUsage,
    /// Resolve a namespace alias chain (e.g. `import NS = Other.Sub`).
    ResolveNamespaceAliasChain,
    /// Classify whether a usage is type-only or value-usage.
    ClassifyTypeOnlyVsValueUsage,
}

/// A single query sent from the Rust core to the helper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticQuery {
    /// Unique query ID for correlation.
    pub id: u64,
    /// The kind of query.
    pub kind: QueryKind,
    /// File path containing the export/member.
    pub file_path: String,
    /// Name of the export (for export-level queries).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_name: Option<String>,
    /// Name of the parent symbol (for member-level queries).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_name: Option<String>,
    /// Name of the member (for member-level queries).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_name: Option<String>,
}

/// A batch of queries sent in a single message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryBatch {
    /// Queries in this batch.
    pub queries: Vec<SemanticQuery>,
    /// Tsconfig path that covers these queries (for project scoping).
    pub tsconfig_path: String,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// A reference found by the helper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundReference {
    /// File path containing the reference.
    pub file_path: String,
    /// Line number (1-based).
    pub line: u32,
    /// Column number (0-based byte offset).
    pub column: u32,
    /// Whether this is a type-only reference.
    pub is_type_only: bool,
    /// Whether this is a write (assignment) rather than a read.
    pub is_write: bool,
}

/// Result of a single semantic query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// Correlation ID matching the query.
    pub id: u64,
    /// Whether the query was successfully processed.
    pub success: bool,
    /// Error message if the query failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// References found (for find-references queries).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<FoundReference>,
    /// Total reference count (may exceed `references.len()` if truncated).
    pub total_references: usize,
    /// Whether the export/member is type-only (for classification queries).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_type_only: Option<bool>,
    /// Resolved alias chain (for namespace alias queries).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alias_chain: Vec<String>,
}

/// A batch of query results in a single response message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseBatch {
    /// Results for each query in the corresponding batch.
    pub results: Vec<QueryResult>,
    /// Wall-clock milliseconds to process this batch.
    pub batch_ms: u64,
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Error message from the helper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMessage {
    /// Human-readable error description.
    pub error: String,
    /// Whether this error is fatal (helper will exit).
    pub fatal: bool,
}

// ---------------------------------------------------------------------------
// Framing helpers
// ---------------------------------------------------------------------------

/// Encode a message into the wire format.
pub fn encode_message(msg_type: MessageType, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut buf = Vec::with_capacity(HEADER_SIZE + payload.len());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.push(msg_type as u8);
    buf.extend_from_slice(payload);
    buf
}

/// Decode the header from a 5-byte buffer.
/// Returns `(payload_size, message_type)`.
pub fn decode_header(header: &[u8; HEADER_SIZE]) -> Option<(u32, MessageType)> {
    let size = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let msg_type = MessageType::from_byte(header[4])?;
    Some((size, msg_type))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encode_decode() {
        let payload = b"hello";
        let encoded = encode_message(MessageType::Query, payload);
        assert_eq!(encoded.len(), HEADER_SIZE + payload.len());

        let mut header = [0u8; HEADER_SIZE];
        header.copy_from_slice(&encoded[..HEADER_SIZE]);
        let (size, msg_type) = decode_header(&header).unwrap();
        assert_eq!(size, payload.len() as u32);
        assert_eq!(msg_type, MessageType::Query);
        assert_eq!(&encoded[HEADER_SIZE..], payload);
    }

    #[test]
    fn query_serialization() {
        let query = SemanticQuery {
            id: 1,
            kind: QueryKind::FindExportReferences,
            file_path: "src/utils.ts".to_string(),
            export_name: Some("formatDate".to_string()),
            parent_name: None,
            member_name: None,
        };
        let json = serde_json::to_string(&query).unwrap();
        let decoded: SemanticQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, 1);
        assert_eq!(decoded.kind, QueryKind::FindExportReferences);
    }

    #[test]
    fn message_type_from_byte() {
        assert_eq!(MessageType::from_byte(0x00), Some(MessageType::Error));
        assert_eq!(MessageType::from_byte(0x01), Some(MessageType::Query));
        assert_eq!(MessageType::from_byte(0x02), Some(MessageType::Response));
        assert_eq!(MessageType::from_byte(0x03), Some(MessageType::Ready));
        assert_eq!(MessageType::from_byte(0x04), Some(MessageType::Shutdown));
        assert_eq!(MessageType::from_byte(0xFF), None);
    }
}
