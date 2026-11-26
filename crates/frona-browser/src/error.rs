use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("connection closed")]
    Disconnected,

    #[error("no active page")]
    NoActivePage,

    #[error("snapshot index {0} not found; call snapshot() first")]
    UnknownSnapshotIndex(usize),

    #[error("invalid target: must supply selector or snapshot index, not both")]
    InvalidTarget,

    #[error("operation timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("tool {tool}: {message}")]
    ToolFailed { tool: &'static str, message: String },

    #[error("{}", cdp_message(.0))]
    Cdp(#[from] chromiumoxide::error::CdpError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Initial connection retries are narrower than operation-level recovery:
    /// authentication, TLS, malformed protocol and timeout errors are not disconnects.
    pub fn is_retryable_connect_error(&self) -> bool {
        use chromiumoxide::error::CdpError;
        use tungstenite::{Error as WsError, error::ProtocolError};
        match self {
            Self::Disconnected | Self::Cdp(CdpError::ChannelSendError(_)) => true,
            Self::Cdp(CdpError::Ws(WsError::ConnectionClosed | WsError::AlreadyClosed)) => true,
            Self::Cdp(CdpError::Ws(WsError::Protocol(
                ProtocolError::ResetWithoutClosingHandshake
                | ProtocolError::HandshakeIncomplete
                | ProtocolError::SendAfterClosing,
            ))) => true,
            Self::Cdp(CdpError::Ws(WsError::Io(error))) => matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::UnexpectedEof
            ),
            _ => false,
        }
    }

    /// The HTTP status of a refused WebSocket upgrade, when that is what failed.
    /// Browserless answers 500 when it cannot start a browser - most often
    /// because a previous Chrome still holds the profile directory - so unlike a
    /// dropped connection this will fail the same way until something clears it.
    pub fn refused_status(&self) -> Option<u16> {
        use chromiumoxide::error::CdpError;
        use tungstenite::Error as WsError;
        match self {
            Self::Cdp(CdpError::Ws(WsError::Http(response))) => Some(response.status().as_u16()),
            _ => None,
        }
    }

    pub fn is_disconnect(&self) -> bool {
        if matches!(self, Error::Disconnected | Error::NoActivePage) {
            return true;
        }
        if let Error::Cdp(cdp) = self {
            return is_cdp_disconnect(cdp);
        }
        false
    }
}

/// A refused WebSocket upgrade carries the server's own explanation in its body
/// (Browserless says why it could not start a session), but the library's message
/// is only the status line. Keep the explanation: "HTTP error: 500" alone sends
/// people hunting through logs for what the response already said.
fn cdp_message(error: &chromiumoxide::error::CdpError) -> String {
    use chromiumoxide::error::CdpError;
    use tungstenite::Error as WsError;
    if let CdpError::Ws(WsError::Http(response)) = error
        && let Some(body) = response.body().as_deref()
    {
        let text: String = String::from_utf8_lossy(body)
            .trim()
            .chars()
            .take(300)
            .collect();
        if !text.is_empty() {
            return format!("{error}: {text}");
        }
    }
    error.to_string()
}

/// Shared by the handler loop and the public `is_disconnect` so they
/// classify identically.
pub(crate) fn is_cdp_disconnect(e: &chromiumoxide::error::CdpError) -> bool {
    use chromiumoxide::error::CdpError;
    matches!(
        e,
        CdpError::Ws(_) | CdpError::ChannelSendError(_) | CdpError::Timeout | CdpError::NoResponse
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_refused_upgrade_reports_what_the_server_said() {
        use chromiumoxide::error::CdpError;
        use tungstenite::{Error as WsError, http::Response};

        let refused = |body: Option<&[u8]>| {
            super::Error::Cdp(CdpError::Ws(WsError::Http(Box::new(
                Response::builder()
                    .status(500)
                    .body(body.map(<[u8]>::to_vec))
                    .unwrap(),
            ))))
            .to_string()
        };
        let with_reason = refused(Some(b"  Failed to launch: cannot write /profiles/alice\n"));
        assert!(with_reason.contains("500"), "{with_reason}");
        assert!(
            with_reason.ends_with("Failed to launch: cannot write /profiles/alice"),
            "{with_reason}"
        );
        // No body (or an empty one) leaves the library's own message alone.
        assert!(!refused(None).contains(": \n"));
        assert_eq!(refused(None), refused(Some(b"   ")));
        // A runaway body is cut.
        assert!(refused(Some(&[b'x'; 5000])).len() < 400);
    }

    use super::*;
    use chromiumoxide::error::CdpError;
    use tungstenite::{Error as WsError, error::ProtocolError};

    #[test]
    fn connect_retry_accepts_only_transport_interruptions() {
        for error in [
            WsError::ConnectionClosed,
            WsError::AlreadyClosed,
            WsError::Protocol(ProtocolError::HandshakeIncomplete),
            WsError::Protocol(ProtocolError::SendAfterClosing),
            WsError::Protocol(ProtocolError::ResetWithoutClosingHandshake),
            WsError::Io(std::io::ErrorKind::ConnectionReset.into()),
            WsError::Io(std::io::ErrorKind::UnexpectedEof.into()),
        ] {
            assert!(Error::Cdp(CdpError::Ws(error)).is_retryable_connect_error());
        }
        for error in [
            CdpError::Timeout,
            CdpError::NoResponse,
            CdpError::Ws(WsError::Io(std::io::ErrorKind::PermissionDenied.into())),
            CdpError::Ws(WsError::Io(std::io::ErrorKind::ConnectionRefused.into())),
            CdpError::Ws(WsError::Protocol(ProtocolError::InvalidOpcode(7))),
        ] {
            assert!(!Error::Cdp(error).is_retryable_connect_error());
        }
        assert!(!Error::NoActivePage.is_retryable_connect_error());
    }
}
