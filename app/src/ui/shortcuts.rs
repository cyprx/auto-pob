//! PoB's keyboard and mouse shortcuts (R25): which key does what where. Reading them is pure;
//! the app carries them out (`AutoPobApp::run_shortcut`).
//!
//! - Build (Build.lua:1150-1186): Ctrl+1..6 tabs, Ctrl+I Import/Export, Ctrl+W and Mouse4
//!   close the build (asking about unsaved changes).
//! - Tree tab (TreeTab.lua:359-384, PassiveTreeView.lua:198-218): Up/Down previous and next
//!   tree, Ctrl+F search, Ctrl+M the tree manager, P heat map, Ctrl+D stat differences,
//!   Ctrl+C the hovered node's text, PageUp/PageDown zoom (Shift: three steps).
//! - Items tab (ItemsTab.lua:1454-1490): E edits the hovered slot's item, Ctrl+F the item
//!   database search. Config and Compare: Ctrl+F their search.
//! - Everywhere (Main.lua:436-446, ItemTools.lua:380-410): F1 opens the hovered thing's wiki
//!   page, or else the Help window at the section for where the user is.
//!
//! None fires under a modal window (the Help window included), except F1 over a row of the
//! gem list's window. While a text field has the keyboard they do what PoB does with a
//! selected control (ControlHost.lua:35-80): Build's keys and the tabs' own loops run first
//! and still fire (Ctrl+1..6, Ctrl+I, Ctrl+W, Mouse4; Up/Down, Ctrl+F, Ctrl+M, E), and the
//! box gets the key too; the tree view's keys, the set lists' F2 and F1's Help come after
//! the controls and don't; a hovered list row's, slot's or gem box's wiki page still opens
//! (OnHoverKeyUp). A box of the top bar or the sidebar is Build's own control, which takes
//! every key before the tab (Build.lua:1230).

use crate::ui::build::BuildTab;
use eframe::egui::{self, Event, Key, PointerButton};

/// What the shortcuts depend on, read before the frame's widgets.
#[derive(Debug, Clone, Default)]
pub struct ShortcutState {
    /// The open build's tab; `None` on the build list
    pub tab: Option<BuildTab>,
    /// A text field has the keyboard
    pub typing: bool,
    /// A window or question holds the app
    pub modal: bool,
    /// The pointer is over the tree selector or its list is open: Up/Down leave the trees
    /// alone (TreeTab.lua:367-371)
    pub spec_select_busy: bool,
    /// The wiki page F1 opens for what the pointer is over (`itemLib.wiki.open`)
    pub wiki: Option<String>,
    /// The text field with the keyboard is the top bar's or the sidebar's
    pub build_box: bool,
    /// A node Ctrl+C can copy is under the pointer: no socket (PassiveTreeView.lua:208)
    pub copy_node: bool,
    /// `wiki` is a list row's, a slot's or a gem box's, read on hover even while typing
    /// (OnHoverKeyUp); a tree node's isn't
    pub hover_wiki: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutAction {
    /// Ctrl+1..6
    Tab(BuildTab),
    /// Ctrl+I: the Import/Export tab with its code box selected
    Import,
    /// Ctrl+W or Mouse4: back to the list, through the unsaved prompt
    Close,
    /// F1 with nothing to open on the wiki
    Help,
    /// F1 over something with a wiki page: its URL
    Wiki(String),
    /// Ctrl+F: the tab's search box
    Search,
    /// Ctrl+M (Tree)
    SpecManager,
    /// Up (Tree)
    PrevSpec,
    /// Down (Tree)
    NextSpec,
    /// P (Tree)
    Heatmap,
    /// Ctrl+D (Tree; the Items tab has its own)
    StatDifferences,
    /// Ctrl+C over a node (Tree)
    CopyNode,
    /// PageUp/PageDown (Tree): PoB's zoom steps, 1.2 each
    Zoom(i32),
    /// E (Items)
    EditSlot,
}

/// PoB's wiki page for `name` (ItemTools.lua:401-405)
pub fn wiki_url(name: &str) -> String {
    format!("https://www.poewiki.net/wiki/{}", name.replace(' ', "_"))
}

/// A press of `key`, with Ctrl when `command`. PoB reads a plain key whatever modifiers are
/// held (TreeTab.lua:367, PassiveTreeView.lua:203, ItemsTab.lua:1459, ListControl.lua:375):
/// Ctrl+P is P to it.
fn pressed(input: &egui::InputState, key: Key, command: bool) -> bool {
    input.events.iter().any(|e| {
        matches!(e, Event::Key { key: k, pressed: true, modifiers, .. }
            if *k == key && (!command || (modifiers.command && !modifiers.alt)))
    })
}

/// A press of `key`, whatever else is held: whether Shift was, PoB's three zoom steps
/// (PassiveTreeView.lua:214-217)
fn pressed_shift(input: &egui::InputState, key: Key) -> Option<bool> {
    input.events.iter().find_map(|e| match e {
        Event::Key {
            key: k,
            pressed: true,
            modifiers,
            ..
        } if *k == key => Some(modifiers.shift),
        _ => None,
    })
}

fn released(input: &egui::InputState, key: Key) -> bool {
    input
        .events
        .iter()
        .any(|e| matches!(e, Event::Key { key: k, pressed: false, .. } if *k == key))
}

/// egui-winit turns Ctrl+C into `Event::Copy`; tests and other platforms send the key
fn copy(input: &egui::InputState) -> bool {
    input.events.iter().any(|e| matches!(e, Event::Copy)) || pressed(input, Key::C, true)
}

/// PoB's list keys (ListControl.lua:375-382): Up and Down step the selection round the list,
/// Home and End go to its ends. The new index, the key taken out of `input`.
pub fn list_key(
    input: &mut egui::InputState,
    len: usize,
    selected: Option<usize>,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let (key, to) = if pressed(input, Key::ArrowUp, false) {
        (
            Key::ArrowUp,
            selected.map_or(len - 1, |i| (i + len - 1) % len),
        )
    } else if pressed(input, Key::ArrowDown, false) {
        (Key::ArrowDown, selected.map_or(0, |i| (i + 1) % len))
    } else if pressed(input, Key::Home, false) {
        (Key::Home, 0)
    } else if pressed(input, Key::End, false) {
        (Key::End, len - 1)
    } else {
        return None;
    };
    input
        .events
        .retain(|e| !matches!(e, Event::Key { key: k, .. } if *k == key));
    Some(to.min(len - 1))
}

/// Keeps the arrow keys on the focused list row `id`, for [`list_key`]: egui would move the
/// focus with them otherwise
pub fn keep_arrows(ctx: &egui::Context, id: egui::Id) {
    ctx.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            egui::EventFilter {
                vertical_arrows: true,
                ..Default::default()
            },
        );
    });
}

/// PoB's list deletes on Delete and Backspace (ListControl.lua:392)
pub fn list_delete(input: &egui::InputState) -> bool {
    pressed(input, Key::Delete, false) || pressed(input, Key::Backspace, false)
}

/// The shortcut this frame's input asks for, if any.
pub fn shortcut_action(state: &ShortcutState, input: &egui::InputState) -> Option<ShortcutAction> {
    use ShortcutAction as A;
    let f1 = released(input, Key::F1);
    if state.modal {
        // the gem list's window still opens a gem's page (GemSelectControl.lua:822-833); the
        // app passes `wiki` under a modal only for it
        return state
            .wiki
            .as_ref()
            .filter(|_| f1)
            .map(|n| A::Wiki(wiki_url(n)));
    }
    let tab = state.tab;
    // Build.lua:1150-1186 reads its keys before any control: they fire whatever has the
    // keyboard
    if tab.is_some() && input.pointer.button_pressed(PointerButton::Extra1) {
        return Some(A::Close);
    }
    if tab.is_some() {
        let digits = [
            (Key::Num1, BuildTab::Tree),
            (Key::Num2, BuildTab::Skills),
            (Key::Num3, BuildTab::Items),
            (Key::Num4, BuildTab::Calcs),
            (Key::Num5, BuildTab::Config),
            (Key::Num6, BuildTab::Notes),
        ];
        for (key, to) in digits {
            if pressed(input, key, true) {
                return Some(A::Tab(to));
            }
        }
        if pressed(input, Key::I, true) {
            return Some(A::Import);
        }
        if pressed(input, Key::W, true) {
            return Some(A::Close);
        }
    }
    // A box of the top bar or the sidebar is Build's own selected control: Build's
    // ProcessControlsInput takes every key before the tab draws (Build.lua:1230)
    if state.typing && state.build_box {
        return None;
    }
    if f1 {
        // a selected control takes the key-up from Main's F1 (ControlHost.lua:56-63); the
        // hovered list, slot and gem boxes still see it (OnHoverKeyUp, ControlHost.lua:71-79)
        return match &state.wiki {
            Some(name) if !state.typing || state.hover_wiki => Some(A::Wiki(wiki_url(name))),
            _ if state.typing => None,
            _ => Some(A::Help),
        };
    }
    let tab = tab?;
    // the tabs read these before their controls (TreeTab.lua:359-386, ItemsTab.lua:1454-1487,
    // ConfigTab.lua:978-993, CompareTab.lua:1685-1739): they fire while typing too. The
    // Calcs tab has its own Ctrl+F (calcs/mod.rs)
    if pressed(input, Key::F, true)
        && matches!(
            tab,
            BuildTab::Tree | BuildTab::Items | BuildTab::Config | BuildTab::Compare
        )
    {
        return Some(A::Search);
    }
    match tab {
        BuildTab::Tree if pressed(input, Key::M, true) => return Some(A::SpecManager),
        BuildTab::Tree if !state.spec_select_busy && pressed(input, Key::ArrowUp, false) => {
            return Some(A::PrevSpec);
        }
        BuildTab::Tree if !state.spec_select_busy && pressed(input, Key::ArrowDown, false) => {
            return Some(A::NextSpec);
        }
        BuildTab::Items if pressed(input, Key::E, false) => return Some(A::EditSlot),
        _ => {}
    }
    // the tree view reads its keys after the controls: a selected control takes them
    // (PassiveTreeView.lua:198-218, ControlHost.lua:39-42). The set lists' keys are their
    // windows' own.
    if state.typing {
        return None;
    }
    match tab {
        BuildTab::Tree => {
            let zoom =
                |key, step: i32| pressed_shift(input, key).map(|s| if s { 3 * step } else { step });
            if pressed(input, Key::D, true) {
                Some(A::StatDifferences)
            } else if state.copy_node && copy(input) {
                Some(A::CopyNode)
            } else if pressed(input, Key::P, false) {
                Some(A::Heatmap)
            } else {
                zoom(Key::PageUp, 1)
                    .or_else(|| zoom(Key::PageDown, -1))
                    .map(A::Zoom)
            }
        }
        _ => None,
    }
}

/// Takes the events of `action` out of `input`, so no widget acts on them too.
pub fn consume(action: &ShortcutAction, input: &mut egui::InputState) {
    use ShortcutAction as A;
    let key = match action {
        A::Tab(tab) => match tab {
            BuildTab::Tree => Key::Num1,
            BuildTab::Skills => Key::Num2,
            BuildTab::Items => Key::Num3,
            BuildTab::Calcs => Key::Num4,
            BuildTab::Config => Key::Num5,
            _ => Key::Num6,
        },
        A::Import => Key::I,
        A::Close => Key::W,
        A::Help | A::Wiki(_) => Key::F1,
        A::Search => Key::F,
        A::SpecManager => Key::M,
        A::PrevSpec => Key::ArrowUp,
        A::NextSpec => Key::ArrowDown,
        A::Heatmap => Key::P,
        A::StatDifferences => Key::D,
        A::CopyNode => Key::C,
        A::Zoom(n) if *n > 0 => Key::PageUp,
        A::Zoom(_) => Key::PageDown,
        A::EditSlot => Key::E,
    };
    input.events.retain(|e| match e {
        Event::Key { key: k, .. } => *k != key,
        Event::Copy => *action != A::CopyNode,
        // P and E would also type
        Event::Text(t) => {
            !(matches!(action, A::Heatmap | A::EditSlot) && t.eq_ignore_ascii_case(key.name()))
        }
        _ => true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Modifiers;

    fn input(events: Vec<Event>) -> egui::InputState {
        let ctx = egui::Context::default();
        let mut out = None;
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| out = Some(ui.input(|i| i.clone())),
        )
        .drop_without_applying_deltas();
        out.unwrap()
    }

    fn key(key: Key, modifiers: Modifiers) -> Vec<Event> {
        vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]
    }

    fn on(tab: BuildTab) -> ShortcutState {
        ShortcutState {
            tab: Some(tab),
            ..ShortcutState::default()
        }
    }

    fn action(state: &ShortcutState, events: Vec<Event>) -> Option<ShortcutAction> {
        shortcut_action(state, &input(events))
    }

    #[test]
    fn keys_map_to_pobs_actions() {
        use ShortcutAction as A;
        let tree = on(BuildTab::Tree);
        let cmd = Modifiers::COMMAND;
        assert_eq!(
            action(&tree, key(Key::Num5, cmd)),
            Some(A::Tab(BuildTab::Config))
        );
        assert_eq!(
            action(&tree, key(Key::Num6, cmd)),
            Some(A::Tab(BuildTab::Notes))
        );
        // Ctrl+7 is the Party tab, which is out (R25)
        assert_eq!(action(&tree, key(Key::Num7, cmd)), None);
        // a digit without Ctrl does nothing
        assert_eq!(action(&tree, key(Key::Num1, Modifiers::NONE)), None);
        assert_eq!(
            action(&tree, key(Key::PageUp, Modifiers::SHIFT)),
            Some(A::Zoom(3))
        );
        assert_eq!(
            action(&tree, key(Key::PageDown, Modifiers::NONE)),
            Some(A::Zoom(-1))
        );
        // Ctrl+C copies a node only with one under the pointer; else the copy goes on
        assert_eq!(action(&tree, vec![Event::Copy]), None);
        let over_node = ShortcutState {
            copy_node: true,
            ..on(BuildTab::Tree)
        };
        assert_eq!(action(&over_node, vec![Event::Copy]), Some(A::CopyNode));
        // the tree keys belong to the Tree tab
        let items = on(BuildTab::Items);
        assert_eq!(action(&items, key(Key::P, Modifiers::NONE)), None);
        assert_eq!(
            action(&items, key(Key::E, Modifiers::NONE)),
            Some(A::EditSlot)
        );
        assert_eq!(action(&items, key(Key::F, cmd)), Some(A::Search));
        // Calcs keeps its own Ctrl+F; the Items tab its own Ctrl+D
        assert_eq!(action(&on(BuildTab::Calcs), key(Key::F, cmd)), None);
        assert_eq!(action(&items, key(Key::D, cmd)), None);
        // Up and Down leave the trees alone while the selector is pointed at or open
        let busy = ShortcutState {
            spec_select_busy: true,
            ..on(BuildTab::Tree)
        };
        assert_eq!(action(&busy, key(Key::ArrowUp, Modifiers::NONE)), None);
        assert_eq!(
            action(&tree, key(Key::ArrowUp, Modifiers::NONE)),
            Some(A::PrevSpec)
        );
        // PoB checks no modifier on its plain keys: Ctrl or Shift held, they still act
        assert_eq!(action(&tree, key(Key::P, cmd)), Some(A::Heatmap));
        assert_eq!(action(&tree, key(Key::ArrowDown, cmd)), Some(A::NextSpec));
        assert_eq!(
            action(&tree, key(Key::PageUp, cmd | Modifiers::SHIFT)),
            Some(A::Zoom(3))
        );
        assert_eq!(action(&items, key(Key::E, cmd)), Some(A::EditSlot));
    }

    #[test]
    fn f1_opens_the_wiki_page_of_what_is_hovered_on_release() {
        let release = vec![Event::Key {
            key: Key::F1,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }];
        let list = ShortcutState::default();
        assert_eq!(action(&list, key(Key::F1, Modifiers::NONE)), None);
        assert_eq!(action(&list, release.clone()), Some(ShortcutAction::Help));
        let hover = ShortcutState {
            wiki: Some("Mageblood".into()),
            ..on(BuildTab::Items)
        };
        assert_eq!(
            action(&hover, release),
            Some(ShortcutAction::Wiki(
                "https://www.poewiki.net/wiki/Mageblood".into()
            ))
        );
        assert_eq!(
            wiki_url("Blade Vortex Support"),
            "https://www.poewiki.net/wiki/Blade_Vortex_Support"
        );
    }

    #[test]
    fn typing_holds_what_pobs_selected_control_takes() {
        let typing = ShortcutState {
            typing: true,
            copy_node: true,
            ..on(BuildTab::Tree)
        };
        // Build's and the tab's own loops read these before the controls (Build.lua:1150,
        // TreeTab.lua:359)
        assert_eq!(
            action(&typing, key(Key::Num2, Modifiers::COMMAND)),
            Some(ShortcutAction::Tab(BuildTab::Skills))
        );
        assert_eq!(
            action(&typing, key(Key::F, Modifiers::COMMAND)),
            Some(ShortcutAction::Search)
        );
        assert_eq!(
            action(&typing, key(Key::ArrowDown, Modifiers::NONE)),
            Some(ShortcutAction::NextSpec)
        );
        // the tree view's keys come after them: the selected control has them
        for (k, m) in [
            (Key::P, Modifiers::NONE),
            (Key::D, Modifiers::COMMAND),
            (Key::C, Modifiers::COMMAND),
            (Key::PageUp, Modifiers::NONE),
            (Key::F2, Modifiers::NONE),
        ] {
            assert_eq!(action(&typing, key(k, m)), None, "{k:?}");
        }
        // a box of the top bar or the sidebar is Build's own: the tab gets nothing
        let build_box = ShortcutState {
            build_box: true,
            ..typing.clone()
        };
        assert_eq!(
            action(&build_box, key(Key::ArrowDown, Modifiers::NONE)),
            None
        );
        assert_eq!(
            action(&build_box, key(Key::Num3, Modifiers::COMMAND)),
            Some(ShortcutAction::Tab(BuildTab::Items))
        );
        let back = vec![Event::PointerButton {
            pos: egui::Pos2::new(5.0, 5.0),
            button: PointerButton::Extra1,
            pressed: true,
            modifiers: Modifiers::NONE,
        }];
        assert_eq!(action(&typing, back.clone()), Some(ShortcutAction::Close));
        let modal = ShortcutState {
            modal: true,
            ..on(BuildTab::Tree)
        };
        assert_eq!(action(&modal, back.clone()), None);
        assert_eq!(action(&modal, key(Key::Num2, Modifiers::COMMAND)), None);
        let f1 = vec![Event::Key {
            key: Key::F1,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }];
        // under a window F1 opens only a page: no Help behind it
        assert_eq!(action(&modal, f1.clone()), None);
        let gem = ShortcutState {
            wiki: Some("Fireball".into()),
            ..modal
        };
        assert_eq!(
            action(&gem, f1.clone()),
            Some(ShortcutAction::Wiki(wiki_url("Fireball")))
        );
        // a selected control takes F1's key-up from Main (ControlHost.lua:56-63): no Help,
        // and no tree node page; a hovered list row's page still opens (OnHoverKeyUp)
        assert_eq!(action(&typing, f1.clone()), None);
        let node = ShortcutState {
            wiki: Some("Node".into()),
            ..typing.clone()
        };
        assert_eq!(action(&node, f1.clone()), None);
        let row = ShortcutState {
            wiki: Some("Mageblood".into()),
            hover_wiki: true,
            ..typing
        };
        assert_eq!(
            action(&row, f1),
            Some(ShortcutAction::Wiki(wiki_url("Mageblood")))
        );
        // the list has its own Mouse4 (folder back, build_list.rs)
        assert_eq!(action(&ShortcutState::default(), back), None);
    }

    #[test]
    fn consuming_takes_the_key_and_its_text() {
        let mut i = input(vec![
            Event::Key {
                key: Key::P,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
            Event::Text("p".into()),
            Event::Key {
                key: Key::Q,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
        ]);
        consume(&ShortcutAction::Heatmap, &mut i);
        assert_eq!(i.events.len(), 1);
        assert!(matches!(i.events[0], Event::Key { key: Key::Q, .. }));
    }
}
