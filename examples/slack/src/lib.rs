//! A Slack client, modeled on mtui's mslack. Live updates come over a
//! WebSocket (Socket Mode with an app token, otherwise RTM); without one it
//! polls the open conversation every 5 s.
#![no_std]
extern crate alloc;

mod fmt;

use alloc::collections::{BTreeMap, BTreeSet};
use fmt::Names;
use qa_sdk::json::Doc;
use qa_sdk::net::{self, Client, Response, Ws};
use qa_sdk::*;

const API: &str = "https://slack.com/api/";
const TICK_MS: u32 = 5_000;
/// Messages kept per conversation.
const KEEP: usize = 400;
const CONFIG: &str = "config";

#[derive(Clone, Copy, PartialEq)]
enum Fetch {
    Initial,
    Newer,
    Refresh,
    Older,
}

#[derive(Clone, PartialEq)]
enum Tag {
    Auth,
    Me,
    Chans,
    Users,
    User(String),
    Counts,
    History(String, Fetch),
    Replies(String, String),
    Posted(String),
    Permalink,
    Deleted(String, String),
    Done,
    Mark,
    Connect,
}

impl Tag {
    /// Requests with a key are never in flight twice.
    fn key(&self) -> Option<String> {
        match self {
            Tag::History(c, _) => Some(cat(&["h:", c])),
            Tag::Replies(_, ts) => Some(cat(&["r:", ts])),
            Tag::User(u) => Some(cat(&["u:", u])),
            Tag::Counts => Some(String::from("counts")),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq)]
struct Msg {
    ts: String,
    user: String,
    bot: String,
    text: String,
    sub: String,
    thread_ts: String,
    replies: u32,
    reactions: Vec<(String, u32, bool)>,
    files: Vec<String>,
    edited: bool,
}

impl Msg {
    fn from(v: Val, me: &str) -> Msg {
        let mut text = String::from(v.get("text").str());
        if text.is_empty() {
            let a = v.path("attachments.0");
            text = String::from(a.get("fallback").opt_str().or(a.get("text").opt_str()).unwrap_or(""));
        }
        let s = |k: &str| String::from(v.get(k).str());
        Msg {
            ts: s("ts"),
            user: s("user"),
            bot: String::from(v.path("bot_profile.name").opt_str().or(v.get("username").opt_str()).unwrap_or("")),
            text,
            sub: s("subtype"),
            thread_ts: s("thread_ts"),
            replies: v.get("reply_count").num() as u32,
            reactions: v
                .get("reactions")
                .iter()
                .map(|r| (String::from(r.get("name").str()), r.get("count").num() as u32, r.get("users").iter().any(|u| u.str() == me)))
                .collect(),
            files: v.get("files").iter().map(|f| String::from(f.get("name").opt_str().or(f.get("title").opt_str()).unwrap_or("file"))).collect(),
            edited: !v.get("edited").is_null(),
        }
    }

    fn who(&self) -> &str {
        if self.user.is_empty() {
            &self.bot
        } else {
            &self.user
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Public,
    Private,
    Mpim,
    Im,
}

struct Chan {
    id: String,
    name: String,
    kind: Kind,
    user: String,
    msgs: Vec<Msg>,
    loaded: bool,
    more: bool,
    unread: bool,
    mentions: u32,
    marked: String,
    row: Node,
    label: Node,
    badge: Node,
}

impl Chan {
    fn label(&self) -> String {
        let p = match self.kind {
            Kind::Public => "# ",
            Kind::Private => "◦ ",
            _ => "@ ",
        };
        cat(&[p, &self.name])
    }
}

struct Thread {
    chan: String,
    ts: String,
    msgs: Vec<Msg>,
}

/// What a button inside a message does.
#[derive(Clone)]
enum Act {
    /// (in thread view, message ts, emoji name)
    React(bool, String, String),
    Thread(String),
    /// A message text's link spans: (span index, URL).
    Links(Vec<(u32, String)>),
    Older,
}

/// What a context menu item does.
#[derive(Clone, Copy, PartialEq)]
enum MenuAct {
    Thread,
    React,
    CopyText,
    CopyLink,
    MarkUnread,
    Delete,
    ChanRead,
    ChanOpen,
    None,
}

/// Reactions offered by the picker.
/// Only ones the runtime's fonts can draw (see `fmt::reaction`).
const PICKS: [&str; 12] = ["heart", "white_check_mark", "x", "star", "smile", "frowning", "point_up", "warning", "zap", "coffee", "sunny", "question"];

/// Names for rendering, and the reverse maps for outgoing mentions.
#[derive(Default)]
struct Dir {
    users: Names,
    /// lowercase handle → user id
    handles: Names,
    chans: Names,
    /// lowercase channel name → id
    chan_ids: Names,
    me: String,
}

impl Dir {
    fn name_of(&self, m: &Msg) -> String {
        if m.user.is_empty() {
            return String::from(if m.bot.is_empty() { "bot" } else { &m.bot });
        }
        self.users.get(&m.user).cloned().unwrap_or_else(|| m.user.clone())
    }

    fn add_user(&mut self, u: Val) {
        let p = u.get("profile");
        let name = String::from(p.get("display_name").opt_str().or(p.get("real_name").opt_str()).or(u.get("name").opt_str()).unwrap_or("?"));
        let id = String::from(u.get("id").str());
        if id.is_empty() {
            return;
        }
        if let Some(h) = u.get("name").opt_str() {
            self.handles.insert(h.to_lowercase(), id.clone());
        }
        self.handles.entry(name.replace(' ', "").to_lowercase()).or_insert(id.clone());
        self.users.insert(id, name);
    }
}

struct Row {
    ts: String,
    node: Node,
    head: bool,
    acts: Vec<Node>,
}

/// A rendered message list (the channel or the thread).
#[derive(Default)]
struct View {
    host: Node,
    list: Node,
    rows: Vec<Row>,
    /// The previous message's author and local time, for grouping.
    last: Option<(String, i64)>,
    day: Option<i64>,
    older: Node,
    /// The day separator above the oldest row.
    first_sep: Node,
    /// The first node of the list after the "Load older" button, where older rows go.
    top: Node,
    /// Each row's context menu.
    menu: Node,
}

/// Tokens live in the runtime's secret store; the rest in the data folder.
#[derive(Default)]
struct Config {
    token: String,
    cookie: String,
    apptoken: String,
    last: String,
    /// Another API base, for testing against a mock server.
    api: String,
}

const SECRETS: [&str; 3] = ["token", "cookie", "apptoken"];

impl Config {
    fn load() -> Config {
        let mut c = Config::default();
        let data = fs::read(fs::APP_DIR, CONFIG).unwrap_or_default();
        let mut migrate = false;
        for line in core::str::from_utf8(&data).unwrap_or("").lines() {
            let (k, v) = line.split_once(' ').unwrap_or((line, ""));
            let v = String::from(v.trim());
            match k {
                "last" => c.last = v,
                "api" => c.api = v,
                // Tokens from an older version, which kept them here.
                k if SECRETS.contains(&k) => {
                    *c.secret(k) = v;
                    migrate = true;
                }
                _ => {}
            }
        }
        for k in SECRETS {
            if let Some(v) = secret::get(k) {
                *c.secret(k) = v;
            }
        }
        if migrate {
            c.save();
        }
        c
    }

    fn secret(&mut self, k: &str) -> &mut String {
        match k {
            "token" => &mut self.token,
            "cookie" => &mut self.cookie,
            _ => &mut self.apptoken,
        }
    }

    fn save(&mut self) {
        for k in SECRETS {
            let v = self.secret(k).clone();
            secret::set(k, &v);
        }
        let mut s = String::new();
        for (k, v) in [("last", &self.last), ("api", &self.api)] {
            if !v.is_empty() {
                s.push_str(&cat(&[k, " ", v, "\n"]));
            }
        }
        let _ = fs::write(fs::APP_DIR, CONFIG, s.as_bytes());
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Live {
    Off,
    Connecting,
    On,
    /// The token can't use Socket Mode or RTM: poll for good.
    Never,
}

struct Slack {
    cfg: Config,
    dir: Dir,
    chans: Vec<Chan>,
    cur: Option<String>,
    thread: Option<Thread>,
    team: String,
    team_id: String,

    // network
    net: Client<Tag>,
    /// users.conversations pages, until the last one arrives.
    acc: Vec<Doc>,
    pages: u32,
    users_done: bool,
    mark_ok: bool,
    counts_ok: bool,
    last_chans: i64,

    // live updates
    live: Live,
    ws: Option<Ws>,
    rtm: bool,
    backoff: u32,
    reconnect_timer: u32,
    ping_timer: u32,
    ping_id: u32,
    tick_timer: u32,
    ticks: u32,
    names_timer: u32,
    note_timer: u32,

    // login screen
    login: Node,
    f_token: Node,
    f_cookie: Node,
    f_app: Node,
    login_err: Node,
    connect: Node,
    help: Node,

    // main screen
    main: Node,
    team_text: Node,
    status: Node,
    signout: Node,
    side_scroll: Node,
    side_list: Node,
    title: Node,
    note: Node,
    note_row: Node,
    input: Node,
    send: Node,
    mview: View,
    tsep: Node,
    tpane: Node,
    tclose: Node,
    tinput: Node,
    tsend: Node,
    tview: View,
    acts: BTreeMap<u32, Act>,
    draft: String,
    tdraft: String,
    mscroll: Node,

    // context menus
    msg_menu: Node,
    chan_pop: Node,
    chan_menu: Node,
    react_pop: Node,
    picks: Vec<(Node, &'static str)>,
    menu_acts: Vec<MenuAct>,
    /// The message a menu or the picker is for: (in the thread, channel, ts).
    target: Option<(bool, String, String)>,
    chan_target: Option<String>,
}

impl App for Slack {
    fn start(ui: &mut Ui) -> Self {
        let root = ui.grid().rows(&[fr(1.0)]).root();

        let login = ui.grid().at(0, 0).pad(24.0).gap(10.0).max_w(560.0).align_x(CENTER).align_y(CENTER).hidden(true).add_to(root);
        ui.text("Slack").style("title").add_to(login);
        ui.text("Sign in with a User OAuth Token (xoxp-…) from a Slack app, or with a browser session token (xoxc-…) and its d cookie (xoxd-…). An app-level token (xapp-…) adds live updates through Socket Mode.")
            .variant("muted")
            .add_to(login);
        ui.text("Token").style("caption").add_to(login);
        let f_token = ui.input().secret(true).placeholder("xoxp-… or xoxc-…").add_to(login);
        ui.text("Cookie (only with an xoxc- token)").style("caption").add_to(login);
        let f_cookie = ui.input().secret(true).placeholder("xoxd-…").add_to(login);
        ui.text("App-level token (optional, for live updates)").style("caption").add_to(login);
        let f_app = ui.input().secret(true).placeholder("xapp-…").add_to(login);
        let login_err = ui.text("").variant("danger").hidden(true).add_to(login);
        let bar = ui.grid().cols(&[auto(), fr(1.0), auto()]).gap(8.0).add_to(login);
        let help = ui.button("Create a Slack app ↗").variant("flat").add_to(bar);
        let connect = ui.button("Connect").variant("primary").at(0, 2).add_to(bar);

        let main = ui.grid().at(0, 0).cols(&[px(250.0).min(170.0), px(1.0), fr(1.0).min(320.0), px(1.0), auto()]).rows(&[fr(1.0)]).hidden(true).add_to(root);

        let side = ui.grid().rows(&[auto(), fr(1.0)]).priority(0).add_to(main);
        let head = ui.grid().cols(&[fr(1.0), auto()]).pad4(12.0, 10.0, 8.0, 16.0).gap(2.0).add_to(side);
        let team_text = ui.text("Slack").style("heading").wrap(1).add_to(head);
        let signout = ui.button("Sign out").variant("flat").add_to(head);
        let status = ui.text("connecting…").style("caption").variant("muted").add_to(head);
        let side_scroll = ui.scroll().add_to(side);
        let side_list = ui.grid().add_to(side_scroll);
        ui.divider().add_to(main);

        let center = ui.grid().rows(&[auto(), auto(), fr(1.0), auto(), auto()]).priority(1).add_to(main);
        let mhead = ui.grid().pad4(12.0, 16.0, 12.0, 16.0).add_to(center);
        let title = ui.text("").style("heading").wrap(1).add_to(mhead);
        ui.divider().add_to(center);
        let mscroll = ui.scroll().stick_end(true).add_to(center);
        let mhost = ui.grid().pad4(4.0, 0.0, 8.0, 0.0).add_to(mscroll);
        let note_row = ui.grid().pad4(4.0, 16.0, 0.0, 16.0).hidden(true).add_to(center);
        let note = ui.text("").style("caption").variant("muted").add_to(note_row);
        let comp = ui.grid().cols(&[fr(1.0), auto()]).gap(8.0).pad4(4.0, 16.0, 16.0, 16.0).add_to(center);
        let input = ui.input().placeholder("Message").add_to(comp);
        let send = ui.button("Send").variant("primary").add_to(comp);

        let tsep = ui.divider().hidden(true).add_to(main);
        let tpane = ui.grid().rows(&[auto(), auto(), fr(1.0), auto()]).min_w(320.0).max_w(420.0).hidden(true).add_to(main);
        let thead = ui.grid().cols(&[fr(1.0), auto()]).pad4(10.0, 10.0, 10.0, 16.0).add_to(tpane);
        ui.text("Thread").style("heading").add_to(thead);
        let tclose = ui.button("✕").variant("flat").tooltip("Close thread (Esc)").add_to(thead);
        ui.divider().add_to(tpane);
        let tscroll = ui.scroll().stick_end(true).add_to(tpane);
        let thost = ui.grid().pad4(4.0, 0.0, 8.0, 0.0).add_to(tscroll);
        let tcomp = ui.grid().cols(&[fr(1.0), auto()]).gap(8.0).pad4(4.0, 16.0, 16.0, 16.0).add_to(tpane);
        let tinput = ui.input().placeholder("Reply…").add_to(tcomp);
        let tsend = ui.button("Reply").add_to(tcomp);

        let msg_pop = ui.popup().detached();
        let msg_menu = ui.menu("").add_to(msg_pop);
        let chan_pop = ui.popup().detached();
        let chan_menu = ui.menu("").add_to(chan_pop);
        let react_pop = ui.popup().detached();
        let rgrid = ui.grid().flow(true).gap(2.0).max_w(232.0).add_to(react_pop);
        let mut picks = Vec::new();
        for name in PICKS {
            let b = ui.button(&fmt::reaction(name)).variant("flat").tooltip(name).add_to(rgrid);
            picks.push((b, name));
        }

        let tick_timer = interval(TICK_MS);
        let mut s = Slack {
            cfg: Config::load(),
            dir: Dir::default(),
            chans: Vec::new(),
            cur: None,
            thread: None,
            team: String::new(),
            team_id: String::new(),
            net: Client::new(),
            acc: Vec::new(),
            pages: 0,
            users_done: false,
            mark_ok: true,
            counts_ok: true,
            last_chans: 0,
            live: Live::Off,
            ws: None,
            rtm: false,
            backoff: 1,
            reconnect_timer: 0,
            ping_timer: 0,
            ping_id: 0,
            tick_timer,
            ticks: 0,
            names_timer: 0,
            note_timer: 0,
            login,
            f_token,
            f_cookie,
            f_app,
            login_err,
            connect,
            help,
            main,
            team_text,
            status,
            signout,
            side_scroll,
            side_list,
            title,
            note,
            note_row,
            input,
            send,
            mview: View { host: mhost, menu: msg_pop, ..View::default() },
            tsep,
            tpane,
            tclose,
            tinput,
            tsend,
            tview: View { host: thost, menu: msg_pop, ..View::default() },
            acts: BTreeMap::new(),
            draft: String::new(),
            tdraft: String::new(),
            mscroll,
            msg_menu,
            chan_pop,
            chan_menu,
            react_pop,
            picks,
            menu_acts: Vec::new(),
            target: None,
            chan_target: None,
        };
        if s.cfg.token.is_empty() {
            s.show_login(ui, "");
        } else {
            s.sign_in(ui);
        }
        s
    }

    fn event(&mut self, ui: &mut Ui, ev: Event) {
        if let Some(r) = self.net.handle(&ev) {
            return self.on_http(ui, r);
        }
        match ev {
            Event::Ws { ws, kind, data } if self.ws.is_some_and(|w| w.0 == ws) => self.on_ws(ui, kind, data),
            Event::Timer(id) => self.on_timer(ui, id),
            Event::Click(id) if id == self.connect => self.connect_clicked(ui),
            Event::Submit(id, _) if id == self.f_token || id == self.f_cookie || id == self.f_app => self.connect_clicked(ui),
            Event::Change(id, v) if id == self.f_token => self.cfg.token = String::from(v.trim()),
            Event::Change(id, v) if id == self.f_cookie => self.cfg.cookie = String::from(v.trim()),
            Event::Change(id, v) if id == self.f_app => self.cfg.apptoken = String::from(v.trim()),
            Event::Click(id) if id == self.help => {
                open_url("https://api.slack.com/apps");
            }
            Event::Click(id) if id == self.signout => self.sign_out(ui),
            Event::Change(id, v) if id == self.input => self.draft = String::from(v),
            Event::Change(id, v) if id == self.tinput => self.tdraft = String::from(v),
            Event::Submit(id, _) | Event::Click(id) if id == self.input || id == self.send => self.send(ui, false),
            Event::Submit(id, _) | Event::Click(id) if id == self.tinput || id == self.tsend => self.send(ui, true),
            Event::Click(id) if id == self.tclose => self.close_thread(ui),
            Event::ContextMenu { node, .. } => self.on_context(ui, node),
            Event::Select(id, i) if id == self.msg_menu || id == self.chan_menu => self.on_menu(ui, i),
            Event::Click(id) if self.picks.iter().any(|p| p.0 == id) => {
                let name = self.picks.iter().find(|p| p.0 == id).map(|p| p.1).unwrap_or("");
                ui.close(self.react_pop);
                if let Some((thread, _, ts)) = self.target.clone() {
                    self.act(ui, Act::React(thread, ts, String::from(name)));
                }
            }
            Event::NearStart(id) if id == self.mscroll => {
                if let Some(ci) = self.cur.as_deref().and_then(|id| self.idx(id)) {
                    if self.chans[ci].loaded && self.chans[ci].more {
                        self.load_history(ci, Fetch::Older);
                    }
                }
            }
            // Sidebar rows are tagged with their index + 1.
            Event::Click(_) if ui.tag() > 0 => {
                if let Some(c) = self.chans.get(ui.tag() as usize - 1) {
                    let cid = c.id.clone();
                    self.open(ui, &cid);
                }
            }
            Event::Click(id) => {
                if let Some(a) = self.acts.get(&id.0).cloned() {
                    self.act(ui, a);
                }
            }
            Event::Link(id, i) => {
                if let Some(Act::Links(l)) = self.acts.get(&id.0) {
                    if let Some((_, url)) = l.iter().find(|l| l.0 == i) {
                        open_url(url);
                    }
                }
            }
            Event::Key { sym, mods, .. } => {
                if sym == key::ESCAPE && self.thread.is_some() {
                    self.close_thread(ui);
                } else if mods & key::ALT != 0 && (sym == key::UP || sym == key::DOWN) {
                    self.switch_rel(ui, if sym == key::UP { -1 } else { 1 });
                }
            }
            _ => {}
        }
    }
}

impl Slack {
    // ---------- screens ----------

    fn show_login(&mut self, ui: &mut Ui, err: &str) {
        ui.hide(self.main, true);
        ui.hide(self.login, false);
        ui.set_value(self.f_token, &self.cfg.token);
        ui.set_value(self.f_cookie, &self.cfg.cookie);
        ui.set_value(self.f_app, &self.cfg.apptoken);
        ui.set_text(self.login_err, err);
        ui.hide(self.login_err, err.is_empty());
        ui.focus(self.f_token);
        set_title("Slack");
    }

    fn connect_clicked(&mut self, ui: &mut Ui) {
        if self.cfg.token.is_empty() {
            return self.show_login(ui, "Paste a token first.");
        }
        if self.cfg.token.starts_with("xoxc-") && self.cfg.cookie.is_empty() {
            return self.show_login(ui, "An xoxc- token needs its d cookie (xoxd-…).");
        }
        ui.set_text(self.login_err, "Connecting…");
        ui.hide(self.login_err, false);
        self.call(Tag::Auth, "auth.test", &[]);
    }

    fn sign_in(&mut self, ui: &mut Ui) {
        ui.hide(self.login, true);
        ui.hide(self.main, false);
        ui.set_text(self.title, "Loading…");
        self.call(Tag::Auth, "auth.test", &[]);
    }

    fn sign_out(&mut self, ui: &mut Ui) {
        self.net.cancel(|_| true);
        if let Some(w) = self.ws.take() {
            w.close();
        }
        for t in [self.reconnect_timer, self.ping_timer] {
            if t != 0 {
                timer_cancel(t);
            }
        }
        self.reconnect_timer = 0;
        self.ping_timer = 0;
        self.live = Live::Off;
        self.close_thread(ui);
        self.chans.clear();
        self.cur = None;
        self.dir = Dir::default();
        self.users_done = false;
        self.mark_ok = true;
        self.counts_ok = true;
        clear_view(ui, &mut self.mview, &mut self.acts);
        ui.destroy(self.side_list);
        self.side_list = ui.grid().add_to(self.side_scroll);
        self.cfg = Config { api: core::mem::take(&mut self.cfg.api), ..Config::default() };
        self.cfg.save();
        self.show_login(ui, "");
    }

    fn set_status(&mut self, ui: &mut Ui) {
        let s = match self.live {
            Live::On => "● live",
            Live::Connecting => "connecting…",
            Live::Off | Live::Never => "○ polling",
        };
        ui.set_text(self.status, s);
    }

    /// A transient line above the composer.
    fn flash(&mut self, ui: &mut Ui, s: &str) {
        ui.set_text(self.note, s);
        ui.hide(self.note_row, false);
        if self.note_timer != 0 {
            timer_cancel(self.note_timer);
        }
        self.note_timer = timer(6_000);
    }

    // ---------- network ----------

    fn call(&mut self, tag: Tag, method: &'static str, params: &[(&str, &str)]) {
        self.call_as(tag, method, params, false);
    }

    fn call_as(&mut self, tag: Tag, method: &'static str, params: &[(&str, &str)], app_token: bool) {
        // Requests with a key are never in flight twice.
        if let Some(k) = tag.key() {
            if self.net.has(|t| t.key().as_ref() == Some(&k)) {
                return;
            }
        }
        let tok = if app_token { &self.cfg.apptoken } else { &self.cfg.token };
        let mut h = cat(&["Authorization: Bearer ", tok, "\nContent-Type: application/x-www-form-urlencoded\n"]);
        if !app_token && !self.cfg.cookie.is_empty() {
            h.push_str(&cat(&["Cookie: d=", &self.cfg.cookie, "\n"]));
        }
        let base = if self.cfg.api.is_empty() { API } else { self.cfg.api.as_str() };
        self.net.send("POST", &cat(&[base, method]), &h, net::form(params).as_bytes(), tag);
    }

    fn on_http(&mut self, ui: &mut Ui, r: Response<Tag>) {
        let doc = r.json();
        let v = doc.as_ref().map(|d| d.root());
        let res = match v {
            _ if r.status == 0 => Err(String::from(core::str::from_utf8(r.body).unwrap_or("network error"))),
            None => Err(cat(&["HTTP ", &itoa(r.status as i64)])),
            Some(v) if v.get("ok").bool() => Ok(v),
            Some(v) => Err(String::from(v.get("error").opt_str().unwrap_or("request failed"))),
        };
        self.on_reply(ui, r.tag, res);
    }

    fn on_reply(&mut self, ui: &mut Ui, tag: Tag, r: Result<Val, String>) {
        let v = match r {
            Ok(v) => v,
            Err(e) => return self.on_error(ui, tag, &e),
        };
        match tag {
            Tag::Auth => {
                self.dir.me = String::from(v.get("user_id").str());
                self.team = String::from(v.get("team").str());
                self.team_id = String::from(v.get("team_id").str());
                self.cfg.save();
                ui.hide(self.login, true);
                ui.hide(self.main, false);
                ui.set_text(self.team_text, &self.team);
                let me = self.dir.me.clone();
                self.call(Tag::Me, "users.info", &[("user", &me)]);
                self.refetch_chans();
                self.pages = 0;
                self.call(Tag::Users, "users.list", &[("limit", "500")]);
                self.connect_live(ui);
            }
            Tag::Me => {
                self.dir.add_user(v.get("user"));
                self.names_changed();
            }
            Tag::Chans => {
                let cursor = v.path("response_metadata.next_cursor").str();
                self.pages += 1;
                let more = !cursor.is_empty() && self.pages < 50;
                let cursor = String::from(cursor);
                self.acc.push(Doc { bytes: v.raw().to_vec() });
                if more {
                    self.call(Tag::Chans, "users.conversations", &[("types", "public_channel,private_channel,mpim,im"), ("exclude_archived", "true"), ("limit", "200"), ("cursor", &cursor)]);
                } else {
                    self.on_chans(ui);
                }
            }
            Tag::Users => {
                for u in v.get("members").iter() {
                    self.dir.add_user(u);
                }
                let cursor = v.path("response_metadata.next_cursor").str();
                if !cursor.is_empty() && self.dir.users.len() < 20_000 {
                    let cursor = String::from(cursor);
                    self.call(Tag::Users, "users.list", &[("limit", "500"), ("cursor", &cursor)]);
                } else {
                    self.users_done = true;
                    self.names_changed();
                    self.want_users();
                }
            }
            Tag::User(_) => {
                self.dir.add_user(v.get("user"));
                self.names_changed();
            }
            Tag::Counts => {
                for k in ["channels", "mpims", "ims"] {
                    for e in v.get(k).iter() {
                        let Some(ci) = self.idx(e.get("id").str()) else { continue };
                        if self.is_cur(ci) {
                            continue;
                        }
                        let c = &mut self.chans[ci];
                        c.unread = e.get("has_unreads").bool();
                        c.mentions = e.get("mention_count").num() as u32;
                        paint_chan(ui, c, false);
                    }
                }
            }
            Tag::History(id, f) => self.on_history(ui, &id, f, v),
            Tag::Replies(chan, ts) => self.on_replies(ui, &chan, &ts, v),
            Tag::Posted(chan) => {
                if let Some(ci) = self.idx(&chan) {
                    let m = Msg::from(v.get("message"), &self.dir.me);
                    self.new_msg(ui, ci, m);
                }
            }
            Tag::Connect => {
                let url = String::from(v.get("url").str());
                let headers = if self.rtm && !self.cfg.cookie.is_empty() { cat(&["Cookie: d=", &self.cfg.cookie, "\n"]) } else { String::new() };
                self.ws = Some(net::ws(&url, &headers));
            }
            Tag::Permalink => {
                clipboard::write(v.get("permalink").str());
                self.flash(ui, "Link copied");
            }
            Tag::Deleted(chan, ts) => {
                if let Some(ci) = self.idx(&chan) {
                    self.remove_msg(ui, ci, &ts);
                }
            }
            Tag::Done | Tag::Mark => {}
        }
    }

    fn on_error(&mut self, ui: &mut Ui, tag: Tag, e: &str) {
        match tag {
            Tag::Auth => {
                let msg = cat(&["Sign-in failed: ", e]);
                self.show_login(ui, &msg);
            }
            Tag::Counts => self.counts_ok = false,
            Tag::Mark => self.mark_ok = !(e.contains("scope") || e.contains("not_allowed")),
            Tag::Connect => {
                if fatal(e) {
                    // Try RTM before giving up on an app token that doesn't work.
                    if !self.rtm && !self.cfg.apptoken.is_empty() {
                        self.rtm = true;
                        self.call(Tag::Connect, "rtm.connect", &[]);
                        return;
                    }
                    self.live = Live::Never;
                    self.set_status(ui);
                } else {
                    self.reconnect_later(ui);
                }
            }
            Tag::Chans if !self.acc.is_empty() => self.on_chans(ui),
            Tag::User(_) | Tag::Me => {}
            Tag::Users => {
                self.users_done = true;
                self.names_changed();
            }
            _ => {
                let msg = cat(&["Slack: ", e]);
                self.flash(ui, &msg);
            }
        }
    }

    fn on_timer(&mut self, ui: &mut Ui, id: u32) {
        if id == self.tick_timer {
            self.tick(ui);
        } else if id == self.reconnect_timer {
            self.reconnect_timer = 0;
            self.connect_live(ui);
        } else if id == self.ping_timer {
            if let (Some(w), true) = (self.ws, self.rtm) {
                self.ping_id += 1;
                w.send_text(&cat(&["{\"id\":", &itoa(self.ping_id as i64), ",\"type\":\"ping\"}"]));
            }
        } else if id == self.names_timer {
            self.names_timer = 0;
            self.redraw_names(ui);
        } else if id == self.note_timer {
            self.note_timer = 0;
            ui.hide(self.note_row, true);
        }
    }

    /// Every 5 s: poll while there is no live connection, and resync now and then.
    fn tick(&mut self, ui: &mut Ui) {
        if self.dir.me.is_empty() {
            return;
        }
        self.ticks += 1;
        let live = self.live == Live::On;
        let t = self.ticks;
        let every = move |n: u32| t % n == 0;
        if let Some(ci) = self.cur.as_deref().and_then(|id| self.idx(id)) {
            if live {
                if every(60) {
                    self.load_history(ci, Fetch::Refresh);
                }
            } else if every(6) {
                self.load_history(ci, Fetch::Refresh);
            } else {
                self.load_history(ci, Fetch::Newer);
            }
        }
        if !live || every(60) {
            self.load_thread();
        }
        if self.counts_ok && (if live { every(60) } else { every(12) }) {
            self.call(Tag::Counts, "client.counts", &[]);
        }
        let _ = ui;
    }

    // ---------- live updates ----------

    fn connect_live(&mut self, ui: &mut Ui) {
        if self.live == Live::Never || self.ws.is_some() {
            return;
        }
        self.live = Live::Connecting;
        self.set_status(ui);
        if !self.cfg.apptoken.is_empty() && !self.rtm {
            self.call_as(Tag::Connect, "apps.connections.open", &[], true);
        } else {
            self.rtm = true;
            self.call(Tag::Connect, "rtm.connect", &[]);
        }
    }

    fn reconnect_later(&mut self, ui: &mut Ui) {
        self.live = Live::Off;
        self.set_status(ui);
        if self.reconnect_timer == 0 {
            self.reconnect_timer = timer(self.backoff * 1000);
            self.backoff = (self.backoff * 2).min(60);
        }
    }

    fn on_ws(&mut self, ui: &mut Ui, kind: u8, data: &[u8]) {
        match kind {
            net::WS_TEXT => {
                let Some(doc) = json::parse(data) else { return };
                let m = doc.root();
                if let (Some(id), Some(w)) = (m.get("envelope_id").opt_str(), self.ws) {
                    w.send_text(&cat(&["{\"envelope_id\":", &json::quote(id), "}"]));
                }
                match m.get("type").str() {
                    "hello" => {
                        self.live = Live::On;
                        self.backoff = 1;
                        self.set_status(ui);
                        if self.rtm && self.ping_timer == 0 {
                            self.ping_timer = interval(30_000);
                        }
                        // Fill anything missed while disconnected.
                        if let Some(ci) = self.cur.as_deref().and_then(|id| self.idx(id)) {
                            self.load_history(ci, Fetch::Newer);
                        }
                    }
                    "disconnect" | "goodbye" => {
                        if let Some(w) = self.ws.take() {
                            w.close();
                        }
                        self.backoff = 1;
                        self.reconnect_later(ui);
                    }
                    "events_api" => self.on_event(ui, m.path("payload.event")),
                    "pong" | "reconnect_url" | "" => {}
                    _ if self.rtm && m.get("reply_to").is_null() => self.on_event(ui, m),
                    _ => {}
                }
            }
            net::WS_CLOSED | net::WS_ERROR => {
                self.ws = None;
                if self.ping_timer != 0 {
                    timer_cancel(self.ping_timer);
                    self.ping_timer = 0;
                }
                self.reconnect_later(ui);
            }
            _ => {}
        }
    }

    fn on_event(&mut self, ui: &mut Ui, ev: Val) {
        let me = self.dir.me.clone();
        match ev.get("type").str() {
            "message" => {
                let Some(ci) = self.idx(ev.get("channel").str()) else {
                    return self.refetch_chans();
                };
                match ev.get("subtype").str() {
                    "message_changed" => {
                        let m = Msg::from(ev.get("message"), &me);
                        let ts = m.ts.clone();
                        self.each_msg(ui, ci, &ts, |x| *x = m.clone());
                    }
                    "message_deleted" => {
                        let ts = String::from(ev.get("deleted_ts").str());
                        self.remove_msg(ui, ci, &ts);
                    }
                    "message_replied" => {}
                    _ => self.new_msg(ui, ci, Msg::from(ev, &me)),
                }
            }
            t @ ("reaction_added" | "reaction_removed") => {
                let item = ev.get("item");
                let Some(ci) = self.idx(item.get("channel").str()) else { return };
                let name = String::from(ev.get("reaction").str());
                let mine = ev.get("user").str() == me;
                let add = t == "reaction_added";
                let ts = String::from(item.get("ts").str());
                self.each_msg(ui, ci, &ts, |m| {
                    let i = m.reactions.iter().position(|r| r.0 == name);
                    match (i, add) {
                        // Our own optimistic update already counted it.
                        (Some(i), true) if !(mine && m.reactions[i].2) => {
                            m.reactions[i].1 += 1;
                            m.reactions[i].2 |= mine;
                        }
                        (None, true) => m.reactions.push((name.clone(), 1, mine)),
                        (Some(i), false) if !(mine && !m.reactions[i].2) => {
                            m.reactions[i].1 = m.reactions[i].1.saturating_sub(1);
                            if mine {
                                m.reactions[i].2 = false;
                            }
                            m.reactions.retain(|r| r.1 > 0);
                        }
                        _ => {}
                    }
                });
            }
            "channel_joined" | "group_joined" | "im_created" | "channel_rename" | "group_rename" | "channel_left" | "group_left" | "channel_archive" => self.refetch_chans(),
            "member_joined_channel" if ev.get("user").str() == me => self.refetch_chans(),
            // Read on another device (RTM only).
            "channel_marked" | "group_marked" | "im_marked" | "mpim_marked" => {
                if let Some(ci) = self.idx(ev.get("channel").str()) {
                    if !self.is_cur(ci) {
                        let c = &mut self.chans[ci];
                        c.unread = ev.get("unread_count_display").num() > 0.0;
                        if !c.unread {
                            c.mentions = 0;
                        }
                        paint_chan(ui, c, false);
                    }
                }
            }
            _ => {}
        }
    }

    // ---------- conversations ----------

    fn idx(&self, id: &str) -> Option<usize> {
        self.chans.iter().position(|c| c.id == id)
    }

    fn is_cur(&self, ci: usize) -> bool {
        self.cur.as_deref() == Some(self.chans[ci].id.as_str())
    }

    fn refetch_chans(&mut self) {
        let now = now_ms();
        if now - self.last_chans < 10_000 {
            return;
        }
        self.last_chans = now;
        self.acc.clear();
        self.pages = 0;
        self.call(Tag::Chans, "users.conversations", &[("types", "public_channel,private_channel,mpim,im"), ("exclude_archived", "true"), ("limit", "200")]);
    }

    fn on_chans(&mut self, ui: &mut Ui) {
        let mut old: BTreeMap<String, Chan> = self.chans.drain(..).map(|c| (c.id.clone(), c)).collect();
        let pages = core::mem::take(&mut self.acc);
        for c in pages.iter().flat_map(|d| d.root().get("channels").iter()) {
            let kind = if c.get("is_im").bool() {
                Kind::Im
            } else if c.get("is_mpim").bool() {
                Kind::Mpim
            } else if c.get("is_private").bool() {
                Kind::Private
            } else {
                Kind::Public
            };
            let mut name = String::from(c.get("name").str());
            if kind == Kind::Mpim {
                let n = name.trim_start_matches("mpdm-");
                name = n.rsplit_once('-').map_or(n, |x| x.0).replace("--", ", ");
            }
            let user = String::from(c.get("user").str());
            if kind == Kind::Im {
                name = self.dir.users.get(&user).cloned().unwrap_or_else(|| user.clone());
            }
            let id = String::from(c.get("id").str());
            let ch = match old.remove(&id) {
                Some(o) => Chan { name, ..o },
                None => Chan {
                    id,
                    name,
                    kind,
                    user,
                    msgs: Vec::new(),
                    loaded: false,
                    more: true,
                    unread: false,
                    mentions: 0,
                    marked: String::new(),
                    row: Node(0),
                    label: Node(0),
                    badge: Node(0),
                },
            };
            self.chans.push(ch);
        }
        self.sort_chans();
        self.dir.chans = self.chans.iter().filter(|c| c.kind <= Kind::Private).map(|c| (c.id.clone(), c.name.clone())).collect();
        self.dir.chan_ids = self.chans.iter().filter(|c| c.kind <= Kind::Private).map(|c| (c.name.to_lowercase(), c.id.clone())).collect();
        self.build_sidebar(ui);
        if self.cur.as_deref().and_then(|id| self.idx(id)).is_some() {
            return;
        }
        let want = if self.cfg.last.is_empty() { "general" } else { self.cfg.last.as_str() };
        let pick = self.chans.iter().position(|c| c.id == want || c.name == want).or(self.chans.iter().position(|c| c.name == "general")).or((!self.chans.is_empty()).then_some(0));
        match pick {
            Some(ci) => {
                let id = self.chans[ci].id.clone();
                self.open(ui, &id);
            }
            None => ui.set_text(self.title, "No conversations"),
        }
        if self.counts_ok {
            self.call(Tag::Counts, "client.counts", &[]);
        }
    }

    fn sort_chans(&mut self) {
        self.chans.sort_by(|a, b| {
            let g = |k: Kind| (k >= Kind::Mpim) as u8;
            g(a.kind).cmp(&g(b.kind)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }

    fn build_sidebar(&mut self, ui: &mut Ui) {
        ui.destroy(self.side_list);
        let list = ui.grid().gap(0.0).pad4(0.0, 6.0, 12.0, 6.0).add_to(self.side_scroll);
        self.side_list = list;
        let mut section = None;
        for (i, c) in self.chans.iter_mut().enumerate() {
            let dm = c.kind >= Kind::Mpim;
            if section != Some(dm) {
                section = Some(dm);
                let h = ui.grid().pad4(12.0, 10.0, 4.0, 10.0).add_to(list);
                ui.text(if dm { "Direct messages" } else { "Channels" }).style("caption").variant("muted").add_to(h);
            }
            let row = ui.grid().cols(&[fr(1.0), auto()]).gap(6.0).pad4(4.0, 10.0, 4.0, 10.0).clickable(true).tag(i as i32 + 1).context_menu(self.chan_pop).add_to(list);
            c.label = ui.text(&c.label()).wrap(1).add_to(row);
            c.badge = ui.text("").style("caption").variant("accent").add_to(row);
            c.row = row;
            let cur = self.cur.as_deref() == Some(c.id.as_str());
            paint_chan(ui, c, cur);
        }
    }

    fn open(&mut self, ui: &mut Ui, id: &str) {
        if let Some(old) = self.cur.as_deref().and_then(|o| self.idx(o)) {
            paint_chan(ui, &self.chans[old], false);
        }
        let Some(ci) = self.idx(id) else { return };
        self.cur = Some(String::from(id));
        self.close_thread(ui);
        let c = &mut self.chans[ci];
        c.unread = false;
        c.mentions = 0;
        paint_chan(ui, c, true);
        let label = c.label();
        ui.set_text(self.title, &label);
        ui.set_str(self.input, prop::input::PLACEHOLDER, &cat(&["Message ", &label]));
        set_title(&cat(&[&label, " — Slack"]));
        self.build_main(ui);
        scroll_end(ui, &self.mview);
        if self.chans[ci].loaded {
            self.load_history(ci, Fetch::Newer);
            self.mark_read(ci);
        } else {
            self.load_history(ci, Fetch::Initial);
        }
        self.cfg.last = String::from(id);
        self.cfg.save();
        ui.focus(self.input);
    }

    fn switch_rel(&mut self, ui: &mut Ui, d: isize) {
        let n = self.chans.len() as isize;
        if n == 0 {
            return;
        }
        let i = self.cur.as_deref().and_then(|id| self.idx(id)).map_or(0, |i| i as isize);
        let id = self.chans[(i + d).rem_euclid(n) as usize].id.clone();
        self.open(ui, &id);
    }

    fn load_history(&mut self, ci: usize, f: Fetch) {
        let c = &self.chans[ci];
        let id = c.id.clone();
        let (oldest, latest) = match f {
            Fetch::Newer => (c.msgs.last().map(|m| m.ts.clone()).unwrap_or_default(), String::new()),
            Fetch::Older => (String::new(), c.msgs.first().map(|m| m.ts.clone()).unwrap_or_default()),
            _ => (String::new(), String::new()),
        };
        if f == Fetch::Newer && oldest.is_empty() && !c.loaded {
            return;
        }
        let limit = if f == Fetch::Refresh { "30" } else { "50" };
        let mut p = vec![("channel", id.as_str()), ("limit", limit)];
        if !oldest.is_empty() {
            p.push(("oldest", &oldest));
        }
        if !latest.is_empty() {
            p.push(("latest", &latest));
        }
        self.call(Tag::History(id.clone(), f), "conversations.history", &p);
    }

    fn on_history(&mut self, ui: &mut Ui, id: &str, f: Fetch, v: Val) {
        let Some(ci) = self.idx(id) else { return };
        let mut fetched: Vec<Msg> = v.get("messages").iter().map(|m| Msg::from(m, &self.dir.me)).collect();
        fetched.reverse();
        let more = v.get("has_more").bool();
        let viewing = self.is_cur(ci);
        let today = self.today();
        let c = &mut self.chans[ci];
        let mut added = Vec::new();
        match f {
            Fetch::Initial => {
                c.msgs = fetched;
                c.more = more;
                c.loaded = true;
                if viewing {
                    self.build_main(ui);
                    scroll_end(ui, &self.mview);
                }
            }
            Fetch::Older => {
                // Rows go in above the ones shown; the view stays where it is.
                let first = c.msgs.first().map_or(u64::MAX, |m| fmt::ts_us(&m.ts));
                fetched.retain(|m| fmt::ts_us(&m.ts) < first);
                c.more = more;
                let mut all = fetched.clone();
                all.append(&mut c.msgs);
                c.msgs = all;
                if viewing {
                    let c = &self.chans[ci];
                    prepend(ui, &self.dir, &mut self.mview, &mut self.acts, &fetched, &c.msgs, today);
                    if !c.more && self.mview.older.0 != 0 {
                        self.acts.remove(&self.mview.older.0);
                        ui.destroy(self.mview.older);
                        self.mview.older = Node(0);
                    }
                }
            }
            Fetch::Newer => {
                let last = c.msgs.last().map_or(0, |m| fmt::ts_us(&m.ts));
                for m in fetched {
                    if fmt::ts_us(&m.ts) > last && !is_reply(&m) {
                        added.push(m.clone());
                        c.msgs.push(m);
                    }
                }
                if viewing {
                    for m in &added {
                        push_msg(ui, &self.dir, &mut self.mview, &mut self.acts, m, false, today);
                    }
                }
            }
            Fetch::Refresh => {
                // Replace the window the fetch covers: picks up edits,
                // reactions, reply counts and deletions. Only rows that
                // changed are redrawn, so the view doesn't move.
                if let Some(lo) = fetched.first().map(|m| fmt::ts_us(&m.ts)) {
                    let keep = c.msgs.iter().position(|m| fmt::ts_us(&m.ts) >= lo).unwrap_or(c.msgs.len());
                    if c.msgs[keep..] != fetched[..] {
                        let old = c.msgs.split_off(keep);
                        c.msgs.extend(fetched.iter().cloned());
                        if viewing {
                            let last_old = old.last().map_or(0, |m| fmt::ts_us(&m.ts));
                            let inserted = fetched.iter().any(|m| fmt::ts_us(&m.ts) < last_old && !old.iter().any(|o| o.ts == m.ts));
                            if inserted {
                                self.build_main(ui);
                            } else {
                                for o in old.iter().filter(|o| !fetched.iter().any(|m| m.ts == o.ts)) {
                                    remove_row(ui, &mut self.mview, &mut self.acts, &o.ts);
                                }
                                for m in &fetched {
                                    match old.iter().find(|o| o.ts == m.ts) {
                                        Some(o) if o != m => replace_row(ui, &self.dir, &mut self.mview, &mut self.acts, m, false),
                                        Some(_) => {}
                                        None => push_msg(ui, &self.dir, &mut self.mview, &mut self.acts, m, false, today),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let c = &mut self.chans[ci];
        if c.msgs.len() > KEEP && f != Fetch::Older {
            // Forget the oldest; scroll anchoring keeps the view still.
            let cut = c.msgs.len() - KEEP;
            let gone: Vec<String> = c.msgs.drain(..cut).map(|m| m.ts).collect();
            c.more = true;
            if viewing {
                for ts in gone {
                    remove_row(ui, &mut self.mview, &mut self.acts, &ts);
                }
            }
        }
        if viewing && (f == Fetch::Initial || !added.is_empty()) {
            self.mark_read(ci);
        }
        self.want_users();
    }

    fn build_main(&mut self, ui: &mut Ui) {
        let today = self.today();
        let Some(ci) = self.cur.as_deref().and_then(|id| self.idx(id)) else { return };
        let c = &self.chans[ci];
        rebuild(ui, &self.dir, &mut self.mview, &mut self.acts, &c.msgs, c.more && c.loaded, false, today);
        if !c.loaded {
            let t = ui.text("Loading…").variant("muted").align_x(CENTER).add_to(self.mview.list);
            let _ = t;
        }
    }

    fn mark_read(&mut self, ci: usize) {
        let c = &mut self.chans[ci];
        c.unread = false;
        c.mentions = 0;
        let Some(ts) = c.msgs.last().map(|m| m.ts.clone()) else { return };
        if self.mark_ok && c.marked != ts {
            c.marked = ts.clone();
            let id = c.id.clone();
            self.call(Tag::Mark, "conversations.mark", &[("channel", &id), ("ts", &ts)]);
        }
    }

    fn new_msg(&mut self, ui: &mut Ui, ci: usize, m: Msg) {
        let me = self.dir.me.clone();
        self.want_user(&m.user);
        let reply = is_reply(&m);
        let mention = m.text.contains(&cat(&["<@", &me, ">"]));
        let today = self.today();
        let chan_id = self.chans[ci].id.clone();
        if let Some(t) = self.thread.as_mut().filter(|t| t.chan == chan_id && t.ts == m.thread_ts) {
            if !t.msgs.iter().any(|x| x.ts == m.ts) {
                t.msgs.push(m.clone());
                push_msg(ui, &self.dir, &mut self.tview, &mut self.acts, &m, true, today);
            }
        }
        let viewing = self.is_cur(ci);
        if reply {
            let parent = m.thread_ts.clone();
            self.each_msg(ui, ci, &parent, |p| p.replies += 1);
        } else if self.chans[ci].loaded && !self.chans[ci].msgs.iter().any(|x| x.ts == m.ts) {
            self.chans[ci].msgs.push(m.clone());
            if viewing {
                push_msg(ui, &self.dir, &mut self.mview, &mut self.acts, &m, false, today);
            }
        }
        if m.user != me && !viewing && !reply {
            let c = &mut self.chans[ci];
            c.unread = true;
            if mention || c.kind == Kind::Im {
                c.mentions += 1;
            }
            paint_chan(ui, c, false);
        }
        if viewing && !reply {
            self.mark_read(ci);
        }
    }

    /// Applies `f` to the message with `ts` in a conversation and its open
    /// thread, and redraws it where it is shown.
    fn each_msg(&mut self, ui: &mut Ui, ci: usize, ts: &str, mut f: impl FnMut(&mut Msg)) {
        let viewing = self.is_cur(ci);
        if let Some(m) = self.chans[ci].msgs.iter_mut().find(|m| m.ts == ts) {
            f(m);
            if viewing {
                replace_row(ui, &self.dir, &mut self.mview, &mut self.acts, m, false);
            }
        }
        let id = self.chans[ci].id.clone();
        if let Some(t) = self.thread.as_mut().filter(|t| t.chan == id) {
            if let Some(m) = t.msgs.iter_mut().find(|m| m.ts == ts) {
                f(m);
                replace_row(ui, &self.dir, &mut self.tview, &mut self.acts, m, true);
            }
        }
    }

    fn remove_msg(&mut self, ui: &mut Ui, ci: usize, ts: &str) {
        self.chans[ci].msgs.retain(|m| m.ts != ts);
        if self.is_cur(ci) {
            remove_row(ui, &mut self.mview, &mut self.acts, ts);
        }
        let id = self.chans[ci].id.clone();
        if let Some(t) = self.thread.as_mut().filter(|t| t.chan == id) {
            t.msgs.retain(|m| m.ts != ts);
            remove_row(ui, &mut self.tview, &mut self.acts, ts);
        }
    }

    // ---------- threads ----------

    fn open_thread(&mut self, ui: &mut Ui, ts: &str) {
        let Some(ci) = self.cur.as_deref().and_then(|id| self.idx(id)) else { return };
        let c = &self.chans[ci];
        let msgs: Vec<Msg> = c.msgs.iter().filter(|m| m.ts == ts).cloned().collect();
        self.thread = Some(Thread { chan: c.id.clone(), ts: String::from(ts), msgs });
        ui.hide(self.tsep, false);
        ui.hide(self.tpane, false);
        self.build_thread(ui);
        scroll_end(ui, &self.tview);
        self.load_thread();
        ui.focus(self.tinput);
    }

    fn close_thread(&mut self, ui: &mut Ui) {
        if self.thread.take().is_some() {
            clear_view(ui, &mut self.tview, &mut self.acts);
            ui.hide(self.tsep, true);
            ui.hide(self.tpane, true);
            ui.focus(self.input);
        }
    }

    fn build_thread(&mut self, ui: &mut Ui) {
        let today = self.today();
        if let Some(t) = &self.thread {
            rebuild(ui, &self.dir, &mut self.tview, &mut self.acts, &t.msgs, false, true, today);
        }
    }

    fn load_thread(&mut self) {
        let Some(t) = &self.thread else { return };
        let (chan, ts) = (t.chan.clone(), t.ts.clone());
        self.call(Tag::Replies(chan.clone(), ts.clone()), "conversations.replies", &[("channel", &chan), ("ts", &ts), ("limit", "200")]);
    }

    fn on_replies(&mut self, ui: &mut Ui, chan: &str, ts: &str, v: Val) {
        let msgs: Vec<Msg> = v.get("messages").iter().map(|m| Msg::from(m, &self.dir.me)).collect();
        let Some(t) = self.thread.as_mut().filter(|t| t.chan == chan && t.ts == ts) else { return };
        if t.msgs != msgs {
            let n = msgs.len().saturating_sub(1) as u32;
            t.msgs = msgs;
            self.build_thread(ui);
            if let Some(ci) = self.idx(chan) {
                self.each_msg(ui, ci, ts, |p| {
                    if p.replies != n && p.ts == ts {
                        p.replies = n;
                    }
                });
            }
        }
        self.want_users();
    }

    // ---------- actions ----------

    fn send(&mut self, ui: &mut Ui, in_thread: bool) {
        let raw = core::mem::take(if in_thread { &mut self.tdraft } else { &mut self.draft });
        let text = fmt::encode(raw.trim_end(), &self.dir.handles, &self.dir.chan_ids);
        if text.trim().is_empty() {
            return;
        }
        let (chan, thread) = match (&self.thread, in_thread) {
            (Some(t), true) => (t.chan.clone(), Some(t.ts.clone())),
            (_, false) => match &self.cur {
                Some(c) => (c.clone(), None),
                None => return,
            },
            _ => return,
        };
        let mut p = vec![("channel", chan.as_str()), ("text", text.as_str())];
        if let Some(t) = &thread {
            p.push(("thread_ts", t));
        }
        self.call(Tag::Posted(chan.clone()), "chat.postMessage", &p);
        ui.set_value(if in_thread { self.tinput } else { self.input }, "");
        // Sending goes back to the newest messages, where the reply will show.
        scroll_end(ui, if in_thread { &self.tview } else { &self.mview });
    }

    fn act(&mut self, ui: &mut Ui, a: Act) {
        match a {
            Act::Thread(ts) => self.open_thread(ui, &ts),
            Act::Links(_) => {}
            Act::Older => {
                if let Some(ci) = self.cur.as_deref().and_then(|id| self.idx(id)) {
                    self.load_history(ci, Fetch::Older);
                }
            }
            Act::React(in_thread, ts, name) => {
                let chan = match (&self.thread, in_thread) {
                    (Some(t), true) => t.chan.clone(),
                    _ => self.cur.clone().unwrap_or_default(),
                };
                let Some(ci) = self.idx(&chan) else { return };
                let list = if in_thread { self.thread.as_ref().map(|t| &t.msgs) } else { Some(&self.chans[ci].msgs) };
                let mine = list.and_then(|l| l.iter().find(|m| m.ts == ts)).is_some_and(|m| m.reactions.iter().any(|r| r.0 == name && r.2));
                self.each_msg(ui, ci, &ts, |m| {
                    if let Some(i) = m.reactions.iter().position(|r| r.0 == name) {
                        let r = &mut m.reactions[i];
                        if mine && r.2 {
                            r.1 = r.1.saturating_sub(1);
                            r.2 = false;
                        } else if !mine && !r.2 {
                            r.1 += 1;
                            r.2 = true;
                        }
                    } else if !mine {
                        m.reactions.push((name.clone(), 1, true));
                    }
                    m.reactions.retain(|r| r.1 > 0);
                });
                let method = if mine { "reactions.remove" } else { "reactions.add" };
                self.call(Tag::Done, method, &[("channel", &chan), ("timestamp", &ts), ("name", &name)]);
            }
        }
    }

    // ---------- context menus ----------

    fn on_context(&mut self, ui: &mut Ui, node: Node) {
        self.target = None;
        self.chan_target = None;
        self.menu_acts.clear();
        let mut items: Vec<&str> = Vec::new();
        if let Some(c) = self.chans.iter().find(|c| c.row == node) {
            self.chan_target = Some(c.id.clone());
            items.push("Mark as read");
            self.menu_acts.push(MenuAct::ChanRead);
            if !self.team_id.is_empty() {
                items.push("Open in Slack ↗");
                self.menu_acts.push(MenuAct::ChanOpen);
            }
            ui.set_items(self.chan_menu, &items.join("\n"));
            return;
        }
        let (thread, ts) = if let Some(r) = self.mview.rows.iter().find(|r| r.node == node) {
            (false, r.ts.clone())
        } else if let Some(r) = self.tview.rows.iter().find(|r| r.node == node) {
            (true, r.ts.clone())
        } else {
            return;
        };
        let chan = if thread { self.thread.as_ref().map(|t| t.chan.clone()) } else { self.cur.clone() };
        let Some(chan) = chan else { return };
        let mine = self.find_msg(thread, &chan, &ts).is_some_and(|m| m.user == self.dir.me);
        let mut add = |label: &'static str, a: MenuAct| {
            items.push(label);
            self.menu_acts.push(a);
        };
        if !thread {
            add("Reply in thread", MenuAct::Thread);
        }
        add("Add reaction…", MenuAct::React);
        add("Copy text", MenuAct::CopyText);
        add("Copy link", MenuAct::CopyLink);
        if !thread {
            add("Mark unread", MenuAct::MarkUnread);
        }
        if mine {
            add("-", MenuAct::None);
            add("Delete message", MenuAct::Delete);
        }
        ui.set_items(self.msg_menu, &items.join("\n"));
        self.target = Some((thread, chan, ts));
    }

    fn find_msg(&self, thread: bool, chan: &str, ts: &str) -> Option<&Msg> {
        if thread {
            self.thread.as_ref().filter(|t| t.chan == chan)?.msgs.iter().find(|m| m.ts == ts)
        } else {
            self.chans[self.idx(chan)?].msgs.iter().find(|m| m.ts == ts)
        }
    }

    fn on_menu(&mut self, ui: &mut Ui, i: i32) {
        let Some(&a) = self.menu_acts.get(i as usize) else { return };
        if let Some(cid) = self.chan_target.clone() {
            let Some(ci) = self.idx(&cid) else { return };
            match a {
                MenuAct::ChanRead => {
                    self.mark_read(ci);
                    let cur = self.is_cur(ci);
                    paint_chan(ui, &self.chans[ci], cur);
                }
                MenuAct::ChanOpen => {
                    open_url(&cat(&["https://app.slack.com/client/", &self.team_id, "/", &cid]));
                }
                _ => {}
            }
            return;
        }
        let Some((thread, chan, ts)) = self.target.clone() else { return };
        match a {
            MenuAct::Thread => self.open_thread(ui, &ts),
            MenuAct::React => ui.open(self.react_pop, Node(0)),
            MenuAct::CopyText => {
                if let Some(m) = self.find_msg(thread, &chan, &ts) {
                    let t = fmt::render(&m.text, &self.dir.users, &self.dir.chans);
                    clipboard::write(&t.rich.text);
                    self.flash(ui, "Copied");
                }
            }
            MenuAct::CopyLink => self.call(Tag::Permalink, "chat.getPermalink", &[("channel", &chan), ("message_ts", &ts)]),
            MenuAct::MarkUnread => {
                let Some(ci) = self.idx(&chan) else { return };
                let c = &mut self.chans[ci];
                let Some(i) = c.msgs.iter().position(|m| m.ts == ts) else { return };
                // Read up to the message before this one.
                let before = if i > 0 { c.msgs[i - 1].ts.clone() } else { String::from("0") };
                c.marked = before.clone();
                c.unread = true;
                self.call(Tag::Mark, "conversations.mark", &[("channel", &chan), ("ts", &before)]);
                self.flash(ui, "Marked unread from here");
            }
            MenuAct::Delete => self.call(Tag::Deleted(chan.clone(), ts.clone()), "chat.delete", &[("channel", &chan), ("ts", &ts)]),
            _ => {}
        }
    }

    // ---------- names ----------

    fn want_user(&mut self, uid: &str) {
        if self.users_done && !uid.is_empty() && !self.dir.users.contains_key(uid) && uid.starts_with(['U', 'W']) {
            let uid = String::from(uid);
            self.call(Tag::User(uid.clone()), "users.info", &[("user", &uid)]);
        }
    }

    /// Asks for names the shown messages need that users.list didn't have.
    fn want_users(&mut self) {
        if !self.users_done {
            return;
        }
        let mut ids: BTreeSet<String> = BTreeSet::new();
        let cur = self.cur.as_deref().and_then(|id| self.idx(id));
        let lists = cur.map(|ci| &self.chans[ci].msgs).into_iter().chain(self.thread.as_ref().map(|t| &t.msgs));
        for l in lists {
            for m in l {
                if !self.dir.users.contains_key(&m.user) {
                    ids.insert(m.user.clone());
                }
            }
        }
        for u in ids {
            self.want_user(&u);
        }
    }

    /// Redraws names shortly, so a burst of user lookups redraws once.
    fn names_changed(&mut self) {
        if self.names_timer == 0 {
            self.names_timer = timer(300);
        }
    }

    fn redraw_names(&mut self, ui: &mut Ui) {
        let mut renamed = false;
        for c in &mut self.chans {
            if c.kind == Kind::Im {
                if let Some(n) = self.dir.users.get(&c.user) {
                    if *n != c.name {
                        c.name = n.clone();
                        renamed = true;
                    }
                }
            }
        }
        if renamed {
            self.sort_chans();
            self.build_sidebar(ui);
            if let Some(ci) = self.cur.as_deref().and_then(|id| self.idx(id)) {
                ui.set_text(self.title, &self.chans[ci].label());
            }
        }
        // Redraw rows in place, so the view doesn't move.
        if let Some(ci) = self.cur.as_deref().and_then(|id| self.idx(id)) {
            for m in &self.chans[ci].msgs {
                replace_row(ui, &self.dir, &mut self.mview, &mut self.acts, m, false);
            }
        }
        if let Some(t) = &self.thread {
            for m in &t.msgs {
                replace_row(ui, &self.dir, &mut self.tview, &mut self.acts, m, true);
            }
        }
    }

    /// Today's local day number.
    fn today(&self) -> i64 {
        time::day(time::now())
    }
}

fn is_reply(m: &Msg) -> bool {
    !m.thread_ts.is_empty() && m.thread_ts != m.ts && m.sub != "thread_broadcast"
}

/// Errors that retrying won't fix.
fn fatal(e: &str) -> bool {
    ["invalid_auth", "not_authed", "not_allowed_token_type", "missing_scope", "account_inactive", "token_revoked", "token_expired", "method_deprecated", "denied"]
        .iter()
        .any(|f| e.contains(f))
}

fn paint_chan(ui: &mut Ui, c: &Chan, cur: bool) {
    if c.row.0 == 0 {
        return;
    }
    ui.set(c.row, prop::grid::SELECTED, cur as i32);
    let unread = c.unread && !cur;
    ui.ctx(c.label, "font.body.weight", Tok::Num(if unread { 700.0 } else { 400.0 }));
    ui.variant(c.label, if unread || cur { "" } else { "muted" });
    let show = c.mentions > 0 && !cur;
    let n = itoa(c.mentions as i64);
    ui.set_text(c.badge, if show { n.as_str() } else { "" });
    ui.hide(c.badge, !show);
}

/// Colors that read on light and dark themes alike.
const NAME_COLORS: [u32; 8] = [0x2f80edff, 0xd9480fff, 0x2b8a3eff, 0xae3ec9ff, 0xc2255cff, 0x1098adff, 0xe67700ff, 0x6741d9ff];

fn name_color(s: &str) -> u32 {
    let h = s.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    NAME_COLORS[(h % NAME_COLORS.len() as u32) as usize]
}

fn clear_view(ui: &mut Ui, v: &mut View, acts: &mut BTreeMap<u32, Act>) {
    for r in v.rows.drain(..) {
        for a in r.acts {
            acts.remove(&a.0);
        }
    }
    acts.remove(&v.older.0);
    if v.list.0 != 0 {
        ui.destroy(v.list);
    }
    v.list = ui.grid().gap(0.0).add_to(v.host);
    v.last = None;
    v.day = None;
    v.older = Node(0);
    v.first_sep = Node(0);
    v.top = Node(0);
}

fn rebuild(ui: &mut Ui, dir: &Dir, v: &mut View, acts: &mut BTreeMap<u32, Act>, msgs: &[Msg], more: bool, thread: bool, today: i64) {
    clear_view(ui, v, acts);
    if more {
        v.older = ui.button("Load older messages").variant("flat").align_x(CENTER).add_to(v.list);
        acts.insert(v.older.0, Act::Older);
    }
    for m in msgs {
        push_msg(ui, dir, v, acts, m, thread, today);
    }
}

/// Shows the newest message. A `stick_end` scroll then follows new ones.
fn scroll_end(ui: &mut Ui, v: &View) {
    if let Some(r) = v.rows.last() {
        ui.scroll_to(r.node);
    }
}

/// Adds older messages above the ones shown. `all` is the whole list after them.
fn prepend(ui: &mut Ui, dir: &Dir, v: &mut View, acts: &mut BTreeMap<u32, Act>, older: &[Msg], all: &[Msg], today: i64) {
    if older.is_empty() {
        return;
    }
    if v.rows.is_empty() {
        for m in older {
            push_msg(ui, dir, v, acts, m, false, today);
        }
        return;
    }
    let block = ui.grid().gap(0.0).detached();
    let mut top = View { host: v.host, list: block, menu: v.menu, ..View::default() };
    for m in older {
        push_msg(ui, dir, &mut top, acts, m, false, today);
    }
    ui.insert(v.list, block, v.top);
    v.top = block;
    // Join the two: one separator per day, one name line per group.
    let first = &v.rows[0];
    let secs = fmt::local_secs(&first.ts);
    if top.day == Some(time::day(secs)) {
        ui.destroy(v.first_sep);
        if let (Some((w, t)), Some(m)) = (&top.last, all.iter().find(|m| m.ts == first.ts)) {
            if w == m.who() && secs - t <= 300 {
                v.rows[0].head = false;
                replace_row(ui, dir, v, acts, m, false);
            }
        }
    }
    v.first_sep = top.first_sep;
    let mut rows = core::mem::take(&mut top.rows);
    rows.append(&mut v.rows);
    v.rows = rows;
}

fn push_msg(ui: &mut Ui, dir: &Dir, v: &mut View, acts: &mut BTreeMap<u32, Act>, m: &Msg, thread: bool, today: i64) {
    let secs = fmt::local_secs(&m.ts);
    let day = time::day(secs);
    if v.day != Some(day) {
        v.day = Some(day);
        v.last = None;
        let sep = ui.grid().cols(&[fr(1.0), auto(), fr(1.0)]).gap(10.0).pad4(12.0, 16.0, 4.0, 16.0).add_to(v.list);
        ui.divider().align_y(CENTER).add_to(sep);
        ui.text(&time::day_label(day, today)).style("caption").variant("muted").add_to(sep);
        ui.divider().align_y(CENTER).add_to(sep);
        if v.rows.is_empty() {
            v.first_sep = sep;
            if v.top.0 == 0 {
                v.top = sep;
            }
        }
    }
    let head = match &v.last {
        Some((w, t)) => w != m.who() || secs - t > 300,
        None => true,
    };
    let (node, a) = msg_row(ui, dir, m, head, thread, v.menu, acts);
    ui.append(v.list, node);
    v.rows.push(Row { ts: m.ts.clone(), node, head, acts: a });
    v.last = Some((String::from(m.who()), secs));
}

fn replace_row(ui: &mut Ui, dir: &Dir, v: &mut View, acts: &mut BTreeMap<u32, Act>, m: &Msg, thread: bool) {
    let Some(r) = v.rows.iter_mut().find(|r| r.ts == m.ts) else { return };
    for a in r.acts.drain(..) {
        acts.remove(&a.0);
    }
    let (node, a) = msg_row(ui, dir, m, r.head, thread, v.menu, acts);
    ui.replace(r.node, node);
    r.node = node;
    r.acts = a;
}

fn remove_row(ui: &mut Ui, v: &mut View, acts: &mut BTreeMap<u32, Act>, ts: &str) {
    if let Some(i) = v.rows.iter().position(|r| r.ts == ts) {
        let r = v.rows.remove(i);
        for a in r.acts {
            acts.remove(&a.0);
        }
        ui.destroy(r.node);
    }
}

/// One message: an optional name/time line, the text, files, then a bar of
/// reactions, the reply count and a link button.
fn msg_row(ui: &mut Ui, dir: &Dir, m: &Msg, head: bool, thread: bool, menu: Node, acts: &mut BTreeMap<u32, Act>) -> (Node, Vec<Node>) {
    let mut a = Vec::new();
    let mut act = |n: Node, x: Act, a: &mut Vec<Node>| {
        acts.insert(n.0, x);
        a.push(n);
    };
    let time = time::hhmm(fmt::local_secs(&m.ts));
    let row = ui.grid().cols(&[fr(1.0), auto()]).gap(4.0).pad4(if head { 8.0 } else { 1.0 }, 8.0, 1.0, 16.0).context_menu(menu).detached();
    let body = ui.grid().gap(2.0).add_to(row);
    if head {
        let h = ui.grid().cols(&[auto(), auto(), fr(1.0)]).gap(8.0).add_to(body);
        ui.text(&dir.name_of(m)).wrap(1).ctx("font.body.weight", Tok::Num(600.0)).ctx("color.fg", Tok::Color(name_color(m.who()))).add_to(h);
        ui.text(&time).style("caption").variant("muted").align_y(END).add_to(h);
    }
    let mut t = fmt::render(&m.text, &dir.users, &dir.chans);
    if m.edited {
        t.rich.styled(" (edited)", 0, "muted");
    }
    if !t.rich.text.is_empty() {
        let quiet = matches!(m.sub.as_str(), "channel_join" | "channel_leave" | "channel_topic" | "channel_purpose" | "group_join" | "group_leave");
        let n = ui.rich(&t.rich).selectable(true).tooltip(&time).variant(if quiet { "muted" } else { "" }).add_to(body);
        if !t.links.is_empty() {
            act(n, Act::Links(t.links), &mut a);
        }
    }
    for f in &m.files {
        ui.text(&cat(&["▤ ", f])).style("caption").variant("muted").add_to(body);
    }
    let replies = m.replies > 0 && !thread;
    if !m.reactions.is_empty() || replies {
        let bar = ui.grid().flow(true).gap(4.0).pad4(2.0, 0.0, 2.0, 0.0).add_to(body);
        for (name, count, mine) in &m.reactions {
            let label = cat(&[&fmt::reaction(name), " ", &itoa(*count as i64)]);
            let b = ui.button(&label).variant(if *mine { "" } else { "flat" }).tooltip(name).add_to(bar);
            act(b, Act::React(thread, m.ts.clone(), name.clone()), &mut a);
        }
        if replies {
            let label = cat(&["↳ ", &itoa(m.replies as i64), if m.replies == 1 { " reply" } else { " replies" }]);
            let b = ui.button(&label).variant("flat").add_to(bar);
            act(b, Act::Thread(m.ts.clone()), &mut a);
        }
    }
    if !thread {
        let b = ui.button("↩").variant("flat").tooltip("Reply in thread").align_y(START).add_to(row);
        act(b, Act::Thread(m.ts.clone()), &mut a);
    }
    (row, a)
}

app!(Slack);
