//! `pruneguard-tsgo` — optional semantic helper for pruneguard.
//!
//! This binary communicates with the pruneguard Rust core over stdio using
//! length-prefixed binary framing. It provides semantic precision refinement
//! for dead-code analysis by leveraging TypeScript type information.
//!
//! Usage:
//!   pruneguard-tsgo headless          # Run in headless mode (stdin/stdout protocol)
//!   pruneguard-tsgo --version         # Print version
//!   pruneguard-tsgo --help            # Print help

use std::io::{Read, Write};

use pruneguard_semantic_protocol::{
    ErrorMessage, HandshakeRequest, MessageType, QueryBatch, QueryResult, ReadyMessage,
    ResponseBatch, HEADER_SIZE, PROTOCOL_VERSION, decode_header, encode_message,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("pruneguard-tsgo: semantic helper for pruneguard");
        eprintln!("Usage: pruneguard-tsgo headless");
        eprintln!("       pruneguard-tsgo --version");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "headless" => {
            init_tracing();
            if let Err(e) = run_headless() {
                eprintln!("pruneguard-tsgo: fatal error: {e}");
                std::process::exit(1);
            }
        }
        "--version" | "-V" => {
            println!("pruneguard-tsgo {}", env!("CARGO_PKG_VERSION"));
        }
        "--help" | "-h" => {
            println!("pruneguard-tsgo — semantic helper for pruneguard dead-code precision");
            println!();
            println!("USAGE:");
            println!("  pruneguard-tsgo headless    Run in headless mode (stdio protocol)");
            println!("  pruneguard-tsgo --version   Print version");
            println!("  pruneguard-tsgo --help      Print this help");
        }
        other => {
            eprintln!("pruneguard-tsgo: unknown command: {other}");
            eprintln!("Usage: pruneguard-tsgo headless");
            std::process::exit(1);
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("PRUNEGUARD_TSGO_LOG")
                .unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
}

fn run_headless() -> Result<(), Box<dyn std::error::Error>> {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    // Phase 1: Read handshake from Rust core
    let (msg_type, payload) = read_message(&mut stdin)?;
    if msg_type != MessageType::Query {
        return Err(format!("expected Query (handshake), got {:?}", msg_type).into());
    }

    let handshake: HandshakeRequest = serde_json::from_slice(&payload)?;
    if handshake.version != PROTOCOL_VERSION {
        let err = ErrorMessage {
            error: format!(
                "protocol version mismatch: helper speaks v{}, core sent v{}",
                PROTOCOL_VERSION, handshake.version
            ),
            fatal: true,
        };
        send_message(&mut stdout, MessageType::Error, &serde_json::to_vec(&err)?)?;
        return Err("protocol version mismatch".into());
    }

    tracing::info!(
        project_root = %handshake.project_root,
        tsconfigs = handshake.tsconfig_paths.len(),
        "initializing semantic helper"
    );

    // TODO: Initialize TypeScript project analysis here.
    // For now, we send a Ready message indicating zero projects loaded.
    // This skeleton will be replaced with actual typescript-go integration.
    let started = std::time::Instant::now();

    let projects_loaded = 0;
    let files_indexed = 0;

    let ready = ReadyMessage {
        version: PROTOCOL_VERSION,
        projects_loaded,
        files_indexed,
        init_ms: started.elapsed().as_millis() as u64,
    };
    send_message(&mut stdout, MessageType::Ready, &serde_json::to_vec(&ready)?)?;

    tracing::info!(
        projects = projects_loaded,
        files = files_indexed,
        init_ms = started.elapsed().as_millis(),
        "semantic helper ready"
    );

    // Phase 2: Query loop
    loop {
        let (msg_type, payload) = match read_message(&mut stdin) {
            Ok(msg) => msg,
            Err(e) => {
                tracing::debug!("stdin closed or error: {e}");
                break;
            }
        };

        match msg_type {
            MessageType::Shutdown => {
                tracing::info!("received shutdown signal");
                break;
            }
            MessageType::Query => {
                let batch: QueryBatch = serde_json::from_slice(&payload)?;
                let batch_started = std::time::Instant::now();

                tracing::debug!(
                    queries = batch.queries.len(),
                    tsconfig = %batch.tsconfig_path,
                    "processing query batch"
                );

                // TODO: Process queries against TypeScript semantic model.
                // For now, return stub results indicating the query was not
                // processed (success=false with a descriptive error).
                let results: Vec<QueryResult> = batch
                    .queries
                    .iter()
                    .map(|q| QueryResult {
                        id: q.id,
                        success: false,
                        error: Some("semantic helper not yet implemented".to_string()),
                        references: Vec::new(),
                        total_references: 0,
                        is_type_only: None,
                        alias_chain: Vec::new(),
                    })
                    .collect();

                let response = ResponseBatch {
                    results,
                    batch_ms: batch_started.elapsed().as_millis() as u64,
                };
                send_message(
                    &mut stdout,
                    MessageType::Response,
                    &serde_json::to_vec(&response)?,
                )?;
            }
            _ => {
                let err = ErrorMessage {
                    error: format!("unexpected message type: {:?}", msg_type),
                    fatal: false,
                };
                send_message(&mut stdout, MessageType::Error, &serde_json::to_vec(&err)?)?;
            }
        }
    }

    Ok(())
}

fn read_message(reader: &mut impl Read) -> Result<(MessageType, Vec<u8>), Box<dyn std::error::Error>> {
    let mut header = [0u8; HEADER_SIZE];
    reader.read_exact(&mut header)?;

    let (size, msg_type) = decode_header(&header)
        .ok_or_else(|| format!("invalid header: {:?}", header))?;

    let mut payload = vec![0u8; size as usize];
    reader.read_exact(&mut payload)?;

    Ok((msg_type, payload))
}

fn send_message(
    writer: &mut impl Write,
    msg_type: MessageType,
    payload: &[u8],
) -> Result<(), std::io::Error> {
    let msg = encode_message(msg_type, payload);
    writer.write_all(&msg)?;
    writer.flush()
}
