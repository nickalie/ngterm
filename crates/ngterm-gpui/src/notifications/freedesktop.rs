//! The banners on the freedesktop notification server (`org.freedesktop.Notifications` on the session bus) through
//! `zbus`: the banner task, the server's signals and the banners on screen (the module docs of
//! [`super`](crate::notifications) say how).

use futures_util::future::Either;
use futures_util::stream::StreamExt as _;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use zbus::message::Type as MessageType;
use zbus::names::OwnedUniqueName;
use zbus::zvariant::Value;
use zbus::{Connection, MatchRule, Message, MessageStream};

use super::{APP_NAME, Banner, Click};

/// The action key a click on a banner's body invokes.
const DEFAULT_ACTION: &str = "default";

/// The hint naming the desktop entry a banner belongs to.
const DESKTOP_ENTRY_HINT: &str = "desktop-entry";

/// The notification server's bus name, object and interface.
const SERVER: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";

/// The bus itself, which says who owns a name (`GetNameOwner`), and its answer when nobody does.
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
const NO_OWNER: &str = "org.freedesktop.DBus.Error.NameHasNoOwner";

/// What the server says of a banner it shows.
#[derive(Debug, PartialEq, Eq)]
enum Signal {
    /// The token for the click that follows (`ActivationToken`).
    Token { id: u32, token: String },
    /// The user clicked the banner or one of its actions (`ActionInvoked`).
    Invoked { id: u32 },
    /// The banner is gone: it expired, the user dismissed it, or it was withdrawn (`NotificationClosed`).
    Closed { id: u32 },
}

impl Signal {
    /// The signal `message` carries when `server` sent it; `None` for anything else.
    fn of(message: &Message, server: Option<&OwnedUniqueName>) -> Option<Self> {
        let header = message.header();
        let server = server?;
        if header.message_type() != MessageType::Signal || header.sender() != Some(&**server) {
            return None;
        }
        let body = message.body();
        match header.member()?.as_str() {
            "ActivationToken" => {
                let (id, token) = body.deserialize::<(u32, String)>().ok()?;
                Some(Self::Token { id, token })
            }
            "ActionInvoked" => {
                let (id, _key) = body.deserialize::<(u32, String)>().ok()?;
                Some(Self::Invoked { id })
            }
            "NotificationClosed" => {
                let (id, _reason) = body.deserialize::<(u32, u32)>().ok()?;
                Some(Self::Closed { id })
            }
            _ => None,
        }
    }
}

/// The banner task's session-bus connection: the server's signals it reads (`signals`) and those that arrived while a
/// call waited for its reply (`early`), handled next.
struct Bus {
    connection: Connection,
    signals: MessageStream,
    early: VecDeque<Message>,
}

impl Bus {
    async fn open() -> zbus::Result<Self> {
        let connection = Connection::session().await?;
        let rule = MatchRule::builder()
            .msg_type(MessageType::Signal)
            .interface(INTERFACE)?
            .path(PATH)?
            .build();
        let signals = MessageStream::for_match_rule(rule, &connection, None).await?;
        Ok(Self {
            connection,
            signals,
            early: VecDeque::new(),
        })
    }

    /// The next of the server's signals, `None` once the connection is gone.
    async fn signal(&mut self) -> Option<Message> {
        if let Some(early) = self.early.pop_front() {
            return Some(early);
        }
        loop {
            match self.signals.next().await? {
                Ok(message) => return Some(message),
                Err(error) => tracing::debug!(%error, "notify: an unreadable message on the bus"),
            }
        }
    }

    /// Waits for `reply` while still reading the signals into `early`: a connection whose signal queue fills stops
    /// reading its socket, the reply included.
    async fn reply(
        signals: &mut MessageStream,
        early: &mut VecDeque<Message>,
        reply: impl Future<Output = zbus::Result<Message>>,
    ) -> zbus::Result<Message> {
        let mut reply = std::pin::pin!(reply);
        loop {
            match futures_util::future::select(reply.as_mut(), signals.next()).await {
                Either::Left((reply, _)) => return reply,
                Either::Right((Some(Ok(message)), _)) => early.push_back(message),
                Either::Right((Some(Err(_)), _)) => {}
                Either::Right((None, _)) => return reply.await,
            }
        }
    }

    /// Posts a banner in place of banner `replaces` (0 for none): its id, and the server's unique name.
    async fn notify(
        &mut self,
        replaces: u32,
        title: &str,
        body: &str,
        app_id: &str,
    ) -> zbus::Result<(u32, Option<OwnedUniqueName>)> {
        let Self {
            connection,
            signals,
            early,
        } = self;
        let actions = vec![DEFAULT_ACTION, ""];
        let hints = HashMap::from([(DESKTOP_ENTRY_HINT, Value::from(app_id))]);
        // the server's own expiry
        let expire_timeout = -1_i32;
        // the icon by the theme name `resources/icons` installs, the default app id whatever this instance runs under:
        // servers that skip the desktop entry (dunst, mako) read it here
        let icon = crate::instance::DEFAULT_APP_ID;
        let arguments = (APP_NAME, replaces, icon, title, body, actions, hints, expire_timeout);
        let call = connection.call_method(Some(SERVER), PATH, Some(INTERFACE), "Notify", &arguments);
        let reply = Self::reply(signals, early, call).await?;
        let server = reply
            .header()
            .sender()
            .map(|name| OwnedUniqueName::from(name.to_owned()));
        Ok((reply.body().deserialize::<u32>()?, server))
    }

    /// The unique name that owns the server's name now, `None` when nobody does.
    async fn owner(&mut self) -> zbus::Result<Option<OwnedUniqueName>> {
        let Self {
            connection,
            signals,
            early,
        } = self;
        let arguments = (SERVER,);
        let call = connection.call_method(Some(BUS), BUS_PATH, Some(BUS), "GetNameOwner", &arguments);
        match Self::reply(signals, early, call).await {
            Ok(reply) => Ok(Some(reply.body().deserialize::<OwnedUniqueName>()?)),
            Err(zbus::Error::MethodError(name, ..)) if name.as_str() == NO_OWNER => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Withdraws banner `id`.
    async fn close(&mut self, id: u32) -> zbus::Result<()> {
        let Self {
            connection,
            signals,
            early,
        } = self;
        let arguments = (id,);
        let call = connection.call_method(Some(SERVER), PATH, Some(INTERFACE), "CloseNotification", &arguments);
        Self::reply(signals, early, call).await.map(drop)
    }
}

/// The banners on screen: their server ids by identity, the tokens the server sent for them, and the server that
/// posted them (whose signals alone count).
#[derive(Default)]
struct Shown {
    ids: HashMap<String, u32>,
    tokens: HashMap<u32, String>,
    server: Option<OwnedUniqueName>,
}

impl Shown {
    /// Forgets banner `id`: the identity it was posted under, if it is still that identity's banner.
    fn forget(&mut self, id: u32) -> Option<String> {
        self.tokens.remove(&id);
        let identity = self.ids.iter().find(|(_, known)| **known == id)?.0.clone();
        self.ids.remove(&identity);
        Some(identity)
    }

    /// Applies the server's signal `message`: the click it reports, if any.
    fn signal(&mut self, message: &Message) -> Option<Click> {
        match Signal::of(message, self.server.as_ref())? {
            Signal::Token { id, token } => {
                if self.ids.values().any(|known| *known == id) {
                    self.tokens.insert(id, token);
                }
                None
            }
            Signal::Invoked { id } => {
                let token = self.tokens.remove(&id);
                let identity = self.forget(id)?;
                Some(Click { identity, token })
            }
            Signal::Closed { id } => {
                self.forget(id);
                None
            }
        }
    }

    /// The banners are `server`'s: a server other than the one that posted the banners known forgets them all, since
    /// their ids are that server's (one that restarted numbers its banners afresh, so they would name another's).
    fn served_by(&mut self, server: Option<OwnedUniqueName>) {
        if self.server == server {
            return;
        }
        if !self.ids.is_empty() {
            tracing::debug!(
                banners = self.ids.len(),
                "notify: the server changed; its banners are forgotten"
            );
        }
        self.ids.clear();
        self.tokens.clear();
        self.server = server;
    }

    /// Makes sure the banner `identity` names, if any, is still the current server's before its id is reused: the
    /// bus says who owns the server's name now. A bus that does not answer leaves the banners known.
    async fn confirm(&mut self, bus: &mut Bus, identity: &str) {
        if !self.ids.contains_key(identity) {
            return;
        }
        match bus.owner().await {
            Ok(owner) => self.served_by(owner),
            Err(error) => tracing::debug!(%error, "notify: the server's owner is unknown"),
        }
    }

    /// Posts `banner` (a send) on `bus`, in place of its identity's banner on screen. A post that fails leaves that
    /// banner known, so a later clear still withdraws it and its click still reveals its pane.
    async fn send(&mut self, bus: &mut Bus, banner: &Banner, app_id: &str) {
        let Banner::Send { identity, title, body } = banner else {
            return;
        };
        self.confirm(bus, identity).await;
        let replaces = self.ids.get(identity).copied().unwrap_or(0);
        match bus.notify(replaces, title, body, app_id).await {
            Ok((id, server)) => {
                self.served_by(server);
                if replaces != 0 && replaces != id {
                    self.tokens.remove(&replaces);
                }
                self.ids.insert(identity.clone(), id);
            }
            Err(error) => tracing::warn!(%error, "notify: the banner was not posted"),
        }
    }

    /// Withdraws banner `identity` from `bus`.
    async fn withdraw(&mut self, bus: &mut Bus, identity: &str) {
        self.confirm(bus, identity).await;
        let Some(id) = self.ids.remove(identity) else {
            return;
        };
        self.tokens.remove(&id);
        if let Err(error) = bus.close(id).await {
            tracing::debug!(%error, "notify: the banner was not withdrawn");
        }
    }

    /// Whether `banner` has anything to do on the server: every send, a withdrawal of a banner on screen.
    fn reaches_server(&self, banner: &Banner) -> bool {
        match banner {
            Banner::Send { .. } => true,
            Banner::Withdraw { identity } => self.ids.contains_key(identity),
        }
    }
}

/// What the banner task handles next.
enum Next {
    Banner(Banner),
    Signal(Message),
    /// The connection is gone: the next banner opens another.
    Lost,
    /// Every sink is gone.
    Done,
}

/// The banner task: posts and withdraws `banners` in order on one session-bus connection (opened with the first
/// banner, again after it drops), each identity's repeat replacing its banner, and calls `clicked` with each banner
/// the user clicked. Ends when every sink is gone.
pub async fn run(banners: async_channel::Receiver<Banner>, app_id: String, clicked: impl Fn(Click) + Send + 'static) {
    let mut shown = Shown::default();
    let mut bus: Option<Bus> = None;
    loop {
        match next(&mut bus, &banners).await {
            Next::Signal(message) => {
                if let Some(click) = shown.signal(&message) {
                    tracing::info!(identity = %click.identity, token = click.token.is_some(), "notify: banner clicked");
                    clicked(click);
                }
            }
            Next::Lost => {
                tracing::debug!("notify: the session-bus connection is gone");
                bus = None;
            }
            Next::Banner(banner) => {
                if !shown.reaches_server(&banner) {
                    continue;
                }
                let Some(open) = connected(&mut bus).await else {
                    continue;
                };
                match &banner {
                    Banner::Send { .. } => shown.send(open, &banner, &app_id).await,
                    Banner::Withdraw { identity } => shown.withdraw(open, identity).await,
                }
            }
            Next::Done => return,
        }
    }
}

/// The next banner or server signal: a signal ready first is handled first, so a click seen before the next banner
/// forgets its banner first.
async fn next(bus: &mut Option<Bus>, banners: &async_channel::Receiver<Banner>) -> Next {
    let Some(open) = bus.as_mut() else {
        return banners.recv().await.map_or(Next::Done, Next::Banner);
    };
    let signal = std::pin::pin!(open.signal());
    let banner = std::pin::pin!(banners.recv());
    match futures_util::future::select(signal, banner).await {
        Either::Left((Some(message), _)) => Next::Signal(message),
        Either::Left((None, _)) => Next::Lost,
        Either::Right((Ok(banner), _)) => Next::Banner(banner),
        Either::Right((Err(_), _)) => Next::Done,
    }
}

/// The task's connection, opened when there is none; `None` when the bus is out of reach (the banner is dropped).
async fn connected(bus: &mut Option<Bus>) -> Option<&mut Bus> {
    if bus.is_none() {
        match Bus::open().await {
            Ok(open) => *bus = Some(open),
            Err(error) => {
                tracing::warn!(%error, "notify: no session bus to post banners on");
                return None;
            }
        }
    }
    bus.as_mut()
}
