// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Taking apart the messages tests expect, failing the test on any other.

use shared::{database::DiscoveredDatabaseSource, protocol::AgentToServer};

use crate::executor::{ExecutorCommand, ScanDatabasesRequest};

/// The request id, sources and error of a database scan answer.
pub fn database_scan_result(
    reply: Option<AgentToServer>,
) -> (String, Vec<DiscoveredDatabaseSource>, Option<String>) {
    match reply {
        Some(AgentToServer::DatabaseScanResult {
            request_id,
            sources,
            error,
        }) => (request_id, sources, error),
        other => panic!("expected a database scan result, got {other:?}"),
    }
}

/// The scan request handed to the executor.
pub fn scan_databases_request(command: Option<ExecutorCommand>) -> ScanDatabasesRequest {
    match command {
        Some(ExecutorCommand::ScanDatabases(request)) => request,
        _ => panic!("expected a database scan request for the executor"),
    }
}
