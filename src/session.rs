//! A tab's persistent terminal session on a machine.
//!
//! The session runs on the machine and outlives its connection: when the connection is lost,
//! it's attached again and only the output missed in the meantime is restored.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use tokio::io::WriteHalf;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep, timeout};
use xconn::async_::rawsocket::BoxStream;

use crate::daemon::Daemon;
use crate::protocol::{self, ControlMsg, Keys, Message};

/// How often an idle attachment pings, to keep the machine from timing it out.
const PING_INTERVAL: Duration = Duration::from_secs(10);
/// How long setting up an attachment may take.
const SETUP_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a lost connection is tried again.
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const RETRY_MAX: Duration = Duration::from_secs(5);
/// The most input one frame carries, well within the machine's limit on a frame.
const INPUT_MAX: usize = 32 * 1024;

/// What a session tells its tab.
#[derive(Debug)]
pub enum Event {
    /// Output for the terminal.
    Output(Vec<u8>),
    /// The connection was lost, and is being restored.
    Reconnecting,
    /// The connection is back.
    Reconnected,
    /// Output missed while the connection was lost is no longer kept on the machine; the
    /// screen is restored without it.
    OutputLost,
    /// The shell exited, with its status, or the tab ended the session. The tab closes
    /// either way.
    #[allow(dead_code)]
    Exited(i32),
    /// Another client took the session over.
    Displaced,
    /// The session couldn't be started, or its connection restored.
    Failed(String),
}

/// What a tab tells its session.
pub enum Command {
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
    },
    /// Ends the session, and the shell with it.
    Terminate,
}

/// Runs a new session on the machine of realm until it ends, taking the tab's commands and
/// sending it events. Its last event is Exited, Displaced or Failed.
pub async fn run(
    dir: PathBuf,
    realm: String,
    cols: u16,
    rows: u16,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<Event>,
) {
    let mut session = Session {
        dir,
        realm,
        cols,
        rows,
        id: String::new(),
        token: String::new(),
    };
    match session.run(&mut commands, &events).await {
        Ok(end) => {
            let _ = events.send(end);
        }
        Err(err) => {
            let _ = events.send(Event::Failed(format!("{err:#}")));
            // The session may still run on the machine: it's ended once the tab is closed.
            while let Some(command) = commands.recv().await {
                if let Command::Terminate = command {
                    break;
                }
            }
            session.terminate().await;
        }
    }
}

struct Session {
    dir: PathBuf,
    realm: String,
    cols: u16,
    rows: u16,
    /// The session's ID and its attachment's token, once it's started.
    id: String,
    token: String,
}

/// An open attachment to the session.
struct Attachment {
    keys: Keys,
    writer: WriteHalf<BoxStream>,
    /// The messages read from the stream, ending with an error when it does.
    messages: mpsc::Receiver<Result<Message>>,
    reply: ControlMsg,
}

impl Session {
    /// Runs the session until it ends, returning how.
    async fn run(
        &mut self,
        commands: &mut mpsc::UnboundedReceiver<Command>,
        events: &mpsc::UnboundedSender<Event>,
    ) -> Result<Event> {
        let create = ControlMsg {
            op: protocol::OP_CREATE.into(),
            ..Default::default()
        };
        let mut attachment = self.attach(create).await?;
        // A new session has no screen to restore; after a reconnect, the snapshot redraws it.
        let mut restore = false;
        loop {
            let seen = match self.serve(&mut attachment, restore, commands, events).await {
                Ok(end) => return Ok(end),
                Err(seen) => seen,
            };
            let _ = events.send(Event::Reconnecting);
            attachment = match self.reconnect(seen, commands).await? {
                Ok(attachment) => attachment,
                Err(end) => return Ok(end),
            };
            let _ = events.send(Event::Reconnected);
            restore = true;
        }
    }

    /// Opens a stream to the machine's terminal sessions and makes req on it, returning the
    /// attachment it starts, if any.
    async fn attach(&mut self, mut req: ControlMsg) -> Result<Attachment> {
        req.cols = self.cols;
        req.rows = self.rows;
        let attachment = timeout(SETUP_TIMEOUT, request(&self.dir, &self.realm, &req))
            .await
            .map_err(|_| anyhow!("the machine didn't answer in time"))??;
        self.id.clone_from(&attachment.reply.session);
        self.token.clone_from(&attachment.reply.token);
        Ok(attachment)
    }

    /// Serves an attachment until the session ends, or the connection does (Err, with the
    /// output offset the terminal has seen up to). Unless restore is set, the snapshot the
    /// output starts with isn't shown.
    async fn serve(
        &mut self,
        attachment: &mut Attachment,
        restore: bool,
        commands: &mut mpsc::UnboundedReceiver<Command>,
        events: &mpsc::UnboundedSender<Event>,
    ) -> Result<Event, u64> {
        let mut seen = attachment.reply.offset;
        let mut snapshot = attachment.reply.snapshot;
        let mut ping = tokio::time::interval_at(Instant::now() + PING_INTERVAL, PING_INTERVAL);
        if restore && attachment.reply.history_gap {
            let _ = events.send(Event::OutputLost);
        }
        loop {
            let sent = tokio::select! {
                message = attachment.messages.recv() => match message {
                    Some(Ok(Message::Data(data))) => {
                        let n = snapshot.min(data.len());
                        snapshot -= n;
                        seen += (data.len() - n) as u64;
                        let shown = if restore { &data[..] } else { &data[n..] };
                        if !shown.is_empty() {
                            let _ = events.send(Event::Output(shown.to_vec()));
                        }
                        Ok(())
                    }
                    Some(Ok(Message::Control(msg))) => match msg.op.as_str() {
                        protocol::OP_EXIT => return Ok(Event::Exited(msg.exit_code.unwrap_or(0))),
                        protocol::OP_DISPLACED => return Ok(Event::Displaced),
                        _ => Ok(()),
                    },
                    Some(Err(err)) => Err(err),
                    None => Err(anyhow!("the connection closed")),
                },
                command = commands.recv() => match command {
                    Some(Command::Input(data)) => attachment.send_input(&data).await,
                    Some(Command::Resize { cols, rows }) => {
                        (self.cols, self.rows) = (cols, rows);
                        attachment
                            .send_control(ControlMsg { op: protocol::OP_SIZE.into(), cols, rows, ..Default::default() })
                            .await
                    }
                    Some(Command::Terminate) | None => {
                        self.terminate().await;
                        return Ok(Event::Exited(0));
                    }
                },
                _ = ping.tick() => {
                    attachment.send_control(ControlMsg { op: protocol::OP_PING.into(), ..Default::default() }).await
                }
            };
            if let Err(err) = sent {
                log(&format!("connection to session {} lost: {err:#}", self.id));
                return Err(seen);
            }
        }
    }

    /// Attaches to the session again, until it's attached, it ends for the tab (Ok(Err), with
    /// how: the tab ended it, or another client took it over), it's refused, or the time for
    /// it runs out. Input meanwhile is dropped: the session might not get it.
    async fn reconnect(
        &mut self,
        seen: u64,
        commands: &mut mpsc::UnboundedReceiver<Command>,
    ) -> Result<Result<Attachment, Event>> {
        let deadline = Instant::now() + RECONNECT_TIMEOUT;
        let mut wait = Duration::from_millis(250);
        loop {
            let req = ControlMsg {
                op: protocol::OP_ATTACH.into(),
                session: self.id.clone(),
                token: self.token.clone(),
                since: Some(seen),
                ..Default::default()
            };
            let err = match self.attach(req).await {
                Ok(attachment) => return Ok(Ok(attachment)),
                Err(err) => err,
            };
            match err.downcast_ref::<Refused>() {
                Some(refused) if refused.code == protocol::ERR_NOT_FOUND => {
                    self.id.clear();
                    bail!("the session has ended");
                }
                Some(refused) if refused.code == protocol::ERR_ATTACHED => return Ok(Err(Event::Displaced)),
                Some(refused) if refused.code != protocol::ERR_UNAVAILABLE => return Err(err),
                _ => {}
            }
            if Instant::now() + wait > deadline {
                return Err(err.context("could not reconnect to the session"));
            }
            let retry = sleep(wait);
            tokio::pin!(retry);
            loop {
                tokio::select! {
                    _ = &mut retry => break,
                    command = commands.recv() => match command {
                        Some(Command::Resize { cols, rows }) => (self.cols, self.rows) = (cols, rows),
                        Some(Command::Input(_)) => {}
                        Some(Command::Terminate) | None => {
                            self.terminate().await;
                            return Ok(Err(Event::Exited(0)));
                        }
                    },
                }
            }
            wait = (wait * 2).min(RETRY_MAX);
        }
    }

    /// Ends the session and its processes, if the machine can be reached.
    async fn terminate(&self) {
        if self.id.is_empty() {
            return;
        }
        let req = ControlMsg {
            op: protocol::OP_TERMINATE.into(),
            session: self.id.clone(),
            ..Default::default()
        };
        match timeout(SETUP_TIMEOUT, request(&self.dir, &self.realm, &req)).await {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => log(&format!("could not end session {}: {err:#}", self.id)),
            Err(_) => log(&format!("could not end session {}: the machine didn't answer", self.id)),
        }
    }
}

/// Opens a stream to the terminal sessions of realm's machine, through the daemon of dir,
/// and makes req on it. A refusal is a [`Refused`] error.
async fn request(dir: &std::path::Path, realm: &str, req: &ControlMsg) -> Result<Attachment> {
    let daemon = Daemon::connect(dir).await?;
    let mut stream = daemon.open_terminal(realm).await?;
    let keys = protocol::exchange_keys(&mut stream).await?;
    protocol::write_frame(&mut stream, &keys.seal_control(req)?).await?;
    let reply = match keys.open(&protocol::read_frame(&mut stream).await?)? {
        Message::Control(reply) => reply,
        Message::Data(_) => bail!("unexpected reply from the machine"),
    };
    if !reply.error.is_empty() || !reply.code.is_empty() {
        return Err(Refused {
            code: reply.code,
            message: reply.error,
        }
        .into());
    }

    let (mut reader, writer) = tokio::io::split(stream);
    let (tx, messages) = mpsc::channel(64);
    let reader_keys = keys.clone();
    tokio::spawn(async move {
        // The daemon's session carries the stream: it's kept as long as the stream is read.
        let _daemon = daemon;
        loop {
            let message = match protocol::read_frame(&mut reader).await {
                Ok(frame) => reader_keys.open(&frame),
                Err(err) => Err(err),
            };
            let failed = message.is_err();
            if tx.send(message).await.is_err() || failed {
                return;
            }
        }
    });
    Ok(Attachment {
        keys,
        writer,
        messages,
        reply,
    })
}

impl Attachment {
    async fn send_input(&mut self, data: &[u8]) -> Result<()> {
        for chunk in data.chunks(INPUT_MAX) {
            protocol::write_frame(&mut self.writer, &self.keys.seal(protocol::KIND_DATA, chunk)?).await?;
        }
        Ok(())
    }

    async fn send_control(&mut self, msg: ControlMsg) -> Result<()> {
        protocol::write_frame(&mut self.writer, &self.keys.seal_control(&msg)?).await
    }
}

/// The machine refusing a request, with its code.
#[derive(Debug)]
struct Refused {
    code: String,
    message: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Refused {}

fn log(message: &str) {
    eprintln!("deskconn-terminal: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a command in a new session on this machine. Needs a running, attached daemon.
    #[tokio::test]
    #[ignore = "needs a running deskconn daemon"]
    async fn runs_a_command() {
        let dir = crate::daemon::config_dir().unwrap();
        let machine = crate::daemon::machines(&dir).unwrap().remove(0);
        let (commands, rx) = mpsc::unbounded_channel();
        let (tx, mut events) = mpsc::unbounded_channel();
        tokio::spawn(run(dir, machine.realm, 80, 24, rx, tx));
        commands
            .send(Command::Input(b"echo deskconn-$((6*7)); exit 3\r".to_vec()))
            .unwrap();

        let mut output = Vec::new();
        let end = loop {
            match timeout(Duration::from_secs(30), events.recv()).await.unwrap().unwrap() {
                Event::Output(data) => output.extend(data),
                Event::Reconnecting | Event::Reconnected => {}
                end => break end,
            }
        };
        assert!(String::from_utf8_lossy(&output).contains("deskconn-42"), "{output:?}");
        assert!(matches!(end, Event::Exited(3)), "{end:?}");
    }
}
