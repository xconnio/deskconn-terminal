//! The local deskconn daemon (`desk serve`), and the machines the CLI knows about.
//!
//! The daemon serves the CLI's realm on `~/.deskconn/deskconn.sock`, or on Windows on loopback,
//! as that file says. Its stream proxy opens a
//! feature's stream to a machine on the daemon's persistent connection to it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use xconn::Value;
#[cfg(not(windows))]
use xconn::async_::client::connect_anonymous;
#[cfg(windows)]
use xconn::async_::client::connect_wampcra;
use xconn::async_::rawsocket::BoxStream;
use xconn::async_::session::Session;

const LOCAL_REALM: &str = "local.deskconn";
const PROXY_STREAM: &str = "local.deskconn.proxy.stream";
const TERMINAL_STREAM: &str = "com.deskconn.terminal.stream";

/// A machine of the signed-in account.
#[derive(Debug, Clone, Deserialize)]
pub struct Machine {
    pub authid: String,
    pub name: String,
    pub realm: String,
    #[serde(default)]
    pub alias: String,
}

impl Machine {
    /// The name the user knows the machine by.
    pub fn label(&self) -> &str {
        if self.alias.is_empty() { &self.name } else { &self.alias }
    }
}

/// The CLI's configuration directory.
pub fn config_dir() -> Result<PathBuf> {
    Ok(dirs::home_dir().context("no home directory")?.join(".deskconn"))
}

/// The machines the CLI lists, as `desk ls` last saved them, with this machine first.
pub fn machines(dir: &Path) -> Result<Vec<Machine>> {
    #[derive(Deserialize)]
    struct Config {
        #[serde(default)]
        machines: Vec<Machine>,
    }
    #[derive(Deserialize)]
    struct Credentials {
        authid: String,
    }

    let path = dir.join("config.yml");
    // None yet, until `desk ls` lists them.
    let data = match std::fs::read_to_string(&path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        data => data.with_context(|| format!("read {}", path.display()))?,
    };
    let config: Config = serde_yaml_ng::from_str(&data).with_context(|| format!("read {}", path.display()))?;
    let mut machines: Vec<_> = config.machines.into_iter().filter(|m| !m.realm.is_empty()).collect();
    // This machine's own credentials, if it's attached.
    let own = std::fs::read_to_string(dir.join("credentials.json"))
        .ok()
        .and_then(|data| serde_json::from_str::<Credentials>(&data).ok());
    if let Some(own) = own
        && let Some(i) = machines.iter().position(|m| m.authid == own.authid)
    {
        let machine = machines.remove(i);
        machines.insert(0, machine);
    }
    Ok(machines)
}

/// A connection to the local daemon.
pub struct Daemon {
    session: Session,
}

impl Daemon {
    pub async fn connect(dir: &Path) -> Result<Self> {
        let socket = dir.join("deskconn.sock");
        #[cfg(not(windows))]
        let session = connect_anonymous(&format!("unix+yamux://{}", socket.display()), LOCAL_REALM).await;
        // Windows has no Unix sockets for the daemon, which serves on loopback instead, for those
        // with the secret in its endpoint file, in place of the socket.
        #[cfg(windows)]
        let session = {
            #[derive(Deserialize)]
            struct Endpoint {
                address: String,
                secret: String,
            }
            let data = std::fs::read_to_string(&socket)
                .context("read the deskconn daemon's endpoint (is `desk serve` running?)")?;
            let endpoint: Endpoint = serde_json::from_str(&data).context("read the deskconn daemon's endpoint")?;
            connect_wampcra(
                &format!("yamux://{}", endpoint.address),
                LOCAL_REALM,
                "local",
                &endpoint.secret,
            )
            .await
        };
        let session = session.map_err(|e| {
            anyhow!(
                "connect to the deskconn daemon (is `desk serve` running?): {}",
                e.message
            )
        })?;
        Ok(Self { session })
    }

    /// Opens a stream to the persistent terminal sessions of the machine of realm.
    pub async fn open_terminal(&self, realm: &str) -> Result<BoxStream> {
        let result = self
            .session
            .call(PROXY_STREAM)
            .arg(realm)
            .arg(TERMINAL_STREAM)
            .execute()
            .await
            .map_err(|e| anyhow!("open terminal: {}", e.message))?;
        if let Some(error) = result.error {
            bail!("open terminal: {}", describe(&error.uri, error.args.as_deref()));
        }
        let reference = result
            .args
            .as_ref()
            .and_then(|args| args.get(1))
            .context("open terminal: the daemon returned no stream")?;
        self.session
            .open_stream_argument(reference)
            .await
            .map_err(|e| anyhow!("open terminal stream: {}", e.message))
    }
}

/// An error's URI, with its message when it has one.
fn describe(uri: &str, args: Option<&[Value]>) -> String {
    match args.and_then(|args| args.first()) {
        Some(Value::Str(message)) => format!("{message} ({uri})"),
        _ => uri.to_owned(),
    }
}
