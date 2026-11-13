//! The Import/Export tab: the account import (OAuth or by name), the import box for links and
//! share codes, and Share. The engine drives PoB's `ImportTab`; this draws its state.
//!
//! The engine never opens a browser or touches the clipboard (R1): the tab asks for both
//! through `ImportOutput`.

use crate::engine::views::{
    AccountView, CharacterEntry, PendingRequest, PublicStage, ShareView, SiteRef,
    account_name_state,
};
use crate::engine::{CharacterPart, Command, ImportMode, Realm, Snapshot};
use crate::theme;
use crate::widgets::{EditBox, EditBoxOptions};
use eframe::egui;
use std::time::Duration;

/// The import box sends its text this long after typing stops.
const DEBOUNCE: f64 = 0.3;

/// The only link the tab opens (R1); the engine already drops anything else.
const AUTHORIZE_PREFIX: &str = "https://www.pathofexile.com/oauth/authorize?";

/// PoB's popup text when a tree import would replace allocated nodes.
const TREE_OVERWRITE: &str = "Importing the passive tree will overwrite your current tree.";
/// PoB's popup text for an import into the open build.
const BUILD_OVERWRITE: &str =
    "Importing to the current build will erase ALL existing data for this build.";

/// PoB's tips on the import options (ImportTab.lua:297-333, 445-461)
const TIP_JEWELS: &str = "Delete all equipped jewels when importing.";
const TIP_SKILLS: &str = "Delete all existing skills when importing.";
const TIP_ITEMS: &str = "Delete all equipped items when importing.";
const TIP_SWAP: &str = "Ignore items and skills in weapon swap.";

/// The commands of one character import: PoB's two buttons. The second goes only after the
/// first request has ended, as PoB's buttons do (one import at a time).
#[derive(Debug, Clone)]
pub struct Plan {
    first: Command,
    then: Option<Command>,
}

impl Plan {
    /// The plan's only command, when it has no second part: it needs no sequencing
    pub fn single(&self) -> Option<Command> {
        self.then.is_none().then(|| self.first.clone())
    }
}

/// What the tab wants asked before it acts.
#[derive(Debug, Clone)]
pub enum Confirm {
    /// Ask with this message; yes starts the plan (`ImportUi::begin`)
    Ask { message: &'static str, plan: Plan },
    /// An import into a new build: it goes through the unsaved-changes prompt
    NewBuild { url: String },
}

#[derive(Debug, Default)]
pub struct ImportOutput {
    pub commands: Vec<Command>,
    pub confirm: Option<Confirm>,
    /// The sign-in link, to open in the browser
    pub open_url: Option<String>,
    /// Text for the system clipboard
    pub copy: Option<String>,
}

struct Chain {
    /// The seq of the first command, once it is sent
    head: Option<u64>,
    rest: Command,
}

pub struct ImportUi {
    account: Option<std::sync::Arc<AccountView>>,
    share: Option<std::sync::Arc<ShareView>>,
    pending: Vec<PendingRequest>,
    /// The tree has allocated nodes, so a tree import replaces something
    tree_used: bool,
    modal: bool,

    oauth_mode: bool,
    realm: Realm,
    /// The league filter; `None` is PoB's "Any" (or "All")
    league: Option<String>,
    selected: Option<String>,
    part_tree: bool,
    part_items: bool,
    delete_jewels: bool,
    clear_skills: bool,
    clear_items: bool,
    ignore_weapon_swap: bool,
    account_box: EditBox,
    /// The account name the view last held: the box takes it when it changes, and otherwise
    /// keeps what was typed, as PoB's box keeps its text
    account_seen: Option<String>,
    account_id: egui::Id,
    chain: Option<Chain>,
    ready: Vec<Command>,

    /// Our `SignIn` went out as this command
    sign_seq: Option<u64>,
    /// The sign-in link has not been opened yet
    await_open: bool,
    open_ready: Option<String>,
    focus: bool,

    code: String,
    code_dirty: Option<f64>,
    /// The latest `SetImportCode`; an older view would put older text back
    code_sent_seq: u64,
    /// The seq of the latest box verdict (`Snapshot::Share`)
    share_seq: u64,
    /// Enter was pressed on text PoB had not judged yet: the `SetImportCode` before it went out
    /// as this seq, and Enter acts once the verdict on the newer text comes
    enter_after: Option<u64>,
    /// The mode picked; `None` is PoB's drop-down as it starts: "Import to this build" for a
    /// build with a file, else "Import to a new build" (ImportTab.lua:585-587, 680)
    mode: Option<ImportMode>,

    site: Option<String>,
    /// An `ExportShareCode` is out; its answer is copied
    await_code: bool,
    copy_ready: Option<String>,
    /// Ctrl+I: the code box takes the keyboard when next drawn (Build.lua:1162-1163)
    focus_code: bool,
    /// The account picked in PoB's history dropdown (NF-8)
    history_pick: Option<String>,
    /// The realm drop-down has taken PoB's (its remembered realm, `main.lastRealm`)
    realm_seen: bool,
    /// The character list whose picks were taken from PoB's drop-downs: a new list starts on
    /// PoB's league and character, as PoB's own drop-downs do
    list_seen: Option<(bool, Realm, Vec<String>)>,
    /// PoB's once-per-tab fetch of the remembered realm's characters was tried
    /// (`TryFetchCharacterList`)
    auto_fetched: bool,
}

impl Default for ImportUi {
    fn default() -> Self {
        ImportUi {
            account: None,
            share: None,
            pending: Vec::new(),
            tree_used: false,
            modal: false,
            oauth_mode: true,
            realm: Realm::Pc,
            league: None,
            selected: None,
            part_tree: true,
            part_items: true,
            // PoB's defaults
            delete_jewels: true,
            clear_skills: true,
            clear_items: true,
            ignore_weapon_swap: false,
            account_box: EditBox::default(),
            account_seen: None,
            account_id: egui::Id::unique("import_account_name"),
            chain: None,
            ready: Vec::new(),
            sign_seq: None,
            await_open: false,
            open_ready: None,
            focus: false,
            code: String::new(),
            code_dirty: None,
            code_sent_seq: 0,
            share_seq: 0,
            enter_after: None,
            mode: None,
            site: None,
            await_code: false,
            copy_ready: None,
            focus_code: false,
            history_pick: None,
            realm_seen: false,
            list_seen: None,
            auto_fetched: false,
        }
    }
}

impl ImportUi {
    /// Ctrl+I selects the code box (`importCodeIn`)
    pub fn focus_code(&mut self) {
        self.focus_code = true;
    }
}

#[cfg(test)]
thread_local! {
    static MARKS: std::cell::RefCell<std::collections::HashMap<String, egui::Rect>> =
        std::cell::RefCell::default();
}

/// Records where a widget was drawn, for the tests.
#[cfg(test)]
pub(crate) fn mark(_ui: &egui::Ui, key: &str, response: &egui::Response) {
    MARKS.with(|m| m.borrow_mut().insert(key.to_owned(), response.rect));
}

#[cfg(not(test))]
pub(crate) fn mark(_ui: &egui::Ui, _key: &str, _response: &egui::Response) {}

/// Where the widget was drawn in the latest frame.
#[cfg(test)]
pub(crate) fn marked(key: &str) -> Option<egui::Rect> {
    MARKS.with(|m| m.borrow().get(key).copied())
}

/// Forgets the marks; the tests do it before each frame so a widget that is gone is not found.
#[cfg(test)]
pub(crate) fn clear_marks() {
    MARKS.with(|m| m.borrow_mut().clear());
}

/// The colour of PoB's verdict on the import box: its `POSITIVE` or `NEGATIVE` code
/// (ImportTab.lua:577-620)
pub fn detail_colour(valid: bool) -> egui::Color32 {
    if valid {
        theme::POSITIVE
    } else {
        theme::NEGATIVE
    }
}

/// Whether the account list has settled: no fetch or import runs (the next part may go).
fn idle(account: &AccountView) -> bool {
    !account.oauth.busy
        && matches!(
            account.public.stage,
            PublicStage::EnterAccount | PublicStage::SelectCharacter
        )
}

fn realm_label(realm: Realm) -> &'static str {
    match realm {
        Realm::Pc => "PC",
        Realm::Xbox => "Xbox",
        Realm::Sony => "Sony",
    }
}

/// The league filter's entry for a drop-down label: the last one, "Any" or "All", is no filter.
fn league_value(labels: &[String], index: usize) -> Option<String> {
    let label = &labels[index];
    if index + 1 == labels.len() && (label == "Any" || label == "All") {
        None
    } else {
        Some(label.clone())
    }
}

impl ImportUi {
    /// Forgets the build's state, as for another build.
    pub fn reset(&mut self) {
        // the requests are the engine's, not the build's
        let pending = std::mem::take(&mut self.pending);
        *self = Self::default();
        self.pending = pending;
    }

    pub fn set_modal(&mut self, modal: bool) {
        self.modal = modal;
    }

    pub fn on_snapshot(&mut self, snapshot: &Snapshot, seq: u64) {
        match snapshot {
            Snapshot::Account(view) => {
                self.account = Some(view.clone());
                self.on_account(view, seq);
            }
            Snapshot::Share(view) => {
                self.share = Some(view.clone());
                self.share_seq = self.share_seq.max(seq);
                // The user's newer typing wins, and so does text sent after the view was taken
                if self.code_dirty.is_none() && seq >= self.code_sent_seq {
                    self.code.clone_from(&view.import.text);
                }
            }
            Snapshot::ShareCode(code) => {
                if std::mem::take(&mut self.await_code) {
                    self.copy_ready = Some(code.to_string());
                }
            }
            Snapshot::Requests(view) => {
                self.pending.clone_from(&view.pending);
                // a tree part that was cancelled or failed ends the import: the items part
                // must not go after the user said stop
                if view.finished.iter().any(|f| {
                    f.label == "Importing passive tree" && (f.cancelled || f.error.is_some())
                }) {
                    self.chain = None;
                }
            }
            Snapshot::Tree(state) => self.tree_used = state.points.used > 0,
            _ => {}
        }
    }

    fn on_account(&mut self, account: &AccountView, seq: u64) {
        let oauth = &account.oauth;
        if self.sign_seq.is_some_and(|s| seq >= s) {
            if oauth.signed_in {
                self.sign_seq = None;
                self.await_open = false;
                self.focus = true;
            } else if self.await_open {
                match &oauth.auth_url {
                    Some(url) if url.starts_with(AUTHORIZE_PREFIX) => {
                        self.await_open = false;
                        self.open_ready = Some(url.clone());
                    }
                    None if !oauth.signing_in => {
                        self.sign_seq = None;
                        self.await_open = false;
                    }
                    _ => {}
                }
            } else if !oauth.signing_in {
                // cancelled or failed
                self.sign_seq = None;
            }
        }
        if self
            .chain
            .as_ref()
            .is_some_and(|c| c.head.is_some_and(|h| seq >= h))
            && idle(account)
            && let Some(chain) = self.chain.take()
        {
            self.ready.push(chain.rest);
        }
    }

    /// A command of ours failed: whatever waited for its answer stops waiting.
    pub fn on_error(&mut self, seq: u64) {
        if self.sign_seq == Some(seq) {
            self.sign_seq = None;
            self.await_open = false;
        }
        if self.chain.as_ref().is_some_and(|c| c.head == Some(seq)) {
            self.chain = None;
        }
    }

    /// The command went out as `seq`.
    pub fn sent(&mut self, seq: u64, command: &Command) {
        match command {
            Command::SignIn => {
                self.sign_seq = Some(seq);
                self.await_open = true;
            }
            Command::ExportShareCode => self.await_code = true,
            Command::SetImportCode { .. } => self.code_sent_seq = seq,
            Command::ImportCharacter { .. } | Command::ImportPublicCharacter { .. } => {
                if let Some(chain) = &mut self.chain
                    && chain.head.is_none()
                {
                    chain.head = Some(seq);
                }
            }
            _ => {}
        }
    }

    /// The sign-in link to open, once. The app asks every frame, so a tab switch after Sign in
    /// does not lose it.
    pub fn take_open_url(&mut self) -> Option<String> {
        self.open_ready.take()
    }

    /// The sign-in completed: the window should come back from behind the browser. Once.
    pub fn take_focus(&mut self) -> bool {
        std::mem::take(&mut self.focus)
    }

    /// Commands that were waiting for an earlier request to end. The app asks every frame,
    /// whichever tab is showing.
    pub fn take_ready(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.ready)
    }

    /// Whether cancelling this request may sign the user out: PoB signs out when the token
    /// refresh inside an OAuth fetch or import is cancelled, and that is in both apps.
    /// Account-name requests carry the same labels, so a signed-in user is asked for those too
    /// (an unneeded question is harmless, a missing one is not).
    pub fn cancel_signs_out(&self, label: &str) -> bool {
        matches!(
            label,
            "Fetching characters" | "Importing passive tree" | "Importing items and skills"
        ) && self.account.as_ref().is_some_and(|a| a.oauth.signed_in)
    }

    /// Some pending request, if cancelled, may sign the user out.
    pub fn pending_signs_out(&self) -> bool {
        self.pending.iter().any(|p| self.cancel_signs_out(&p.label))
    }

    /// Starts a plan: its first command goes now, the second after the first request ends.
    pub fn begin(&mut self, plan: Plan) -> Command {
        self.chain = plan.then.map(|rest| Chain { head: None, rest });
        plan.first
    }

    /// Sends the import box's edit still being typed.
    pub fn flush(&mut self) -> Option<Command> {
        self.code_dirty.take()?;
        Some(Command::SetImportCode {
            text: self.code.clone(),
        })
    }

    /// The tab is not showing: an answer that was to be copied is no longer wanted.
    pub fn leave(&mut self) -> Option<Command> {
        self.await_code = false;
        self.copy_ready = None;
        self.flush()
    }

    #[cfg(test)]
    pub(crate) fn type_for_test(&mut self, text: &str, now: f64) {
        self.code = text.to_owned();
        self.code_dirty = Some(now);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, now_epoch: i64) -> ImportOutput {
        let mut out = ImportOutput {
            open_url: self.open_ready.take(),
            copy: self.copy_ready.take(),
            ..ImportOutput::default()
        };
        // Ctrl+C on the tab copies the build's code; a focused text box keeps its own copy
        if !self.modal && !ui.ctx().text_edit_focused() {
            let copied = ui.input_mut(|i| {
                let key = i.consume_key(egui::Modifiers::COMMAND, egui::Key::C);
                let events = i.events.len();
                i.events.retain(|e| !matches!(e, egui::Event::Copy));
                key || events != i.events.len()
            });
            if copied {
                out.commands.push(Command::ExportShareCode);
            }
        }
        self.try_fetch(&mut out);
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.account_panel(ui, now_epoch, &mut out);
            ui.add_space(8.0);
            self.code_panel(ui, &mut out);
            ui.add_space(8.0);
            self.share_panel(ui, &mut out);
        });
        out
    }

    /// PoB's `TryFetchCharacterList` (ImportTab.lua:734-739): opening the tab with its button
    /// (not Ctrl+I, Build.lua:437, 1159) presses Fetch once for the remembered realm. Signed
    /// out, the press does nothing but the try is spent, as PoB's.
    fn try_fetch(&mut self, out: &mut ImportOutput) {
        if self.auto_fetched || self.focus_code || self.modal {
            return;
        }
        let Some(account) = self.account.clone() else {
            return;
        };
        let oauth = &account.oauth;
        if !oauth.remembered_realm {
            return;
        }
        self.auto_fetched = true;
        let running = oauth.busy
            || oauth.signing_in
            || self
                .pending
                .iter()
                .any(|p| p.label == "Signing in" || p.label == "Fetching characters");
        if oauth.signed_in && !oauth.fetched && !running {
            self.realm = oauth.realm;
            out.commands
                .push(Command::ListCharacters { realm: oauth.realm });
        }
    }

    // ---- the account ---------------------------------------------------------------------

    fn account_panel(&mut self, ui: &mut egui::Ui, now: i64, out: &mut ImportOutput) {
        theme::heading(ui, "Import from a Path of Exile account");
        let Some(account) = self.account.clone() else {
            ui.spinner();
            return;
        };
        // the drop-down starts on PoB's remembered realm (ImportTab.lua:204, 339)
        if !self.realm_seen {
            self.realm_seen = true;
            self.realm = account.oauth.realm;
        }
        ui.horizontal(|ui| {
            let before = self.realm;
            let combo = crate::widgets::dropdown::Dropdown::from_id_salt("import_realm")
                .selected_text(realm_label(self.realm))
                .show_ui(ui, |ui| {
                    for realm in [Realm::Pc, Realm::Xbox, Realm::Sony] {
                        let r = crate::widgets::dropdown::value(
                            ui,
                            &mut self.realm,
                            realm,
                            realm_label(realm),
                        );
                        mark(ui, &format!("realm:{}", realm.pob_id()), &r);
                    }
                });
            mark(ui, "realm_combo", &combo.response);
            // the wheel and the arrows step it (DropDownControl.lua:476-516)
            let realms = [Realm::Pc, Realm::Xbox, Realm::Sony];
            let at = realms.iter().position(|r| *r == self.realm).unwrap_or(0);
            if let Some(i) =
                crate::widgets::scroll_step::combo_step(ui, &combo.response, realms.len(), at)
            {
                self.realm = realms[i];
            }
            if before != self.realm {
                self.league = None;
                self.selected = None;
            }
            ui.separator();
            let before = self.oauth_mode;
            let r = ui.selectable_value(&mut self.oauth_mode, true, "OAuth");
            mark(ui, "mode:oauth", &r);
            let r = ui.selectable_value(&mut self.oauth_mode, false, "Account name");
            mark(ui, "mode:name", &r);
            if before != self.oauth_mode {
                self.league = None;
                self.selected = None;
            }
        });
        if self.oauth_mode {
            self.oauth_panel(ui, &account, now, out);
        } else {
            self.public_panel(ui, &account, out);
        }
    }

    fn pending_id(&self, label: &str) -> Option<u64> {
        self.pending.iter().find(|p| p.label == label).map(|p| p.id)
    }

    fn oauth_panel(
        &mut self,
        ui: &mut egui::Ui,
        account: &AccountView,
        now: i64,
        out: &mut ImportOutput,
    ) {
        let oauth = &account.oauth;
        if !oauth.signed_in {
            if oauth.signing_in || self.sign_seq.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    match oauth.login_deadline {
                        Some(deadline) => ui.label(format!(
                            "Waiting for the browser… ({} s)",
                            (deadline - now).max(0)
                        )),
                        None => ui.label("Waiting for the browser…"),
                    };
                    let id = self.pending_id("Signing in");
                    let r = ui.add_enabled(id.is_some(), egui::Button::new("Cancel"));
                    mark(ui, "signin_cancel", &r);
                    if r.clicked()
                        && let Some(id) = id
                    {
                        out.commands.push(Command::CancelRequest { id });
                    }
                    let url = oauth.auth_url.clone();
                    let r = ui.add_enabled(url.is_some(), egui::Button::new("Copy link"));
                    mark(ui, "copy_link", &r);
                    if r.clicked() {
                        out.copy = url;
                    }
                });
                if oauth.login_deadline.is_some() {
                    ui.ctx().request_repaint_after(Duration::from_secs(1));
                }
            } else {
                let r = ui.add(theme::primary_button("Sign in with Path of Exile"));
                mark(ui, "sign_in", &r);
                if r.clicked() {
                    out.commands.push(Command::SignIn);
                }
            }
            Self::status_lines(
                ui,
                &oauth.status,
                &oauth.status_coded,
                oauth.error.as_deref(),
            );
            return;
        }
        let picks = (
            oauth.realm,
            oauth.league_selected.clone(),
            oauth.character_selected.clone(),
        );
        // Fetch: not while a sign-in or another fetch runs (the engine would refuse)
        let fetched = oauth.fetched && oauth.realm == self.realm;
        let waiting = oauth.retry_at.map_or(0, |at| (at - now).max(0));
        let running = oauth.busy
            || oauth.signing_in
            || self
                .pending
                .iter()
                .any(|p| p.label == "Signing in" || p.label == "Fetching characters");
        ui.horizontal(|ui| {
            let label = if fetched {
                "Fetched"
            } else {
                "Fetch characters"
            };
            let r = ui.add_enabled(
                !running && !fetched && waiting == 0,
                egui::Button::new(label),
            );
            mark(ui, "fetch", &r);
            if r.clicked() {
                out.commands
                    .push(Command::ListCharacters { realm: self.realm });
            }
            if running {
                ui.spinner();
            }
            let r = ui.button("Sign out");
            mark(ui, "sign_out", &r);
            if r.clicked() {
                out.commands.push(Command::SignOut);
            }
        });
        if waiting > 0 {
            ui.colored_label(ui.visuals().warn_fg_color, format!("Retry in {waiting} s"));
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        Self::status_lines(
            ui,
            &oauth.status,
            &oauth.status_coded,
            oauth.error.as_deref(),
        );
        if fetched {
            let realm = self.realm;
            let importing = oauth.busy;
            self.character_list(
                ui,
                (true, picks),
                &oauth.leagues,
                &oauth.characters,
                importing,
                out,
                |league, name, part| Command::ImportCharacter {
                    realm,
                    league,
                    name,
                    part,
                },
            );
        }
    }

    fn public_panel(&mut self, ui: &mut egui::Ui, account: &AccountView, out: &mut ImportOutput) {
        let public = &account.public;
        match public.stage {
            PublicStage::EnterAccount => {
                // PoB's line above the box (ImportTab.lua:333-337)
                ui.label("To start importing a character, enter the character's account name:");
                ui.horizontal(|ui| {
                    let opts = EditBoxOptions {
                        width: 220.0,
                        hint: Some("Account name#1234"),
                        ..EditBoxOptions::default()
                    };
                    let current = if self.account_seen.as_ref() == Some(&public.account) {
                        self.account_box.text.clone()
                    } else {
                        self.account_seen = Some(public.account.clone());
                        public.account.clone()
                    };
                    let (r, _) = self
                        .account_box
                        .show(ui, self.account_id, &current, false, &opts);
                    mark(ui, "account_name", &r);
                    // its "x" empties it (ImportTab.lua:341, clearable)
                    if let Some(x) =
                        crate::widgets::edit_box::clear_button(ui, &mut self.account_box.text)
                    {
                        mark(ui, "account_clear", &x);
                        if x.clicked() {
                            // the box keeps the keyboard, so the view doesn't refill it
                            ui.memory_mut(|m| m.request_focus(self.account_id));
                        }
                    }
                    // PoB enables Start only for a name with its discriminator (NF-8)
                    let (_, can_start) = account_name_state(&self.account_box.text);
                    let name = self.account_box.text.trim().to_owned();
                    let mut start = ui.add_enabled(can_start, theme::primary_button("Start"));
                    // PoB's tip on Start for a name without its discriminator
                    // (ImportTab.lua:370-375)
                    if account_name_state(&self.account_box.text).0 {
                        let tip =
                            format!("Missing discriminator e.g. {}#1234", self.account_box.text);
                        start = start.on_disabled_hover_text(tip);
                    }
                    mark(ui, "start", &start);
                    // Only Start asks for the list: PoB's box has no Enter function
                    // (ImportTab.lua:340-341), and leaving it is no request either
                    if start.clicked() {
                        out.commands.push(Command::ListPublicCharacters {
                            account: name,
                            realm: self.realm,
                        });
                    }
                    // PoB's recent accounts: a pick fills the box, X forgets the picked one
                    if !public.history.is_empty() {
                        // a pick of ours, else PoB's own selection (its last account used)
                        let current = self
                            .history_pick
                            .clone()
                            .or_else(|| public.history_selected.clone())
                            .filter(|a| public.history.contains(a));
                        let combo =
                            crate::widgets::dropdown::Dropdown::from_id_salt("import_history")
                                .selected_text(current.as_deref().unwrap_or("Recent"))
                                .show_ui(ui, |ui| {
                                    for account in &public.history {
                                        let item = crate::widgets::dropdown::item(
                                            ui,
                                            current.as_ref() == Some(account),
                                            account,
                                        );
                                        mark(ui, &format!("history:{account}"), &item);
                                        if item.clicked() {
                                            // PoB's pick puts the name in the box; it stays there
                                            // while the box has focus, until Start or Enter
                                            self.history_pick = Some(account.clone());
                                            self.account_box.text = account.clone();
                                            ui.memory_mut(|m| m.request_focus(self.account_id));
                                        }
                                    }
                                });
                        mark(ui, "history", &combo.response);
                        // the wheel and the arrows step it as a pick (DropDownControl.lua)
                        let at = current
                            .as_ref()
                            .and_then(|c| public.history.iter().position(|a| a == c))
                            .unwrap_or(0);
                        if let Some(i) = crate::widgets::scroll_step::combo_step(
                            ui,
                            &combo.response,
                            public.history.len(),
                            at,
                        ) {
                            // as the click: the name goes in the box, which takes the keyboard
                            let account = public.history[i].clone();
                            self.history_pick = Some(account.clone());
                            self.account_box.text = account;
                            ui.memory_mut(|m| m.request_focus(self.account_id));
                        }
                        let picked = current;
                        let remove = ui
                            .add_enabled(picked.is_some(), egui::Button::new("X"))
                            .on_hover_text("Removes account from the dropdown list");
                        mark(ui, "history_remove", &remove);
                        if remove.clicked()
                            && let Some(account) = picked
                        {
                            self.history_pick = None;
                            out.commands.push(Command::RemoveAccountHistory { account });
                        }
                    }
                });
                if account_name_state(&self.account_box.text).0 {
                    // PoB's "^1" (red) label
                    ui.label(crate::widgets::pob_text(
                        &format!("^1{}", public.missing_discriminator),
                        14.0,
                        ui.visuals().text_color(),
                    ));
                }
                Self::status_lines(ui, &public.status, &public.status_coded, None);
            }
            PublicStage::Fetching | PublicStage::Importing => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&public.status);
                });
            }
            PublicStage::SelectCharacter => {
                Self::status_lines(ui, &public.status, &public.status_coded, None);
                let picks = (
                    public.realm,
                    public.league_selected.clone(),
                    public.character_selected.clone(),
                );
                self.character_list(
                    ui,
                    (false, picks),
                    &public.leagues,
                    &public.characters,
                    false,
                    out,
                    |league, name, part| Command::ImportPublicCharacter { league, name, part },
                );
                if !public.bandit_note.is_empty() {
                    ui.label(&public.bandit_note);
                }
                let r = ui.button("Close");
                mark(ui, "close_public", &r);
                if r.clicked() {
                    out.commands.push(Command::ClosePublicCharacters);
                }
            }
        }
    }

    /// PoB's "Character import status: ..." label and its "Error: ..." label in its
    /// `NEGATIVE` colour (ImportTab.lua:78-81, 172-176)
    fn status_lines(ui: &mut egui::Ui, status: &str, coded: &str, error: Option<&str>) {
        if !status.is_empty() {
            // PoB's colours: a warning or a failure is coloured (`WARNING`, `NEGATIVE`)
            let text = if coded.is_empty() { status } else { coded };
            ui.label(crate::widgets::pob_text(
                &format!("^7Character import status: {text}"),
                14.0,
                ui.visuals().text_color(),
            ));
        }
        if let Some(error) = error {
            ui.colored_label(theme::NEGATIVE, format!("Error: {error}"));
        }
    }

    /// The league filter, the characters, PoB's import options and the Import button. `picks`
    /// is which section (OAuth or not) and PoB's realm, league and character picks: a new list
    /// starts on them, as PoB's drop-downs do.
    #[allow(clippy::too_many_arguments)]
    fn character_list(
        &mut self,
        ui: &mut egui::Ui,
        (oauth, (realm, league, character)): (bool, (Realm, Option<String>, Option<String>)),
        leagues: &[String],
        characters: &[CharacterEntry],
        importing: bool,
        out: &mut ImportOutput,
        command: impl Fn(Option<String>, String, CharacterPart) -> Command,
    ) {
        let key = (
            oauth,
            realm,
            characters.iter().map(|c| c.name.clone()).collect(),
        );
        if self.list_seen.as_ref() != Some(&key) {
            // PoB's picks are for its own realm; one of ours not fetched yet has none
            if realm == self.realm {
                self.league = league;
                self.selected = character;
            }
            self.list_seen = Some(key);
        }
        // PoB's line above the list (ImportTab.lua:166, 410)
        ui.label("Choose character to import data from:");
        ui.horizontal(|ui| {
            ui.label("League:");
            let combo = crate::widgets::dropdown::Dropdown::from_id_salt("import_league")
                .selected_text(self.league.as_deref().unwrap_or("Any"))
                .show_ui(ui, |ui| {
                    for index in 0..leagues.len() {
                        let value = league_value(leagues, index);
                        let r = crate::widgets::dropdown::value(
                            ui,
                            &mut self.league,
                            value,
                            &leagues[index],
                        );
                        mark(ui, &format!("league:{}", leagues[index]), &r);
                    }
                });
            mark(ui, "league_combo", &combo.response);
            let at = (0..leagues.len())
                .position(|i| league_value(leagues, i) == self.league)
                .unwrap_or(0);
            if let Some(i) =
                crate::widgets::scroll_step::combo_step(ui, &combo.response, leagues.len(), at)
            {
                self.league = league_value(leagues, i);
            }
        });
        let visible: Vec<&CharacterEntry> = characters
            .iter()
            .filter(|c| self.league.as_ref().is_none_or(|l| &c.league == l))
            .collect();
        if self
            .selected
            .as_ref()
            .is_some_and(|s| !visible.iter().any(|c| &c.name == s))
        {
            self.selected = None;
        }
        egui::ScrollArea::vertical()
            .id_salt("import_characters")
            .max_height(180.0)
            .show(ui, |ui| {
                for c in &visible {
                    let selected = self.selected.as_ref() == Some(&c.name);
                    // PoB's entry: the name, then the class in its colour and the level in
                    // grey, with the league when no league is picked (ImportTab.lua:1068-1073)
                    let colour = if c.colour.is_empty() { "^7" } else { &c.colour };
                    let mut detail = format!("{colour}{} ^x808080lvl {}", c.class, c.level);
                    if self.league.is_none() {
                        detail.push_str(&format!(" in {}", c.league));
                    }
                    let default = ui.visuals().text_color();
                    let text = format!("^7{}   {detail}", c.name);
                    let r = ui
                        .selectable_label(selected, crate::widgets::pob_text(&text, 14.0, default));
                    mark(ui, &format!("char:{}", c.name), &r);
                    if r.clicked() {
                        self.selected = Some(c.name.clone());
                    }
                }
            });
        // PoB's two buttons' names and their option boxes, with PoB's labels and tips
        ui.checkbox(&mut self.part_tree, "Passive Tree and Jewels");
        ui.indent("import_tree_opts", |ui| {
            let r = ui
                .checkbox(&mut self.delete_jewels, "Delete jewels")
                .on_hover_text(TIP_JEWELS);
            mark(ui, "opt:jewels", &r);
        });
        ui.checkbox(&mut self.part_items, "Items and Skills");
        ui.indent("import_item_opts", |ui| {
            for (value, label, tip, key) in [
                (
                    &mut self.clear_skills,
                    "Delete skills",
                    TIP_SKILLS,
                    "opt:skills",
                ),
                (
                    &mut self.clear_items,
                    "Delete equipment",
                    TIP_ITEMS,
                    "opt:items",
                ),
                (
                    &mut self.ignore_weapon_swap,
                    "Ignore weapon swap",
                    TIP_SWAP,
                    "opt:swap",
                ),
            ] {
                let r = ui.checkbox(value, label).on_hover_text(tip);
                mark(ui, key, &r);
            }
        });
        let can = self.selected.is_some() && (self.part_tree || self.part_items) && !importing;
        let r = ui.add_enabled(can, theme::primary_button("Import"));
        mark(ui, "import_character", &r);
        if r.clicked()
            && let Some(name) = self.selected.clone()
        {
            let tree = self.part_tree.then(|| {
                command(
                    self.league.clone(),
                    name.clone(),
                    CharacterPart::Tree {
                        delete_jewels: self.delete_jewels,
                    },
                )
            });
            let items = self.part_items.then(|| {
                command(
                    self.league.clone(),
                    name.clone(),
                    CharacterPart::ItemsAndSkills {
                        clear_items: self.clear_items,
                        clear_skills: self.clear_skills,
                        ignore_weapon_swap: self.ignore_weapon_swap,
                    },
                )
            });
            let asks = tree.is_some() && self.tree_used;
            let plan = match (tree, items) {
                (Some(first), then) => Plan { first, then },
                (None, Some(first)) => Plan { first, then: None },
                (None, None) => return,
            };
            if asks {
                out.confirm = Some(Confirm::Ask {
                    message: TREE_OVERWRITE,
                    plan,
                });
            } else {
                out.commands.push(self.begin(plan));
            }
        }
    }

    // ---- the import box ------------------------------------------------------------------

    fn code_panel(&mut self, ui: &mut egui::Ui, out: &mut ImportOutput) {
        theme::heading(ui, "Import from a link or share code");
        let now = ui.input(|i| i.time);
        // Enter is PoB's `enterFunc` (Import when the text is valid, ImportTab.lua:671-675), so
        // only Shift+Enter breaks the line
        let entered = ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.shift
                )
            })
        });
        let r = ui.add(
            egui::TextEdit::multiline(&mut self.code)
                .id_salt("import_code_box")
                .hint_text("Paste a build link (pobb.in, pastebin, …) or a PoB share code")
                .return_key(Some(egui::KeyboardShortcut::new(
                    egui::Modifiers::SHIFT,
                    egui::Key::Enter,
                )))
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        let entered = entered && r.has_focus();
        mark(ui, "code_box", &r);
        crate::widgets::url_click::open_on_ctrl_click(ui, &r, &self.code);
        if std::mem::take(&mut self.focus_code) {
            r.request_focus();
        }
        // its "x" empties it (ImportTab.lua:670, clearable)
        let cleared = crate::widgets::edit_box::clear_button(ui, &mut self.code);
        if let Some(x) = &cleared {
            mark(ui, "code_clear", x);
        }
        if r.changed() || cleared.is_some_and(|x| x.clicked()) {
            self.code_dirty = Some(now);
            self.enter_after = None;
        }
        if let Some(since) = self.code_dirty {
            let left = DEBOUNCE - (now - since);
            if left <= 0.0 {
                self.code_dirty = None;
                out.commands.push(Command::SetImportCode {
                    text: self.code.clone(),
                });
            } else {
                ui.ctx()
                    .request_repaint_after(Duration::from_secs_f64(left));
            }
        }
        let share = self.share.clone();
        let Some(view) = share.as_deref().map(|s| &s.import) else {
            return;
        };
        let stale = self.code_dirty.is_some();
        if !stale && !self.code.trim().is_empty() && !view.detail.is_empty() {
            ui.colored_label(detail_colour(view.valid), &view.detail);
        }
        // PoB offers both only for a build with a file; its box puts a build without one on
        // "Import to a new build" (ImportTab.lua:585-587)
        if !view.can_import_to_this_build {
            self.mode = Some(ImportMode::NewBuild);
        }
        let mut mode = self.mode.unwrap_or(ImportMode::ThisBuild);
        let ok = view.valid && !view.fetching && !stale && !self.code.trim().is_empty();
        ui.horizontal(|ui| {
            // PoB's drop-down waits for a valid text (ImportTab.lua:680-682)
            ui.add_enabled_ui(ok, |ui| {
                if view.can_import_to_this_build {
                    let r =
                        ui.radio_value(&mut mode, ImportMode::ThisBuild, "Import to this build");
                    mark(ui, "mode:this", &r);
                }
                let r = ui.radio_value(&mut mode, ImportMode::NewBuild, "Import to a new build");
                mark(ui, "mode:new", &r);
                if view.can_import_to_this_build {
                    let r =
                        ui.radio_value(&mut mode, ImportMode::Comparison, "Import as comparison");
                    mark(ui, "mode:compare", &r);
                }
            });
        });
        if Some(mode) != self.mode && (self.mode.is_some() || mode != ImportMode::ThisBuild) {
            self.mode = Some(mode);
        }
        ui.horizontal(|ui| {
            // PoB's button says so while the paste is fetched (ImportTab.lua:701-703)
            let label = if view.fetching {
                "Retrieving paste.."
            } else {
                "Import"
            };
            let r = ui.add_enabled(ok, theme::primary_button(label));
            mark(ui, "import_code", &r);
            if view.fetching {
                ui.spinner();
            }
            // Enter on text still being judged: it goes now, and Enter acts on PoB's verdict
            if entered && !ok && stale {
                self.code_dirty = None;
                self.enter_after = Some(self.code_sent_seq);
                out.commands.push(Command::SetImportCode {
                    text: self.code.clone(),
                });
            }
            let mut act = r.clicked() || (entered && ok);
            if let Some(before) = self.enter_after
                && self.code_sent_seq > before
                && self.share_seq >= self.code_sent_seq
                && !view.fetching
            {
                self.enter_after = None;
                act |= ok;
            }
            if act {
                let url = self.code.trim().to_owned();
                match mode {
                    ImportMode::ThisBuild => {
                        out.confirm = Some(Confirm::Ask {
                            message: BUILD_OVERWRITE,
                            plan: Plan {
                                first: Command::ImportFromUrl {
                                    url,
                                    mode: ImportMode::ThisBuild,
                                },
                                then: None,
                            },
                        });
                    }
                    ImportMode::NewBuild => out.confirm = Some(Confirm::NewBuild { url }),
                    // the build stays as it is; PoB then shows its Compare tab
                    ImportMode::Comparison => out.commands.push(Command::ImportFromUrl {
                        url,
                        mode: ImportMode::Comparison,
                    }),
                }
            }
        });
    }

    // ---- share ---------------------------------------------------------------------------

    fn share_panel(&mut self, ui: &mut egui::Ui, out: &mut ImportOutput) {
        theme::heading(ui, "Share this build");
        let share = self.share.clone();
        let Some(view) = share.as_deref() else {
            ui.spinner();
            return;
        };
        let chosen = self
            .site
            .clone()
            .filter(|id| view.sites.iter().any(|s| &s.id == id))
            .or_else(|| {
                view.last_site
                    .clone()
                    .filter(|id| view.sites.iter().any(|s| &s.id == id))
            })
            .or_else(|| view.sites.first().map(|s| s.id.clone()));
        ui.horizontal(|ui| {
            let label = |id: &Option<String>| {
                view.sites
                    .iter()
                    .find(|s| Some(&s.id) == id.as_ref())
                    .map_or("", |s: &SiteRef| s.label.as_str())
                    .to_owned()
            };
            let combo = crate::widgets::dropdown::Dropdown::from_id_salt("import_site")
                .selected_text(label(&chosen))
                .show_ui(ui, |ui| {
                    for site in &view.sites {
                        let mut value = chosen.clone();
                        let r = crate::widgets::dropdown::value(
                            ui,
                            &mut value,
                            Some(site.id.clone()),
                            &site.label,
                        );
                        mark(ui, &format!("site:{}", site.id), &r);
                        if r.clicked() {
                            self.site = Some(site.id.clone());
                        }
                    }
                });
            mark(ui, "site_combo", &combo.response);
            let at = view
                .sites
                .iter()
                .position(|s| Some(&s.id) == chosen.as_ref())
                .unwrap_or(0);
            if let Some(i) =
                crate::widgets::scroll_step::combo_step(ui, &combo.response, view.sites.len(), at)
            {
                self.site = Some(view.sites[i].id.clone());
            }
            let r = ui.add_enabled(
                chosen.is_some() && !view.sharing,
                theme::primary_button("Share"),
            );
            mark(ui, "share", &r);
            if r.clicked()
                && let Some(site) = chosen.clone()
            {
                out.commands.push(Command::ShareBuild { site });
            }
            if view.sharing {
                ui.spinner();
            }
        });
        if let Some(link) = &view.link {
            ui.horizontal(|ui| {
                ui.label(link);
                let r = ui.button("Copy");
                mark(ui, "copy_link_result", &r);
                if r.clicked() {
                    out.copy = Some(link.clone());
                }
            });
        }
        let r = ui
            .button("Copy code")
            .on_hover_text("Copies the build's share code (Ctrl+C)");
        mark(ui, "copy_code", &r);
        if r.clicked() {
            out.commands.push(Command::ExportShareCode);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engine::views::{ImportCodeView, OAuthView, PublicView, RequestsView};
    use std::sync::Arc;

    const URL: &str = "https://www.pathofexile.com/oauth/authorize?client_id=x&state=y";

    /// Texts drawn in a frame.
    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        galleys(shapes)
            .iter()
            .map(|g| g.text().to_owned())
            .collect()
    }

    /// The text galleys drawn in a frame.
    fn galleys(shapes: &[egui::epaint::ClippedShape]) -> Vec<Arc<egui::Galley>> {
        fn walk(shape: &egui::epaint::Shape, out: &mut Vec<Arc<egui::Galley>>) {
            match shape {
                egui::epaint::Shape::Text(t) => out.push(t.galley.clone()),
                egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        shapes.iter().for_each(|c| walk(&c.shape, &mut out));
        out
    }

    /// Drives the tab like the app does: it numbers and logs every command and tells the tab.
    pub(crate) struct Harness {
        pub ui: ImportUi,
        pub ctx: egui::Context,
        pub time: f64,
        pub now: i64,
        pub seq: u64,
        pub log: Vec<String>,
        pub confirms: Vec<Confirm>,
        pub opened: Vec<String>,
        pub copied: Vec<String>,
        pub drawn: Vec<String>,
        pub galleys: Vec<Arc<egui::Galley>>,
    }

    impl Harness {
        pub fn new() -> Self {
            let mut h = Harness {
                ui: ImportUi::default(),
                ctx: egui::Context::default(),
                time: 1.0,
                now: 1_000,
                seq: 0,
                log: Vec::new(),
                confirms: Vec::new(),
                opened: Vec::new(),
                copied: Vec::new(),
                drawn: Vec::new(),
                galleys: Vec::new(),
            };
            h.account(signed_in());
            h.share(share());
            h
        }

        pub fn account(&mut self, view: AccountView) {
            self.seq += 1;
            self.ui
                .on_snapshot(&Snapshot::Account(Arc::new(view)), self.seq);
        }

        pub fn share(&mut self, view: ShareView) {
            self.seq += 1;
            self.ui
                .on_snapshot(&Snapshot::Share(Arc::new(view)), self.seq);
        }

        pub fn frame(&mut self, events: Vec<egui::Event>) {
            clear_marks();
            self.time += 0.016;
            let input = egui::RawInput {
                events,
                time: Some(self.time),
                ..Default::default()
            };
            let mut out = ImportOutput::default();
            let full = self.ctx.run_ui(input, |ui| {
                out = self.ui.show(ui, self.now);
            });
            self.drawn = texts(&full.shapes);
            self.galleys = galleys(&full.shapes);
            full.drop_without_applying_deltas();
            for command in self.ui.take_ready().into_iter().chain(out.commands) {
                self.seq += 1;
                self.ui.sent(self.seq, &command);
                self.log.push(format!("{command:?}"));
            }
            self.confirms.extend(out.confirm);
            self.opened.extend(out.open_url);
            self.copied.extend(out.copy);
        }

        pub fn click(&mut self, key: &str) {
            let pos = marked(key)
                .unwrap_or_else(|| panic!("no widget {key}"))
                .center();
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![egui::Event::PointerMoved(pos)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.frame(vec![]);
        }

        pub fn idle_for(&mut self, seconds: f64) {
            self.time += seconds;
            self.frame(vec![]);
        }

        /// The colour of the drawn section that reads `section`
        pub fn colour_of(&self, section: &str) -> Option<egui::Color32> {
            self.galleys.iter().find_map(|g| {
                g.job
                    .sections
                    .iter()
                    .find(|s| {
                        let range = usize::from(s.byte_range.start)..usize::from(s.byte_range.end);
                        &g.job.text[range] == section
                    })
                    .map(|s| s.format.color)
            })
        }

        pub fn has_text(&self, needle: &str) -> bool {
            self.drawn.iter().any(|t| t.contains(needle))
        }
    }

    fn ch(name: &str, league: &str) -> CharacterEntry {
        CharacterEntry {
            name: name.into(),
            class: "Witch".into(),
            level: 90,
            league: league.into(),
            colour: "^x7070FF".into(),
        }
    }

    fn signed_in() -> AccountView {
        AccountView {
            oauth: OAuthView {
                signed_in: true,
                fetched: true,
                leagues: vec!["Standard".into(), "Settlers".into(), "Any".into()],
                characters: vec![
                    ch("Alice", "Standard"),
                    ch("Bob", "Settlers"),
                    ch("Cara", "Standard"),
                ],
                ..OAuthView::default()
            },
            public: PublicView::default(),
        }
    }

    fn signed_out() -> AccountView {
        AccountView::default()
    }

    fn share() -> ShareView {
        ShareView {
            sites: vec![
                SiteRef {
                    id: "POBBin".into(),
                    label: "pobb.in".into(),
                },
                SiteRef {
                    id: "Pastebin".into(),
                    label: "pastebin.com".into(),
                },
            ],
            import: ImportCodeView {
                can_import_to_this_build: true,
                ..ImportCodeView::default()
            },
            ..ShareView::default()
        }
    }

    fn waiting(url: Option<&str>) -> AccountView {
        AccountView {
            oauth: OAuthView {
                signing_in: true,
                auth_url: url.map(str::to_owned),
                login_deadline: Some(1_042),
                ..OAuthView::default()
            },
            ..AccountView::default()
        }
    }

    #[test]
    fn sign_in_opens_the_auth_url_once() {
        let mut h = Harness::new();
        h.account(signed_out());
        h.frame(vec![]);
        h.click("sign_in");
        assert_eq!(h.log, ["SignIn"]);
        assert!(
            h.opened.is_empty(),
            "nothing opens before the engine answers"
        );
        // the answer, then more snapshots of the same sign-in
        for _ in 0..3 {
            h.account(waiting(Some(URL)));
            h.frame(vec![]);
        }
        assert_eq!(h.opened, [URL]);
        // the wait is shown with PoB's deadline counting down, and offers Cancel and Copy link
        assert!(
            h.has_text("Waiting for the browser… (42 s)"),
            "{:?}",
            h.drawn
        );
        h.now = 1_030;
        h.frame(vec![]);
        assert!(h.has_text("(12 s)"), "{:?}", h.drawn);
        assert!(marked("signin_cancel").is_some() && marked("copy_link").is_some());
    }

    #[test]
    fn a_link_nobody_asked_for_is_not_opened() {
        let mut h = Harness::new();
        // an auth_url in a snapshot, with no Sign in click of ours
        h.account(waiting(Some(URL)));
        h.frame(vec![]);
        assert!(h.opened.is_empty());
    }

    #[test]
    fn only_the_poe_authorize_url_is_opened() {
        for bad in [
            "https://evil.example/oauth/authorize?x=1",
            "https://www.pathofexile.com/account/view-profile",
            "http://www.pathofexile.com/oauth/authorize?x=1",
        ] {
            let mut h = Harness::new();
            h.account(signed_out());
            h.frame(vec![]);
            h.click("sign_in");
            h.account(waiting(Some(bad)));
            h.frame(vec![]);
            assert!(h.opened.is_empty(), "{bad} was opened");
        }
    }

    #[test]
    fn copy_link_copies_the_sign_in_link() {
        let mut h = Harness::new();
        h.account(signed_out());
        h.frame(vec![]);
        h.click("sign_in");
        h.account(waiting(Some(URL)));
        h.frame(vec![]);
        h.click("copy_link");
        assert_eq!(h.copied, [URL]);
    }

    #[test]
    fn focus_is_requested_when_sign_in_completes() {
        let mut h = Harness::new();
        h.account(signed_out());
        h.frame(vec![]);
        assert!(!h.ui.take_focus());
        h.click("sign_in");
        h.account(waiting(Some(URL)));
        assert!(!h.ui.take_focus(), "waiting is not signed in");
        h.account(signed_in());
        assert!(h.ui.take_focus());
        assert!(!h.ui.take_focus(), "once");
        // a later snapshot that is signed in does not ask again
        h.account(signed_in());
        assert!(!h.ui.take_focus());
    }

    #[test]
    fn no_focus_when_signed_in_without_our_click() {
        let mut h = Harness::new();
        h.account(signed_out());
        h.account(signed_in());
        assert!(!h.ui.take_focus());
    }

    #[test]
    fn a_cancelled_sign_in_asks_for_no_focus_later() {
        let mut h = Harness::new();
        h.account(signed_out());
        h.frame(vec![]);
        h.click("sign_in");
        h.account(waiting(Some(URL)));
        h.account(signed_out());
        h.account(signed_in());
        assert!(!h.ui.take_focus());
    }

    pub(crate) fn tree_snapshot(used: u32, ascendancy_used: u32) -> Snapshot {
        let state: crate::engine::views::TreeState = serde_json::from_value(serde_json::json!({
            "version": "3_28", "version_label": "3.28", "class_id": 0, "ascendancy_id": 0,
            "points": {"used": used, "available": 100,
                       "ascendancy_used": ascendancy_used, "ascendancy_available": 8},
            "active_spec": 0, "can_undo": false, "can_redo": false
        }))
        .expect("a tree state");
        Snapshot::Tree(Arc::new(state))
    }

    fn deliver_tree(h: &mut Harness, used: u32, ascendancy_used: u32) {
        h.seq += 1;
        h.ui.on_snapshot(&tree_snapshot(used, ascendancy_used), h.seq);
    }

    fn pick_character(h: &mut Harness, name: &str) {
        h.frame(vec![]);
        h.click(&format!("char:{name}"));
    }

    fn account_with(busy: bool, retry_at: Option<i64>) -> AccountView {
        let mut view = signed_in();
        view.oauth.busy = busy;
        view.oauth.retry_at = retry_at;
        view
    }

    #[test]
    fn import_sends_tree_then_items_and_asks_before_overwriting() {
        let mut h = Harness::new();
        pick_character(&mut h, "Alice");
        // uncheck "Ignore weapon swap"'s opposite so every option is non-default somewhere
        h.ui.ignore_weapon_swap = true;
        h.ui.clear_items = false;
        deliver_tree(&mut h, 3, 0);
        h.click("import_character");
        // the tree has nodes: nothing goes before the question
        assert!(h.log.is_empty(), "{:?}", h.log);
        let Some(Confirm::Ask { message, plan }) = h.confirms.pop() else {
            panic!("expected a question");
        };
        assert_eq!(
            message,
            "Importing the passive tree will overwrite your current tree."
        );
        let first = h.ui.begin(plan);
        h.seq += 1;
        h.ui.sent(h.seq, &first);
        assert_eq!(
            format!("{first:?}"),
            "ImportCharacter { realm: Pc, league: None, name: \"Alice\", part: Tree { delete_jewels: true } }"
        );
        // the tree request runs: the items wait
        let head = h.seq;
        let mut view = account_with(true, None);
        h.seq = head;
        h.ui.on_snapshot(&Snapshot::Account(Arc::new(view.clone())), head);
        h.frame(vec![]);
        assert!(
            h.log.is_empty(),
            "items went before the tree ended: {:?}",
            h.log
        );
        // an older snapshot, from before the tree command, is no end
        h.ui.on_snapshot(
            &Snapshot::Account(Arc::new(account_with(false, None))),
            head - 1,
        );
        h.frame(vec![]);
        assert!(h.log.is_empty(), "{:?}", h.log);
        // the tree request ends
        view.oauth.busy = false;
        h.ui.on_snapshot(&Snapshot::Account(Arc::new(view)), head + 1);
        h.frame(vec![]);
        assert_eq!(
            h.log,
            [
                "ImportCharacter { realm: Pc, league: None, name: \"Alice\", part: ItemsAndSkills { clear_items: false, clear_skills: true, ignore_weapon_swap: true } }"
            ]
        );
        // and once
        h.ui.on_snapshot(
            &Snapshot::Account(Arc::new(account_with(false, None))),
            head + 2,
        );
        h.frame(vec![]);
        assert_eq!(h.log.len(), 1);
    }

    #[test]
    fn an_empty_tree_imports_without_asking() {
        let mut h = Harness::new();
        pick_character(&mut h, "Alice");
        h.click("import_character");
        assert!(h.confirms.is_empty());
        assert_eq!(h.log.len(), 1);
        assert!(
            h.log[0].contains("Tree { delete_jewels: true }"),
            "{:?}",
            h.log
        );
    }

    #[test]
    fn only_allocated_points_make_the_tree_worth_asking_about() {
        // class start and ascendancy nodes do not count, as PoB's CountAllocNodes
        for (used, ascendancy) in [(0, 0), (0, 4)] {
            let mut h = Harness::new();
            deliver_tree(&mut h, used, ascendancy);
            pick_character(&mut h, "Alice");
            h.click("import_character");
            assert!(h.confirms.is_empty(), "{used} {ascendancy}");
            assert_eq!(h.log.len(), 1);
        }
        let mut h = Harness::new();
        deliver_tree(&mut h, 1, 0);
        pick_character(&mut h, "Alice");
        h.click("import_character");
        assert_eq!(h.confirms.len(), 1);
        assert!(h.log.is_empty());
    }

    fn finished_tree(cancelled: bool, error: Option<&str>) -> Snapshot {
        Snapshot::Requests(Arc::new(RequestsView {
            pending: vec![],
            finished: vec![crate::engine::views::FinishedRequest {
                id: 1,
                label: "Importing passive tree".into(),
                error: error.map(str::to_owned),
                cancelled,
                new_build: false,
            }],
        }))
    }

    #[test]
    fn a_cancelled_or_failed_tree_part_never_sends_the_items() {
        for (cancelled, error) in [
            (true, Some("Cancelled")),
            (false, Some("Failed to retrieve character data")),
        ] {
            let mut h = Harness::new();
            pick_character(&mut h, "Alice");
            h.click("import_character");
            assert_eq!(h.log.len(), 1);
            let head = h.seq;
            h.ui.on_snapshot(&Snapshot::Account(Arc::new(account_with(true, None))), head);
            // the engine sends the finished request first, then the settled account
            h.ui.on_snapshot(&finished_tree(cancelled, error), head + 1);
            h.ui.on_snapshot(
                &Snapshot::Account(Arc::new(account_with(false, None))),
                head + 1,
            );
            h.frame(vec![]);
            assert_eq!(
                h.log.len(),
                1,
                "items went after a stopped tree: {:?}",
                h.log
            );
        }
        // a tree that finished well lets them go
        let mut h = Harness::new();
        pick_character(&mut h, "Alice");
        h.click("import_character");
        let head = h.seq;
        h.ui.on_snapshot(&finished_tree(false, None), head + 1);
        h.ui.on_snapshot(
            &Snapshot::Account(Arc::new(account_with(false, None))),
            head + 1,
        );
        h.frame(vec![]);
        assert_eq!(h.log.len(), 2);
    }

    #[test]
    fn items_only_needs_no_tree_question() {
        let mut h = Harness::new();
        pick_character(&mut h, "Alice");
        h.ui.part_tree = false;
        deliver_tree(&mut h, 3, 0);
        h.click("import_character");
        assert!(h.confirms.is_empty());
        assert_eq!(h.log.len(), 1);
        assert!(h.log[0].contains("ItemsAndSkills"), "{:?}", h.log);
    }

    #[test]
    fn league_filter_is_ui_side_and_sent_with_import() {
        let mut h = Harness::new();
        h.frame(vec![]);
        assert!(marked("char:Alice").is_some() && marked("char:Bob").is_some());
        h.click("league_combo");
        h.click("league:Standard");
        assert!(h.log.is_empty(), "filtering is the UI's: {:?}", h.log);
        h.frame(vec![]);
        assert!(marked("char:Alice").is_some() && marked("char:Cara").is_some());
        assert!(marked("char:Bob").is_none(), "Bob plays in Settlers");
        h.click("char:Cara");
        h.click("import_character");
        assert!(
            h.log[0].contains("league: Some(\"Standard\")"),
            "{:?}",
            h.log
        );
        assert!(h.log[0].contains("name: \"Cara\""));
        // back to Any: no league, Bob is back
        h.click("league_combo");
        h.click("league:Any");
        h.frame(vec![]);
        assert!(marked("char:Bob").is_some());
    }

    #[test]
    fn a_filtered_out_character_is_deselected() {
        let mut h = Harness::new();
        pick_character(&mut h, "Bob");
        h.click("league_combo");
        h.click("league:Standard");
        h.frame(vec![]);
        assert!(h.ui.selected.is_none());
    }

    #[test]
    fn realm_is_sent_with_fetch() {
        let mut h = Harness::new();
        let mut view = signed_in();
        view.oauth.fetched = false;
        h.account(view);
        h.frame(vec![]);
        h.click("realm_combo");
        h.click("realm:XBOX");
        h.click("fetch");
        assert_eq!(h.log, ["ListCharacters { realm: Xbox }"]);
    }

    #[test]
    fn fetch_is_disabled_while_a_sign_in_or_fetch_runs() {
        for view in [
            account_with(true, None),
            {
                let mut v = signed_in();
                v.oauth.fetched = false;
                v.oauth.busy = true;
                v
            },
            {
                let mut v = signed_in();
                v.oauth.fetched = false;
                v.oauth.signing_in = true;
                v
            },
        ] {
            let mut h = Harness::new();
            h.account(view);
            h.frame(vec![]);
            h.click("fetch");
            assert!(h.log.is_empty(), "{:?}", h.log);
        }
        // a pending request of ours blocks it even before the view says so
        let mut h = Harness::new();
        let mut view = signed_in();
        view.oauth.fetched = false;
        h.account(view);
        h.seq += 1;
        h.ui.on_snapshot(
            &Snapshot::Requests(Arc::new(RequestsView {
                pending: vec![PendingRequest {
                    id: 7,
                    label: "Fetching characters".into(),
                }],
                finished: vec![],
            })),
            h.seq,
        );
        h.frame(vec![]);
        h.click("fetch");
        assert!(h.log.is_empty(), "{:?}", h.log);
        // idle, it goes
        let mut h = Harness::new();
        let mut view = signed_in();
        view.oauth.fetched = false;
        h.account(view);
        h.frame(vec![]);
        h.click("fetch");
        assert_eq!(h.log, ["ListCharacters { realm: Pc }"]);
    }

    #[test]
    fn retry_countdown_from_retry_at() {
        let mut h = Harness::new();
        let mut view = signed_in();
        view.oauth.fetched = false;
        view.oauth.retry_at = Some(1_030);
        h.account(view);
        h.frame(vec![]);
        assert!(h.has_text("Retry in 30 s"), "{:?}", h.drawn);
        h.now = 1_010;
        h.frame(vec![]);
        assert!(h.has_text("Retry in 20 s"), "{:?}", h.drawn);
        // Fetch waits for it
        h.click("fetch");
        assert!(h.log.is_empty());
        h.now = 1_031;
        h.frame(vec![]);
        assert!(!h.has_text("Retry in"), "{:?}", h.drawn);
        h.click("fetch");
        assert_eq!(h.log, ["ListCharacters { realm: Pc }"]);
    }

    #[test]
    fn code_box_validates_after_300ms() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.click("code_box");
        h.frame(vec![egui::Event::Text("https://pobb.in/abc".into())]);
        assert!(h.log.is_empty(), "sent while typing: {:?}", h.log);
        h.idle_for(0.1);
        assert!(h.log.is_empty(), "{:?}", h.log);
        h.idle_for(0.3);
        assert_eq!(h.log, ["SetImportCode { text: \"https://pobb.in/abc\" }"]);
        h.idle_for(1.0);
        assert_eq!(h.log.len(), 1, "once");
        // PoB's verdict, in its POSITIVE or NEGATIVE colour
        assert_eq!(detail_colour(true), theme::POSITIVE);
        assert_eq!(detail_colour(false), theme::NEGATIVE);
        let mut view = share();
        view.import.text = "https://pobb.in/abc".into();
        view.import.valid = true;
        view.import.detail = "URL is valid (pobb.in)".into();
        h.share(view);
        h.frame(vec![]);
        assert!(h.has_text("URL is valid (pobb.in)"), "{:?}", h.drawn);
    }

    #[test]
    fn the_verdict_waits_for_the_typing_to_settle() {
        let mut h = Harness::new();
        let mut view = share();
        view.import.valid = true;
        view.import.detail = "URL is valid (pobb.in)".into();
        h.share(view);
        h.ui.type_for_test("x", h.time);
        h.frame(vec![]);
        assert!(
            !h.has_text("URL is valid"),
            "an older verdict for newer text"
        );
        h.click("import_code");
        assert!(
            h.confirms.is_empty(),
            "Import is off until the box is settled"
        );
    }

    #[test]
    fn this_build_hidden_without_a_file() {
        let mut h = Harness::new();
        h.frame(vec![]);
        assert!(marked("mode:this").is_some());
        let mut view = share();
        view.import.can_import_to_this_build = false;
        h.share(view);
        h.frame(vec![]);
        assert!(marked("mode:new").is_some());
        assert!(marked("mode:this").is_none());
        // a mode chosen earlier falls back
        let mut h = Harness::new();
        h.share(valid_share());
        h.frame(vec![]);
        h.click("mode:compare");
        assert_eq!(h.ui.mode, Some(ImportMode::Comparison));
        let mut view = valid_share();
        view.import.can_import_to_this_build = false;
        h.share(view);
        h.frame(vec![]);
        assert_eq!(h.ui.mode, Some(ImportMode::NewBuild));
    }

    /// PoB's drop-down starts on "Import to this build" for a build with a file, and waits for
    /// a valid text (ImportTab.lua:680-682)
    #[test]
    fn the_mode_starts_on_this_build_and_waits_for_a_valid_text() {
        let mut h = Harness::new();
        h.frame(vec![]);
        // not valid yet: the choice is off
        h.click("mode:new");
        assert_eq!(h.ui.mode, None);
        h.share(valid_share());
        h.frame(vec![]);
        h.click("import_code");
        let Some(Confirm::Ask { plan, .. }) = h.confirms.pop() else {
            panic!("PoB's default mode asks first: {:?}", h.log);
        };
        assert_eq!(
            format!("{:?}", plan.single().unwrap()),
            "ImportFromUrl { url: \"https://pobb.in/abc\", mode: ThisBuild }"
        );
        h.click("mode:new");
        assert_eq!(h.ui.mode, Some(ImportMode::NewBuild));
    }

    /// Enter in the box imports a valid text (`enterFunc`, ImportTab.lua:671-675); on text PoB
    /// has not judged yet it sends the text and acts on the verdict
    #[test]
    fn enter_in_the_code_box_imports() {
        let mut h = Harness::new();
        h.share(valid_share());
        h.frame(vec![]);
        h.ui.mode = Some(ImportMode::Comparison);
        h.click("code_box");
        let enter = || egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        h.frame(vec![enter()]);
        assert_eq!(
            h.log,
            ["ImportFromUrl { url: \"https://pobb.in/abc\", mode: Comparison }"]
        );
        assert!(!h.ui.code.contains('\n'), "Enter is no new line");
        // typed and not judged: the text goes now, the import on PoB's verdict
        h.log.clear();
        h.frame(vec![egui::Event::Text("x".into())]);
        h.frame(vec![enter()]);
        assert_eq!(h.log, ["SetImportCode { text: \"https://pobb.in/abcx\" }"]);
        h.frame(vec![]);
        assert_eq!(h.log.len(), 1, "nothing before the verdict");
        let mut view = valid_share();
        view.import.text = "https://pobb.in/abcx".into();
        h.share(view);
        h.frame(vec![]);
        assert_eq!(
            h.log[1..],
            ["ImportFromUrl { url: \"https://pobb.in/abcx\", mode: Comparison }"]
        );
    }

    #[test]
    fn import_as_comparison_mode_offered() {
        // PoB's third mode, only for a build with a file
        let mut h = Harness::new();
        h.share(valid_share());
        h.frame(vec![]);
        assert!(marked("mode:compare").is_some());
        h.click("mode:compare");
        assert_eq!(h.ui.mode, Some(ImportMode::Comparison));
        // the build stays: no question, the import goes at once
        h.click("import_code");
        assert!(h.confirms.is_empty());
        assert_eq!(
            h.log,
            ["ImportFromUrl { url: \"https://pobb.in/abc\", mode: Comparison }"]
        );
        // a build without a file: not offered, and a chosen mode falls back
        let mut view = valid_share();
        view.import.can_import_to_this_build = false;
        h.share(view);
        h.frame(vec![]);
        assert!(marked("mode:compare").is_none());
        assert_eq!(h.ui.mode, Some(ImportMode::NewBuild));
    }

    fn valid_share() -> ShareView {
        let mut view = share();
        view.import.text = "https://pobb.in/abc".into();
        view.import.valid = true;
        view.import.detail = "URL is valid (pobb.in)".into();
        view
    }

    #[test]
    fn this_build_asks_first() {
        let mut h = Harness::new();
        h.share(valid_share());
        h.frame(vec![]);
        h.click("mode:this");
        h.click("import_code");
        assert!(h.log.is_empty(), "nothing before the question: {:?}", h.log);
        let Some(Confirm::Ask { message, plan }) = h.confirms.pop() else {
            panic!("expected a question");
        };
        assert_eq!(
            message,
            "Importing to the current build will erase ALL existing data for this build."
        );
        assert_eq!(
            format!("{:?}", plan.single().unwrap()),
            "ImportFromUrl { url: \"https://pobb.in/abc\", mode: ThisBuild }"
        );
    }

    #[test]
    fn new_build_import_goes_to_the_unsaved_prompt() {
        let mut h = Harness::new();
        h.share(valid_share());
        h.frame(vec![]);
        h.click("mode:new");
        h.click("import_code");
        assert!(h.log.is_empty());
        assert!(matches!(
            h.confirms.as_slice(),
            [Confirm::NewBuild { url }] if url == "https://pobb.in/abc"
        ));
    }

    #[test]
    fn share_sends_site_id_and_copy_uses_the_ui_clipboard() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.click("site_combo");
        h.click("site:Pastebin");
        h.click("share");
        assert_eq!(h.log, ["ShareBuild { site: \"Pastebin\" }"]);
        // the link comes back as text; Copy puts it on the clipboard through the UI
        let mut view = share();
        view.link = Some("https://pastebin.com/xyz".into());
        h.share(view);
        h.frame(vec![]);
        assert!(h.copied.is_empty(), "no copy unless asked");
        h.click("copy_link_result");
        assert_eq!(h.copied, ["https://pastebin.com/xyz"]);
        assert_eq!(h.log.len(), 1, "no command carries a copy");
    }

    #[test]
    fn share_is_off_while_sharing_and_shows_a_spinner() {
        let mut h = Harness::new();
        let mut view = share();
        view.sharing = true;
        h.share(view);
        h.frame(vec![]);
        h.click("share");
        assert!(h.log.is_empty());
    }

    #[test]
    fn the_site_defaults_to_the_last_one_used() {
        let mut h = Harness::new();
        let mut view = share();
        view.last_site = Some("Pastebin".into());
        h.share(view);
        h.frame(vec![]);
        h.click("share");
        assert_eq!(h.log, ["ShareBuild { site: \"Pastebin\" }"]);
    }

    fn ctrl_c() -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key: egui::Key::C,
            physical_key: Some(egui::Key::C),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]
    }

    #[test]
    fn ctrl_c_on_the_tab_exports_the_code() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.frame(ctrl_c());
        assert_eq!(h.log, ["ExportShareCode"]);
        assert!(h.copied.is_empty(), "nothing to copy yet");
        // the answer is copied once, through the UI
        h.seq += 1;
        h.ui.on_snapshot(&Snapshot::ShareCode(Arc::new("eNrt".into())), h.seq);
        h.frame(vec![]);
        h.frame(vec![]);
        assert_eq!(h.copied, ["eNrt"]);
        // a code nobody asked for is not copied
        h.seq += 1;
        h.ui.on_snapshot(&Snapshot::ShareCode(Arc::new("other".into())), h.seq);
        h.frame(vec![]);
        assert_eq!(h.copied, ["eNrt"]);
    }

    #[test]
    fn ctrl_c_in_the_code_box_copies_text_not_the_build() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.click("code_box");
        h.frame(ctrl_c());
        assert!(h.log.iter().all(|c| c != "ExportShareCode"), "{:?}", h.log);
    }

    #[test]
    fn copy_code_button_exports_too() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.click("copy_code");
        assert_eq!(h.log, ["ExportShareCode"]);
    }

    #[test]
    fn ctrl_c_waits_under_a_modal() {
        let mut h = Harness::new();
        h.ui.set_modal(true);
        h.frame(vec![]);
        h.frame(ctrl_c());
        assert!(h.log.is_empty());
    }

    #[test]
    fn leaving_the_tab_flushes_the_code_and_drops_a_wanted_copy() {
        let mut h = Harness::new();
        h.ui.type_for_test("abc", h.time);
        let flushed = h.ui.leave();
        assert_eq!(
            format!("{flushed:?}"),
            "Some(SetImportCode { text: \"abc\" })"
        );
        assert!(h.ui.leave().is_none());
        h.ui.sent(1, &Command::ExportShareCode);
        h.ui.leave();
        h.ui.on_snapshot(&Snapshot::ShareCode(Arc::new("late".into())), 2);
        h.frame(vec![]);
        assert!(h.copied.is_empty());
    }

    #[test]
    fn account_name_mode_lists_and_imports_public_characters() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.click("mode:name");
        h.click("account_name");
        h.frame(vec![egui::Event::Text("Someone#1234".into())]);
        h.click("start");
        assert_eq!(
            h.log,
            ["ListPublicCharacters { account: \"Someone#1234\", realm: Pc }"]
        );
        let mut view = signed_in();
        view.public = PublicView {
            account: "Someone#1234".into(),
            stage: PublicStage::SelectCharacter,
            leagues: vec!["Standard".into(), "All".into()],
            characters: vec![ch("Dora", "Standard")],
            ..PublicView::default()
        };
        h.account(view);
        h.frame(vec![]);
        h.click("char:Dora");
        h.ui.part_items = false;
        h.click("import_character");
        assert_eq!(
            h.log[1],
            "ImportPublicCharacter { league: None, name: \"Dora\", part: Tree { delete_jewels: true } }"
        );
        h.click("close_public");
        assert_eq!(h.log[2], "ClosePublicCharacters");
    }

    #[test]
    fn leaving_the_account_name_box_starts_no_request() {
        // typing a name, then blurring by every route, sends nothing; Enter and Start do
        for away in ["realm_combo", "mode:oauth", "code_box"] {
            let mut h = Harness::new();
            h.frame(vec![]);
            h.click("mode:name");
            h.click("account_name");
            h.frame(vec![egui::Event::Text("Someone#1234".into())]);
            h.click(away);
            h.frame(vec![]);
            assert!(h.log.is_empty(), "blurring via {away} sent {:?}", h.log);
            // and the box keeps the name, as PoB's keeps its text
            assert_eq!(h.ui.account_box.text, "Someone#1234", "via {away}");
        }
        let mut h = Harness::new();
        h.frame(vec![]);
        h.click("mode:name");
        h.click("account_name");
        h.frame(vec![egui::Event::Text("Someone#1234".into())]);
        h.frame(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: Some(egui::Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        h.frame(vec![]);
        // PoB's box has no Enter function (ImportTab.lua:340-341): only Start asks
        assert!(h.log.is_empty(), "Enter sent {:?}", h.log);
        h.click("start");
        assert_eq!(
            h.log,
            ["ListPublicCharacters { account: \"Someone#1234\", realm: Pc }"]
        );
    }

    /// The name and code boxes are clearable: an "x" empties them (EditControl.lua:73-77)
    #[test]
    fn the_name_and_code_boxes_clear_on_their_x() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.click("mode:name");
        h.click("account_name");
        h.frame(vec![egui::Event::Text("Someone#1234".into())]);
        h.frame(vec![]);
        h.click("account_clear");
        h.frame(vec![]);
        assert_eq!(h.ui.account_box.text, "");
        h.click("code_box");
        h.frame(vec![egui::Event::Text("abc".into())]);
        h.frame(vec![]);
        h.click("code_clear");
        assert_eq!(h.ui.code, "");
        h.idle_for(1.0);
        assert_eq!(
            h.log.last().map(String::as_str),
            Some("SetImportCode { text: \"\" }")
        );
    }

    #[test]
    fn leaving_the_code_box_imports_nothing() {
        // the box only validates; it never fetches or imports by itself
        let mut h = Harness::new();
        h.frame(vec![]);
        h.click("code_box");
        h.frame(vec![egui::Event::Text("https://pobb.in/abc".into())]);
        h.click("realm_combo");
        h.idle_for(1.0);
        assert!(
            h.log.iter().all(|c| c.starts_with("SetImportCode")),
            "{:?}",
            h.log
        );
    }

    #[test]
    fn sign_out_and_cancel_by_request_id() {
        let mut h = Harness::new();
        h.frame(vec![]);
        h.click("sign_out");
        assert_eq!(h.log, ["SignOut"]);
        let mut h = Harness::new();
        h.account(signed_out());
        h.frame(vec![]);
        h.click("sign_in");
        h.account(waiting(Some(URL)));
        h.seq += 1;
        h.ui.on_snapshot(
            &Snapshot::Requests(Arc::new(RequestsView {
                pending: vec![
                    PendingRequest {
                        id: 3,
                        label: "Sharing on pobb.in".into(),
                    },
                    PendingRequest {
                        id: 9,
                        label: "Signing in".into(),
                    },
                ],
                finished: vec![],
            })),
            h.seq,
        );
        h.frame(vec![]);
        h.click("signin_cancel");
        assert_eq!(h.log.last().unwrap(), "CancelRequest { id: 9 }");
    }

    #[test]
    fn cancelling_an_oauth_fetch_or_import_may_sign_you_out() {
        let mut h = Harness::new();
        for label in [
            "Fetching characters",
            "Importing passive tree",
            "Importing items and skills",
        ] {
            assert!(h.ui.cancel_signs_out(label), "{label}");
        }
        for label in [
            "Signing in",
            "Sharing on pobb.in",
            "Fetching build from pobb.in",
        ] {
            assert!(!h.ui.cancel_signs_out(label), "{label}");
        }
        // signed out: no token is at stake
        h.account(signed_out());
        assert!(!h.ui.cancel_signs_out("Fetching characters"));
        // signed in, with an account-name request running too: the OAuth one still asks
        let mut view = signed_in();
        view.public.stage = PublicStage::Importing;
        h.account(view);
        assert!(h.ui.cancel_signs_out("Importing passive tree"));
    }

    // ---- Task 13: PoB's account history and its labels (NF-8, controller rulings) ----

    #[test]
    fn missing_discriminator_shows_pobs_label_and_disables_start() {
        let mut h = Harness::new();
        let mut view = signed_in();
        view.public.missing_discriminator = "Missing discriminator e.g. #1234".into();
        h.account(view);
        h.frame(vec![]);
        h.click("mode:name");
        h.click("account_name");
        h.frame(vec![egui::Event::Text("Someone".into())]);
        h.frame(vec![]);
        assert!(h.has_text("Missing discriminator e.g. #1234"));
        h.click("start");
        assert!(h.log.is_empty(), "{:?}", h.log);
        // the box keeps what was typed, as PoB's: the discriminator goes after it
        h.click("account_name");
        h.frame(vec![egui::Event::Key {
            key: egui::Key::End,
            physical_key: Some(egui::Key::End),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        h.frame(vec![egui::Event::Text("#1234".into())]);
        h.frame(vec![]);
        assert!(!h.has_text("Missing discriminator"));
        h.click("start");
        assert_eq!(
            h.log,
            ["ListPublicCharacters { account: \"Someone#1234\", realm: Pc }"]
        );
    }

    #[test]
    fn history_pick_fills_the_box_and_x_removes_it() {
        let mut h = Harness::new();
        let mut view = signed_in();
        view.public.history = vec!["alice#1234".into(), "Bob#0001".into()];
        h.account(view);
        h.frame(vec![]);
        h.click("mode:name");
        h.click("history");
        h.frame(vec![]);
        h.click("history:Bob#0001");
        h.click("start");
        assert_eq!(
            h.log,
            ["ListPublicCharacters { account: \"Bob#0001\", realm: Pc }"]
        );
        h.click("history_remove");
        assert_eq!(
            h.log[1..],
            ["RemoveAccountHistory { account: \"Bob#0001\" }"]
        );
    }

    /// 13a's account history steps with the wheel, as a pick into the account box
    #[test]
    fn the_wheel_steps_the_account_history() {
        let mut h = Harness::new();
        let mut view = signed_in();
        view.public.history = vec!["alice#1234".into(), "Bob#0001".into()];
        h.account(view);
        h.frame(vec![]);
        h.click("mode:name");
        let history = marked("history").expect("no history").center();
        h.frame(vec![egui::Event::PointerMoved(history)]);
        h.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::Vec2::new(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        }]);
        h.frame(vec![]);
        h.click("start");
        assert_eq!(
            h.log,
            ["ListPublicCharacters { account: \"Bob#0001\", realm: Pc }"]
        );
    }

    /// The realm is one of PoB's searchable dropdowns (SearchHost)
    #[test]
    fn the_realm_dropdown_filters_as_typed() {
        let mut h = Harness::new();
        h.account(signed_in());
        h.frame(vec![]);
        h.click("realm_combo");
        h.frame(vec![]);
        h.frame(vec![egui::Event::Text("xb".into())]);
        h.frame(vec![]);
        assert_eq!(
            marked("realm:PC").unwrap().height(),
            0.0,
            "PC wasn't filtered out"
        );
        h.click("realm:XBOX");
        assert_eq!(h.ui.realm, Realm::Xbox);
    }

    #[test]
    fn the_character_list_shows_pobs_bandit_tip() {
        let mut h = Harness::new();
        let mut view = signed_in();
        view.public = PublicView {
            account: "Someone#1234".into(),
            stage: PublicStage::SelectCharacter,
            leagues: vec!["Standard".into(), "All".into()],
            characters: vec![ch("Dora", "Standard")],
            bandit_note: "Tip: After you finish importing a character, make sure you update the bandit choice,".into(),
            ..PublicView::default()
        };
        h.account(view);
        h.frame(vec![]);
        h.click("mode:name");
        h.frame(vec![]);
        assert!(h.has_text("update the bandit choice"));
    }

    #[test]
    fn account_name_state_follows_pobs_patterns() {
        // (label shown, Start enabled) as ImportTab.lua:367-404 decide them
        for (text, want) in [
            ("", (false, false)),
            ("Name", (true, false)),
            ("Name#123", (true, false)),
            ("Name#1234", (false, true)),
            ("Name-1234", (false, true)),
            ("#1234", (false, false)),
            (" #1234", (false, false)),
            ("Name#1234 ", (true, false)),
            ("Näme#1234", (false, true)),
            ("Name#12345", (true, false)),
        ] {
            assert_eq!(account_name_state(text), want, "{text:?}");
        }
    }

    #[test]
    fn x_removes_pobs_selected_account_without_a_pick() {
        let mut h = Harness::new();
        let mut view = signed_in();
        view.public.history = vec!["alice#1234".into(), "Bob#0001".into()];
        view.public.history_selected = Some("alice#1234".into());
        h.account(view);
        h.frame(vec![]);
        h.click("mode:name");
        h.click("history_remove");
        assert_eq!(h.log, ["RemoveAccountHistory { account: \"alice#1234\" }"]);
    }

    /// PoB's drop-downs start on its own picks: the remembered realm, the last league (else
    /// the first) and the last character (else the first) (ImportTab.lua:121-131, 1085-1093)
    #[test]
    fn the_lists_start_on_pobs_picks() {
        let mut h = Harness::new();
        let mut view = signed_in();
        view.oauth.realm = Realm::Xbox;
        view.oauth.league_selected = Some("Standard".into());
        view.oauth.character_selected = Some("Cara".into());
        h.account(view.clone());
        h.frame(vec![]);
        assert_eq!(h.ui.realm, Realm::Xbox);
        assert!(h.has_text("Xbox"));
        assert_eq!(h.ui.league.as_deref(), Some("Standard"));
        assert_eq!(h.ui.selected.as_deref(), Some("Cara"));
        assert!(marked("char:Bob").is_none(), "Bob plays in Settlers");
        // one click imports PoB's pick, as PoB's buttons do
        h.click("import_character");
        assert!(h.log[0].contains("name: \"Cara\""), "{:?}", h.log);
        // the user's own picks stay while the list is the same
        h.click("league_combo");
        h.click("league:Any");
        h.click("char:Bob");
        h.account(view);
        h.frame(vec![]);
        assert_eq!(h.ui.selected.as_deref(), Some("Bob"));
        // PoB's realm label for Sony
        assert_eq!(realm_label(Realm::Sony), "Sony");
    }

    /// The account name list starts on PoB's picks too
    #[test]
    fn the_public_list_starts_on_pobs_picks() {
        let mut h = Harness::new();
        let mut view = signed_out();
        view.public.stage = PublicStage::SelectCharacter;
        view.public.leagues = vec!["Settlers".into(), "Standard".into(), "All".into()];
        view.public.characters = vec![ch("Alice", "Standard"), ch("Bob", "Settlers")];
        view.public.league_selected = Some("Settlers".into());
        view.public.character_selected = Some("Bob".into());
        h.account(view);
        h.frame(vec![]);
        h.click("mode:name");
        h.frame(vec![]);
        assert_eq!(h.ui.league.as_deref(), Some("Settlers"));
        assert_eq!(h.ui.selected.as_deref(), Some("Bob"));
        assert!(marked("char:Alice").is_none());
    }

    /// PoB's entry: name, the class in its colour, "lvl N" in grey, and the league only with
    /// no league picked (ImportTab.lua:1068-1073)
    #[test]
    fn a_character_entry_reads_as_pobs() {
        let mut h = Harness::new();
        h.frame(vec![]);
        assert!(
            h.has_text("Alice   Witch lvl 90 in Standard"),
            "{:?}",
            h.drawn
        );
        h.click("league_combo");
        h.click("league:Standard");
        h.frame(vec![]);
        assert!(h.has_text("Alice   Witch lvl 90"));
        assert!(!h.has_text("Alice   Witch lvl 90 in"));
    }

    /// PoB's option labels and tips (ImportTab.lua:297-333)
    #[test]
    fn the_import_options_carry_pobs_tips() {
        let mut h = Harness::new();
        h.frame(vec![]);
        for (key, tip) in [
            ("opt:jewels", TIP_JEWELS),
            ("opt:skills", TIP_SKILLS),
            ("opt:items", TIP_ITEMS),
            ("opt:swap", TIP_SWAP),
        ] {
            let pos = marked(key).unwrap().center();
            h.frame(vec![egui::Event::PointerMoved(pos)]);
            for _ in 0..40 {
                h.time += 0.1;
                h.frame(vec![]);
            }
            assert!(h.has_text(tip), "{key}: {:?}", h.drawn);
        }
        for label in [
            "Passive Tree and Jewels",
            "Delete jewels",
            "Items and Skills",
            "Delete skills",
            "Delete equipment",
            "Ignore weapon swap",
        ] {
            assert!(h.has_text(label), "{label}");
        }
    }

    /// PoB's labels: "Character import status: " and "Error: " (ImportTab.lua:78-81, 172-176)
    #[test]
    fn the_status_and_error_read_as_pobs() {
        let mut h = Harness::new();
        let mut view = signed_in();
        view.oauth.status = "Authenticated".into();
        view.oauth.error = Some("Could not import character".into());
        h.account(view);
        h.frame(vec![]);
        assert!(h.has_text("Character import status: Authenticated"));
        // a status PoB colours keeps its colour
        let mut view = signed_out();
        view.oauth.status = "Not authenticated".into();
        view.oauth.status_coded = "^xFF9922Not authenticated".into();
        h.account(view);
        h.frame(vec![]);
        assert_eq!(
            h.colour_of("Not authenticated"),
            Some(egui::Color32::from_rgb(0xFF, 0x99, 0x22))
        );
        let mut view = signed_in();
        view.oauth.status = "Authenticated".into();
        view.oauth.error = Some("Could not import character".into());
        h.account(view);
        h.frame(vec![]);
        assert!(h.has_text("Character import status: Authenticated"));
        assert!(h.has_text("Choose character to import data from:"));
        assert!(h.has_text("Error: Could not import character"));
    }

    /// PoB's tip on Start for a name without its discriminator (ImportTab.lua:370-375)
    #[test]
    fn start_tells_what_the_name_lacks() {
        let mut h = Harness::new();
        h.account(signed_out());
        h.frame(vec![]);
        h.click("mode:name");
        h.click("account_name");
        h.frame(vec![egui::Event::Text("Someone".into())]);
        h.idle_for(1.0);
        assert!(h.has_text("To start importing a character, enter the character's account name:"));
        let pos = marked("start").unwrap().center();
        h.frame(vec![egui::Event::PointerMoved(pos)]);
        for _ in 0..40 {
            h.time += 0.1;
            h.frame(vec![]);
        }
        assert!(
            h.has_text("Missing discriminator e.g. Someone#1234"),
            "{:?}",
            h.drawn
        );
    }

    /// PoB's Import button reads "Retrieving paste.." while a paste is fetched
    #[test]
    fn import_says_it_is_retrieving_the_paste() {
        let mut h = Harness::new();
        let mut view = valid_share();
        view.import.fetching = true;
        h.share(view);
        h.frame(vec![]);
        assert!(h.has_text("Retrieving paste.."));
    }

    /// Opening the tab fetches the remembered realm's characters once, signed in, as PoB's
    /// `TryFetchCharacterList` (ImportTab.lua:734-739); not on Ctrl+I, and only once
    #[test]
    fn opening_the_tab_fetches_the_remembered_realm_once() {
        let fetchable = |realm| {
            let mut view = signed_in();
            view.oauth.fetched = false;
            view.oauth.realm = realm;
            view.oauth.remembered_realm = true;
            view
        };
        let mut h = Harness::new();
        h.account(fetchable(Realm::Sony));
        h.frame(vec![]);
        assert_eq!(h.log, ["ListCharacters { realm: Sony }"]);
        h.frame(vec![]);
        assert_eq!(h.log.len(), 1, "once");
        // Ctrl+I opens the tab without it
        let mut h = Harness::new();
        h.account(fetchable(Realm::Pc));
        h.ui.focus_code();
        h.frame(vec![]);
        assert!(h.log.is_empty(), "{:?}", h.log);
        // no remembered realm: nothing
        let mut h = Harness::new();
        let mut view = fetchable(Realm::Pc);
        view.oauth.remembered_realm = false;
        h.account(view);
        h.frame(vec![]);
        assert!(h.log.is_empty());
        // signed out: the try is spent, nothing goes
        let mut h = Harness::new();
        let mut view = fetchable(Realm::Pc);
        view.oauth.signed_in = false;
        h.account(view);
        h.frame(vec![]);
        h.account(fetchable(Realm::Pc));
        h.frame(vec![]);
        assert!(h.log.is_empty(), "{:?}", h.log);
    }
}
