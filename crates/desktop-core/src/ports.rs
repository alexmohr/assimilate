// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::net::{Ipv4Addr, TcpListener};

/// Asks the OS for a TCP port on 127.0.0.1 that is free right now.
///
/// The listener is dropped before returning, so another process could take
/// the port before the server binds it. The window is short, and a server
/// that then fails to bind is restarted with a fresh port.
///
/// # Errors
///
/// Fails if no loopback port can be bound.
pub fn free_loopback_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_a_bindable_non_zero_port() {
        let port = free_loopback_port().unwrap();
        assert_ne!(port, 0);
        TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
    }
}
