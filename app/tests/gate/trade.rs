//! Buy similar (R10, R20): PoB's trade search window opened from our three entry points, and
//! the link its search button makes, against upstream PoB opening the same window from its
//! display panel and pressing the same controls.

use super::net_harness::{NetSession, canned, canned_web, net_session};
use super::{fixture_xml, lock, pob_dir};
use auto_pob::engine::upstream::{load_xml, upstream_lua};
use auto_pob::engine::{
    Affected, Backend, BuySource, Command, CompareSource, PopupControlKind, PopupInput, PopupKind,
    PopupView,
};
use mlua::LuaSerdeExt;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Upstream's side, written apart from the glue: PoB's own display-panel button opens the
/// window, the league fetch's callback is played with the list it would have fetched, and every
/// change is a press of the window's own controls. `OpenURL` and `LaunchSubScript` record.
const UPSTREAM_LIB: &str = r#"
launches = 0
local stubLaunch = LaunchSubScript
LaunchSubScript = function(...)
    launches = launches + 1
    return stubLaunch(...)
end
local opened
OpenURL = function(url) opened = url end

local function popup()
    local p = main.popups[1]
    assert(p, "no window is open")
    return p
end

-- The item list's double click (ItemListControl.lua:315-321), then the panel's Buy similar.
-- The realm is picked as a user would; the league list is what the refused fetch's callback
-- would have set (CompareBuySimilar.lua:323-339), its pick made as the callback makes it. The
-- callback's own `rebuildUrl` is reached through another control: Listed, there and back.
local function openDisplayed(realm, leagues, leagueIndex)
    local button = build.itemsTab.controls.displayItemBuySimilar
    assert(button:IsShown(), "Buy similar is not shown")
    local depth = #main.popups
    button.onClick()
    assert(#main.popups == depth + 1, "Buy similar opened no window")
    local p = popup()
    for i, name in ipairs(p.controls.realmDrop.list) do
        if name == realm then
            p.controls.realmDrop:SetSel(i)
        end
    end
    p.controls.leagueDrop:SetList(leagues)
    p.controls.leagueDrop:SetSel(leagueIndex, true)
    local listed = p.controls.listedDrop
    local was = listed.selIndex
    listed:SetSel(was == 1 and 2 or 1)
    listed:SetSel(was)
end

function openBuy(itemId, realm, leagues, leagueIndex)
    build.itemsTab.controls.itemList:OnSelClick(1, itemId, true)
    openDisplayed(realm, leagues, leagueIndex)
end

-- Ctrl+V on the Items tab (ItemsTab.lua:1477), then the panel's Buy similar
function openPasted(text, realm, leagues, leagueIndex)
    build.itemsTab:CreateDisplayItemFromRaw(text, true)
    openDisplayed(realm, leagues, leagueIndex)
end

-- Sets a control of the window as the user would; `value` is a gate `PopupInput`
function setControl(name, value)
    local c = popup().controls[name]
    assert(c and c:IsShown() and c:IsEnabled(), name .. " is not usable")
    if value.Choice then
        c:SetSel(value.Choice.index + 1)
    elseif value.Check ~= nil then
        c.state = value.Check
        c.changeFunc(value.Check)
    elseif value.Text then
        c:SetText(value.Text, true)
    else
        error("unknown input")
    end
end

-- Presses Open URL; returns the link it opened, or nil while it is disabled
function search()
    opened = nil
    local button = popup().controls.search
    if not button:IsEnabled() then
        return nil
    end
    button.onClick()
    return opened
end

function closeBuy()
    popup().controls.close.onClick()
    assert(#main.popups == 0, "the window stayed open")
end

-- The equipped items, in PoB's slot order: { slot, id }
function equipped()
    local out = {}
    local tab = build.itemsTab
    for _, slot in ipairs(tab.orderedSlots) do
        if slot:IsShown() and slot.selItemId ~= 0 and tab.items[slot.selItemId] then
            table.insert(out, { slot = slot.slotName, id = slot.selItemId })
        end
    end
    return out
end
"#;

/// A rare helmet with an armour row and searchable mods, pasted on both sides
const ARMOUR: &str = "Rarity: RARE\nDoom Visor\nRoyal Burgonet\nQuality: 20\nImplicits: 0\n+80 to maximum Life\n+40% to Fire Resistance\n50% increased Armour";

struct Upstream {
    lua: mlua::Lua,
}

impl Upstream {
    fn new(xml: &str, name: &str) -> Upstream {
        let lua = upstream_lua(&pob_dir()).expect("starting upstream PoB");
        // the wrapper runs in dev mode, which writes a build to its file when it is left
        lua.load("launch.devMode = false").exec().unwrap();
        load_xml(&lua, xml, name).unwrap();
        lua.load(UPSTREAM_LIB)
            .set_name("=trade upstream")
            .exec()
            .expect("trade upstream lib");
        Upstream { lua }
    }

    fn eval<T: mlua::FromLuaMulti>(&self, code: &str) -> T {
        self.lua
            .load(code)
            .set_name("=upstream trade")
            .eval()
            .unwrap_or_else(|e| panic!("{code}: {e}"))
    }

    fn call<T: mlua::FromLuaMulti>(&self, function: &str, args: impl mlua::IntoLuaMulti) -> T {
        let f: mlua::Function = self.lua.globals().get(function).unwrap();
        f.call(args).unwrap_or_else(|e| panic!("{function}: {e}"))
    }

    fn equipped(&self) -> Vec<(String, u32)> {
        #[derive(serde::Deserialize)]
        struct Raw {
            slot: String,
            id: u32,
        }
        let value: mlua::Value = self.eval("return equipped()");
        let raw: Vec<Raw> = self.lua.from_value(value).unwrap();
        raw.into_iter().map(|r| (r.slot, r.id)).collect()
    }

    /// Opens the window on item `id`, makes `edits` and presses Open URL
    fn url(&self, id: u32, realm: &str, league: &str, edits: &[(String, PopupInput)]) -> String {
        let (list, index) = leagues(league);
        self.call::<()>("openBuy", (id, realm.to_string(), list, index));
        self.finish(&format!("item {id}"), edits)
    }

    /// `url` for an item pasted onto the display panel
    fn pasted_url(
        &self,
        text: &str,
        realm: &str,
        league: &str,
        edits: &[(String, PopupInput)],
    ) -> String {
        let (list, index) = leagues(league);
        self.call::<()>(
            "openPasted",
            (text.to_string(), realm.to_string(), list, index),
        );
        self.finish("the pasted item", edits)
    }

    fn finish(&self, what: &str, edits: &[(String, PopupInput)]) -> String {
        for (name, value) in edits {
            let value = self.lua.to_value(value).unwrap();
            self.call::<()>("setControl", (name.clone(), value));
        }
        let url: Option<String> = self.call("search", ());
        self.call::<()>("closeBuy", ());
        url.unwrap_or_else(|| panic!("upstream: Open URL stayed disabled for {what}"))
    }
}

/// The league list PoB's fetch would set for the app's `league`, and the index of `league`
/// (CompareBuySimilar.lua:323-339; R10)
fn leagues(league: &str) -> (Vec<String>, usize) {
    let fixed = ["Standard", "Hardcore", "Ruthless", "Hardcore Ruthless"];
    let mut list: Vec<String> = Vec::new();
    if !fixed.contains(&league) {
        list.push(league.to_string());
    }
    list.extend(fixed.iter().map(|s| s.to_string()));
    let index = list.iter().position(|l| l == league).unwrap() + 1;
    (list, index)
}

/// Every file under `root`, with its bytes
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn run(s: &mut NetSession, command: Command) -> anyhow::Result<Affected> {
    let what = format!("{command:?}");
    let out = s.backend.apply(command, &|_| {});
    if out.as_ref().is_ok_and(|a| a.recalc) {
        s.backend.recalculate().unwrap();
    }
    let popups: usize = s.eval("return #pob.main.main.popups");
    assert_eq!(popups, 0, "{what} left a popup open");
    out
}

fn popup(s: &mut NetSession) -> PopupView {
    s.backend
        .popup_view()
        .unwrap()
        .expect("no Buy similar window is open")
}

/// The non-default choices of R10's second round, read off our window: Listed "Any", the base
/// type checked, the first defence row checked with its Min cleared and a Max, every searchable
/// mod row toggled and the first mod's Min set to 1
fn edits_of(view: &PopupView) -> Vec<(String, PopupInput)> {
    let mut edits = vec![(
        "listedDrop".to_string(),
        PopupInput::Choice {
            index: 3,
            label: "Any".into(),
        },
    )];
    let usable = |name: &str| {
        view.controls
            .iter()
            .find(|c| c.name == name && c.shown && c.enabled)
    };
    if usable("baseTypeCheck").is_some() {
        edits.push(("baseTypeCheck".into(), PopupInput::Check(true)));
    }
    if usable("def1Check").is_some() {
        edits.push(("def1Check".into(), PopupInput::Check(true)));
        edits.push(("def1Min".into(), PopupInput::Text(String::new())));
        // with the Min cleared, a Max keeps the row in the search
        edits.push(("def1Max".into(), PopupInput::Text("5000".into())));
    }
    let mut mods: Vec<_> = view
        .controls
        .iter()
        .filter(|c| {
            c.kind == PopupControlKind::Check
                && c.name.starts_with("mod")
                && c.name.ends_with("Check")
                && c.shown
                && c.enabled
        })
        .collect();
    mods.sort_by_key(|c| c.name.clone());
    for c in mods {
        edits.push((c.name.clone(), PopupInput::Check(!c.checked)));
    }
    if usable("mod1Min").is_some() {
        edits.push(("mod1Min".into(), PopupInput::Text("1".into())));
    }
    edits
}

/// Opens our window from `source`, makes the edits (the non-default round reads them off the
/// window first) and presses Open URL. Returns the link and the edits made.
fn our_url(
    s: &mut NetSession,
    source: BuySource,
    non_default: bool,
) -> (String, Vec<(String, PopupInput)>) {
    run(s, Command::OpenPopup(PopupKind::BuySimilar { source })).expect("opening Buy similar");
    let view = popup(s);
    assert_eq!(view.title, "Buy Similar");
    // its number boxes are TradeHelpers.newPlainNumericEdit's: `isNumeric` off, so no + or -
    // and no stepping (TradeHelpers.lua:548-559)
    for c in view
        .controls
        .iter()
        .filter(|c| c.kind == PopupControlKind::Edit)
    {
        assert!(!c.numeric, "{} steps, as PoB's doesn't", c.name);
    }
    let edits = if non_default {
        edits_of(&view)
    } else {
        Vec::new()
    };
    for (name, value) in &edits {
        run(
            s,
            Command::SetPopupControl {
                name: name.clone(),
                value: value.clone(),
            },
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    let pressed = run(
        s,
        Command::PressPopupButton {
            name: "search".into(),
        },
    )
    .expect("pressing Open URL");
    // the window stays open after a search
    assert!(s.backend.popup_view().unwrap().is_some());
    let url = pressed.external_url.expect("Open URL answered no link").url;
    let closed = run(
        s,
        Command::PressPopupButton {
            name: "close".into(),
        },
    )
    .unwrap();
    assert!(closed.external_url.is_none());
    assert!(
        s.backend.popup_view().unwrap().is_none(),
        "Close left it open"
    );
    (url, edits)
}

/// Our equipped items, as the Items view shows them: (slot, item id)
fn our_equipped(s: &NetSession) -> Vec<(String, u32)> {
    let value: mlua::Value = s.eval(
        "local out = {} local tab = pob.build.itemsTab
         for _, slot in ipairs(tab.orderedSlots) do
            if slot:IsShown() and slot.selItemId ~= 0 and tab.items[slot.selItemId] then
                table.insert(out, { slot.slotName, slot.selItemId })
            end
         end
         return out",
    );
    let raw: Vec<(String, u32)> = s.backend.lua.from_value(value).unwrap();
    raw
}

fn open_xml(s: &mut NetSession, name: &str, xml: &str) -> PathBuf {
    let path = s.user.join(format!("Path of Building/Builds/{name}.xml"));
    fs::write(&path, xml).unwrap();
    run(s, Command::OpenBuild { path: path.clone() }).expect("open");
    path
}

#[test]
#[ignore]
fn trade_urls_match_pob() {
    let _g = lock();
    let started = Instant::now();
    let web = canned_web();
    let mut s = net_session("trade-urls", canned(&web), None);
    let gear_xml = super::items_harness::setup_xml("gear");
    let attack_xml = fs::read_to_string(fixture_xml("attack")).unwrap();
    let refused = s.backend.net_handle().refused();
    let (mut saw_def, mut saw_mod, mut saw_unique) = (false, false, false);
    let mut compared = 0;

    // one upstream state, as ours: the window's module remembers Listed and the realm across
    // builds on both sides
    let up = Upstream::new(&attack_xml, "attack");
    for (name, xml) in [("attack", &attack_xml), ("gear", &gear_xml)] {
        let path = open_xml(&mut s, name, xml);
        if name == "gear" {
            load_xml(&up.lua, xml, name).unwrap();
        }
        let items = our_equipped(&s);
        assert!(!items.is_empty(), "{name}: no equipped item");
        eprintln!("{name}: {items:?}");
        assert_eq!(items, up.equipped(), "{name}: the equipped items differ");
        let before = files(&s.user);

        for (round, (realm, league)) in [("PC", "Standard"), ("PS4", "Settlers")]
            .into_iter()
            .enumerate()
        {
            run(
                &mut s,
                Command::SetTradeDefaults {
                    realm: realm.into(),
                    league: league.into(),
                },
            )
            .unwrap();
            for (i, (slot, id)) in items.iter().enumerate() {
                // the display panel's button for the first item, the slot menu for the rest
                let source = if i == 0 {
                    run(&mut s, Command::EditItem { id: *id }).unwrap();
                    BuySource::DisplayItem
                } else {
                    BuySource::Slot(slot.clone())
                };
                let (ours, edits) = our_url(&mut s, source, round == 1);
                if round == 1 {
                    saw_def |= edits.iter().any(|(n, _)| n == "def1Min");
                    saw_mod |= edits.iter().any(|(n, _)| n == "mod1Min");
                }
                saw_unique |= ours.contains("%22name%22");
                let theirs = up.url(*id, realm, league, &edits);
                assert_eq!(
                    ours, theirs,
                    "{name} {slot} {realm} {league}: the links differ"
                );
                assert!(ours.starts_with("https://www.pathofexile.com/trade/search/"));
                if round == 1 {
                    assert!(
                        ours.starts_with("https://www.pathofexile.com/trade/search/sony/Settlers?"),
                        "{ours}"
                    );
                }
                compared += 1;
            }
            if round == 0 {
                run(&mut s, Command::CancelDisplayItem { if_id: None }).ok();
            }
        }
        assert_eq!(files(&s.user), before, "{name}: a file was written");
        assert!(path.exists());
    }
    // ---- A pasted rare armour, never equipped: its defence rows and PoB's own slot ----
    run(
        &mut s,
        Command::PasteItem {
            text: ARMOUR.into(),
        },
    )
    .unwrap();
    let (ours, edits) = our_url(&mut s, BuySource::DisplayItem, true);
    saw_def |= edits.iter().any(|(n, _)| n == "def1Min");
    saw_mod |= edits.iter().any(|(n, _)| n == "mod1Min");
    let theirs = up.pasted_url(ARMOUR, "PS4", "Settlers", &edits);
    assert_eq!(ours, theirs, "the pasted armour: the links differ");
    // `"ar":{"max":5000}`: the Min cleared, the row searched
    assert!(
        ours.contains("%22ar%22%3A%7B%22max%22%3A5000%7D"),
        "no armour filter: {ours}"
    );
    run(&mut s, Command::CancelDisplayItem { if_id: None }).unwrap();
    compared += 1;

    assert!(saw_def, "no item had a defence row (def1Min)");
    assert!(
        saw_mod,
        "no item had a searchable mod with a value (mod1Min)"
    );
    assert!(saw_unique, "no unique was searched by name");

    // ---- Compare's Buy: the comparison's item, with its row's slot ----
    let gear_up = up;
    open_xml(&mut s, "attack", &attack_xml);
    let gear_path = s.user.join("Path of Building/Builds/gear.xml");
    run(
        &mut s,
        Command::LoadCompare {
            source: CompareSource::File(gear_path),
        },
    )
    .unwrap();
    let belt: u32 = gear_up.eval("return build.itemsTab.slots['Belt'].selItemId");
    let (ours, _) = our_url(&mut s, BuySource::CompareSlot("Belt".into()), false);
    let theirs = gear_up.url(belt, "PS4", "Settlers", &[]);
    assert_eq!(ours, theirs, "Compare Belt: the links differ");

    // every league fetch so far was refused, as upstream's stub returned nothing for each
    let ours_refused = s.backend.net_handle().refused() - refused;
    let upstream_launches: usize = gear_up.eval("return launches");
    assert!(ours_refused > 0, "no league fetch was tried");
    assert_eq!(ours_refused, upstream_launches, "the fetches tried differ");

    // ---- A realm picked in the window keeps a league ----
    let slot = our_equipped(&s)[0].0.clone();
    run(
        &mut s,
        Command::OpenPopup(PopupKind::BuySimilar {
            source: BuySource::Slot(slot),
        }),
    )
    .unwrap();
    run(
        &mut s,
        Command::SetPopupControl {
            name: "realmDrop".into(),
            value: PopupInput::Choice {
                index: 2,
                label: "Xbox".into(),
            },
        },
    )
    .unwrap();
    let league = popup(&mut s)
        .controls
        .into_iter()
        .find(|c| c.name == "leagueDrop")
        .unwrap();
    assert!(league.enabled, "the realm pick left the leagues loading");
    let url = run(
        &mut s,
        Command::PressPopupButton {
            name: "search".into(),
        },
    )
    .unwrap()
    .external_url
    .unwrap()
    .url;
    assert!(
        url.starts_with("https://www.pathofexile.com/trade/search/xbox/Settlers?"),
        "{url}"
    );
    run(&mut s, Command::ClosePopup).unwrap();

    // ---- No item, no window ----
    let err = run(
        &mut s,
        Command::OpenPopup(PopupKind::BuySimilar {
            source: BuySource::Slot("Ring 3".into()),
        }),
    )
    .unwrap_err()
    .to_string();
    assert_eq!(err, "PoB doesn't offer that here");
    assert!(s.backend.popup_view().unwrap().is_none());

    // ---- Only trade search links are kept ----
    let evil: Option<String> =
        s.eval("return glue.capture_url(function() OpenURL('https://evil.test') end)");
    assert_eq!(evil, None, "an unmatched link was kept");
    let kept: Option<String> = s.eval(
        "return glue.capture_url(function() OpenURL('https://www.pathofexile.com/trade/search/Standard?q=1') end)",
    );
    assert_eq!(
        kept.as_deref(),
        Some("https://www.pathofexile.com/trade/search/Standard?q=1")
    );

    // ---- No request: the window's open and its realm pick each fetched, refused ----
    assert_eq!(
        s.backend.net_handle().refused() - refused,
        ours_refused + 2,
        "the realm pick's fetches"
    );
    assert!(
        s.backend.net_handle().running_ids().is_empty(),
        "a subscript runs"
    );
    assert!(web.requests().is_empty(), "a request was made");
    assert!(web.unrouted().is_empty());
    eprintln!("compared {compared} links");
    let secs = started.elapsed().as_secs_f64();
    assert!(secs < 25.0, "took {secs:.1}s");
}
