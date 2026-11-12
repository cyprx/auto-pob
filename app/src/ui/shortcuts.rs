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
//! Like Ctrl+Z, none fires under a modal window, except F1 over a gem of the gem list's
//! window. The keyboard ones also wait while a text field has the keyboard, except the keys
//! no text field uses that PoB reads whatever has the focus: F1, and Ctrl+F, which goes from
//! one search box to another. The mouse's back button doesn't wait either, as in PoB.

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

fn pressed(input: &egui::InputState, key: Key, command: bool) -> bool {
    input.events.iter().any(|e| {
        matches!(e, Event::Key { key: k, pressed: true, modifiers, .. }
            if *k == key && modifiers.command == command && !modifiers.alt)
    })
}

/// A press of `key` without Ctrl: whether Shift was held, PoB's three zoom steps
fn pressed_shift(input: &egui::InputState, key: Key) -> Option<bool> {
    input.events.iter().find_map(|e| match e {
        Event::Key {
            key: k,
            pressed: true,
            modifiers,
            ..
        } if *k == key && !modifiers.command && !modifiers.alt => Some(modifiers.shift),
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

/// The shortcut this frame's input asks for, if any.
pub fn shortcut_action(state: &ShortcutState, input: &egui::InputState) -> Option<ShortcutAction> {
    use ShortcutAction as A;
    let f1 = released(input, Key::F1);
    if state.modal {
        // a window's own gem list still opens a gem's page (GemSelectControl.lua:822-833)
        return state
            .wiki
            .as_ref()
            .filter(|_| f1)
            .map(|n| A::Wiki(wiki_url(n)));
    }
    if f1 {
        return Some(match &state.wiki {
            Some(name) => A::Wiki(wiki_url(name)),
            None => A::Help,
        });
    }
    let tab = state.tab;
    if tab.is_some() && input.pointer.button_pressed(PointerButton::Extra1) {
        return Some(A::Close);
    }
    // the Calcs tab has its own Ctrl+F (calcs/mod.rs)
    let search = pressed(input, Key::F, true)
        && matches!(
            tab,
            Some(BuildTab::Tree | BuildTab::Items | BuildTab::Config | BuildTab::Compare)
        );
    if state.typing {
        // Ctrl+F goes on from a search box, as in PoB: from the uniques' search it moves to
        // the rares' (ItemsTab.lua:1471-1480)
        return search.then_some(A::Search);
    }
    let tab = tab?;
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
    if search {
        return Some(A::Search);
    }
    match tab {
        BuildTab::Tree => {
            let zoom =
                |key, step: i32| pressed_shift(input, key).map(|s| if s { 3 * step } else { step });
            if pressed(input, Key::M, true) {
                Some(A::SpecManager)
            } else if pressed(input, Key::D, true) {
                Some(A::StatDifferences)
            } else if copy(input) {
                Some(A::CopyNode)
            } else if pressed(input, Key::P, false) {
                Some(A::Heatmap)
            } else if let Some(steps) = zoom(Key::PageUp, 1).or_else(|| zoom(Key::PageDown, -1)) {
                Some(A::Zoom(steps))
            } else if state.spec_select_busy {
                None
            } else if pressed(input, Key::ArrowUp, false) {
                Some(A::PrevSpec)
            } else if pressed(input, Key::ArrowDown, false) {
                Some(A::NextSpec)
            } else {
                None
            }
        }
        BuildTab::Items if pressed(input, Key::E, false) => Some(A::EditSlot),
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
        assert_eq!(action(&tree, vec![Event::Copy]), Some(A::CopyNode));
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
    fn typing_and_modals_hold_the_keys_but_not_mouse4() {
        let typing = ShortcutState {
            typing: true,
            ..on(BuildTab::Tree)
        };
        assert_eq!(action(&typing, key(Key::Num2, Modifiers::COMMAND)), None);
        assert_eq!(action(&typing, key(Key::P, Modifiers::NONE)), None);
        // Ctrl+F still goes to the search box
        assert_eq!(
            action(&typing, key(Key::F, Modifiers::COMMAND)),
            Some(ShortcutAction::Search)
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
        // F1 isn't a typing key
        assert_eq!(action(&typing, f1), Some(ShortcutAction::Help));
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
