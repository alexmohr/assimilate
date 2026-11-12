// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The parts every stand-in SSH host of the tests shares: a connection that
//! keeps the channels it is offered, and an SFTP handler that only needs to
//! say what it supports. The types the expansions name (`Channel`, `Msg`,
//! `Session`, `ChannelOpenHandle`, `ChannelId`, `StatusCode`, `Version`, `HashMap`, `Future`) must
//! be in scope where a macro is used.

/// `channel_open_session` for a handler holding `sessions: HashMap<ChannelId,
/// Channel<Msg>>`: it keeps the channel until a subsystem is requested on it.
macro_rules! keep_offered_channels {
    () => {
        async fn channel_open_session(
            &mut self,
            channel: Channel<Msg>,
            reply: ChannelOpenHandle,
            _session: &mut Session,
        ) -> Result<(), Self::Error> {
            self.sessions.insert(channel.id(), channel);
            reply.accept().await;
            Ok(())
        }
    };
}

/// The SFTP handler items that do not depend on what the host stores: the
/// error type, what is unsupported, and the protocol version it answers with.
macro_rules! sftp_handler_basics {
    () => {
        type Error = StatusCode;

        fn unimplemented(&self) -> Self::Error {
            StatusCode::OpUnsupported
        }

        fn init(
            &mut self,
            _version: u32,
            _extensions: HashMap<String, String>,
        ) -> impl Future<Output = Result<Version, Self::Error>> + Send {
            std::future::ready(Ok(Version::new()))
        }
    };
}
