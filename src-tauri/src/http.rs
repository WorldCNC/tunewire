//! One shared HTTP client (connection reuse) for every non-yt-dlp request:
//! streams, lyrics, Spotify embed pages, the yt-dlp self-installer.

use std::sync::OnceLock;
use std::time::Duration;

use ureq::unversioned::transport::{Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport};

pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140 Safari/537.36";

/// Statuses are returned (not turned into errors) so callers can map them.
pub fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::config::Config::builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(15)))
            .timeout_recv_body(Some(Duration::from_secs(30)))
            .build()
            .into()
    })
}

/// A stream connection that receives nothing for this long has stalled.
pub const STREAM_IDLE: Duration = Duration::from_secs(20);

/// For audio streams: no deadline for a whole body (on a slow line a song,
/// or a 4 MB chunk, may take minutes), but a read that waits longer than
/// `STREAM_IDLE` fails, and the downloader resumes where it stopped.
pub fn stream_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let config = ureq::config::Config::builder().http_status_as_error(false).timeout_connect(Some(Duration::from_secs(10))).timeout_recv_response(Some(Duration::from_secs(15))).build();
        ureq::Agent::with_parts(config, IdleConnector(DefaultConnector::default()), ureq::unversioned::resolver::DefaultResolver::default())
    })
}

/// The default connections, with every wait for input capped at `STREAM_IDLE`.
#[derive(Debug)]
struct IdleConnector(DefaultConnector);

impl Connector for IdleConnector {
    type Out = IdleTransport;

    fn connect(&self, details: &ConnectionDetails, chained: Option<()>) -> Result<Option<IdleTransport>, ureq::Error> {
        Ok(self.0.connect(details, chained)?.map(IdleTransport))
    }
}

#[derive(Debug)]
struct IdleTransport(Box<dyn Transport>);

impl Transport for IdleTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.0.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.0.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let idle = STREAM_IDLE.into();
        self.0.await_input(NextTimeout { after: timeout.after.min(idle), ..timeout })
    }

    fn is_open(&mut self) -> bool {
        self.0.is_open()
    }

    fn is_tls(&self) -> bool {
        self.0.is_tls()
    }
}
