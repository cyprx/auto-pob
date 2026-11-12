//! Gate tests for the settings engine (R12, R13): PoB's Options window run as a session over
//! a temp user folder, the overrides over PoB's shared settings, and what reaches
//! `Settings.xml`. Upstream's own `main:LoadSettings()` reads the file back.
//! Included from `gate.rs`, whose helpers it reaches through `super::`.

use super::lock;
use super::longtail_harness::{LongtailSession, control, misc_attributes, upstream_settings};
use auto_pob::engine::{
    Affected, Backend, Command, Event, PopupControlKind, PopupInput, PopupKind, SettingEffect,
    SettingsView, Snapshot, send_snapshots,
};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// The attributes of the seeded `<Misc>`, each away from PoB's default except where the test
/// changes it. The three that are ruled out (`betaTest`, `showPublicBuilds`,
/// `disableDevAutoSave`) are set to values that Save must carry through untouched.
fn seed_attributes(user: &std::path::Path) -> BTreeMap<String, String> {
    let custom = super::longtail_harness::settings_folder(user).join("Custom Builds/");
    std::fs::create_dir_all(&custom).unwrap();
    [
        ("buildSortMode", "LEVEL"),
        ("connectionProtocol", "2"),
        ("proxyURL", "socks5://127.0.0.1:1080"),
        ("buildPath", custom.to_str().unwrap()),
        ("nodePowerTheme", "GREEN/BLUE"),
        ("colorPositive", "^x112233"),
        ("colorNegative", "^x445566"),
        ("colorHighlight", "^x778899"),
        ("showThousandsSeparators", "true"),
        ("thousandsSeparator", "."),
        ("useCompactValues", "true"),
        ("decimalSeparator", ","),
        ("showTitlebarName", "false"),
        ("betaTest", "true"),
        ("edgeSearchHighlight", "false"),
        ("defaultGemQuality", "20"),
        ("defaultCharLevel", "1"),
        ("defaultItemAffixQuality", "0.8"),
        ("showWarnings", "false"),
        ("slotOnlyTooltips", "false"),
        ("migrateEldritchImplicits", "false"),
        ("notSupportedModTooltips", "false"),
        ("invertSliderScrollDirection", "true"),
        ("disableDevAutoSave", "true"),
        ("showPublicBuilds", "false"),
        ("showFlavourText", "false"),
        ("showAnimations", "false"),
        ("showAllItemAffixes", "false"),
        ("disableScrollControlInteraction", "true"),
        ("dpiScaleOverridePercent", "150"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn settings_xml(attributes: &BTreeMap<String, String>) -> String {
    let attrs: String = attributes
        .iter()
        .map(|(k, v)| format!(" {k}=\"{v}\""))
        .collect();
    format!(
        "<PathOfBuilding>\n\t<Mode mode=\"LIST\"><Arg boolean=\"false\"/><Arg string=\"\"/></Mode>\n\t<Misc{attrs}/>\n</PathOfBuilding>\n"
    )
}

/// PoB's own defaults for the rows (headless: the DPI stub answers 1), so the test can show
/// that each row is compared away from them.
fn pob_defaults(user: &std::path::Path) -> BTreeMap<&'static str, String> {
    [
        ("buildSortMode", "NAME".to_string()),
        ("connectionProtocol", "0".into()),
        ("proxyURL", "".into()),
        (
            "buildPath",
            format!(
                "{}/",
                super::longtail_harness::settings_folder(user)
                    .join("Builds")
                    .display()
            ),
        ),
        ("nodePowerTheme", "RED/BLUE".into()),
        ("colorPositive", "^x33FF77".into()),
        ("colorNegative", "^xDD0022".into()),
        ("colorHighlight", "^xFF0000".into()),
        ("showThousandsSeparators", "true".into()),
        ("thousandsSeparator", ",".into()),
        ("useCompactValues", "false".into()),
        ("decimalSeparator", ".".into()),
        ("showTitlebarName", "true".into()),
        ("edgeSearchHighlight", "true".into()),
        ("defaultGemQuality", "0".into()),
        ("defaultCharLevel", "1".into()),
        ("defaultItemAffixQuality", "0.5".into()),
        ("showWarnings", "true".into()),
        ("slotOnlyTooltips", "true".into()),
        ("migrateEldritchImplicits", "true".into()),
        ("notSupportedModTooltips", "true".into()),
        ("invertSliderScrollDirection", "false".into()),
        ("showFlavourText", "true".into()),
        ("showAnimations", "true".into()),
        ("showAllItemAffixes", "true".into()),
        ("disableScrollControlInteraction", "false".into()),
        ("dpiScaleOverridePercent", "1".into()),
    ]
    .into_iter()
    .collect()
}

/// The rows' keys that the app hides (R13)
const HIDDEN: [&str; 3] = ["betaTest", "showPublicBuilds", "disableDevAutoSave"];

/// Compares every row's `value` with `expected`, key for key; nothing is missing or extra
fn assert_values(view: &SettingsView, expected: &BTreeMap<&str, String>, what: &str) {
    let actual: BTreeMap<&str, &str> = view
        .rows
        .iter()
        .map(|r| (r.key.as_str(), r.value.as_str()))
        .collect();
    let want: BTreeMap<&str, &str> = expected.iter().map(|(k, v)| (*k, v.as_str())).collect();
    assert_eq!(actual, want, "{what}: the rows' values");
    assert_eq!(view.rows.len(), expected.len(), "{what}: a row is repeated");
}

fn row<'a>(view: &'a SettingsView, key: &str) -> &'a auto_pob::engine::SettingRow {
    view.rows
        .iter()
        .find(|r| r.key == key)
        .unwrap_or_else(|| panic!("no row {key}"))
}

fn open_options(s: &mut LongtailSession) -> auto_pob::engine::PopupView {
    s.run(Command::OpenPopup(PopupKind::Options)).unwrap();
    let view = s.popup().expect("the Options session is open");
    assert_eq!(view.kind, PopupKind::Options);
    assert_eq!(view.title, "Options");
    assert_eq!(s.open_popups(), 0, "the session sits outside main.popups");
    view
}

fn press(s: &mut LongtailSession, name: &str) -> Affected {
    let affected = s
        .run(Command::PressPopupButton { name: name.into() })
        .unwrap();
    assert!(affected.settings, "a press asks for the settings view");
    assert_eq!(s.open_popups(), 0);
    affected
}

fn set(s: &mut LongtailSession, name: &str, value: PopupInput) {
    s.run(Command::SetPopupControl {
        name: name.into(),
        value,
    })
    .unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(s.open_popups(), 0);
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn settings_shared_and_overridden() {
    let _guard = lock();
    let seed = Mutex::new(BTreeMap::new());
    let mut s = LongtailSession::start("settings", &|user| {
        let attributes = seed_attributes(user);
        let xml = settings_xml(&attributes);
        *seed.lock().unwrap() = attributes;
        xml
    });
    let seed = seed.into_inner().unwrap();
    let defaults = pob_defaults(&s.user);
    let seeded_bytes = s.settings_bytes();
    let (manifest_bytes, manifest_time) = s.manifest();
    // the seed is what upstream reads too, so a row can't pass by agreeing with a wrong seed
    let rows_of_seed: BTreeMap<&str, String> = defaults
        .keys()
        .map(|k| {
            (
                *k,
                seed.get(*k)
                    .cloned()
                    .unwrap_or_else(|| defaults[*k].clone()),
            )
        })
        .collect();

    // ---- the view is answered with no build open ----
    // the seed starts PoB on its list: no build is open, and Options' Save rebuilds the list
    assert!(!s.backend.build_loaded().unwrap(), "no build is open");
    let events = std::cell::RefCell::new(Vec::new());
    send_snapshots(
        &mut s.backend,
        0,
        Affected {
            settings: true,
            popup: true,
            stats: true,
            ..Affected::default()
        },
        &|e| events.borrow_mut().push(e),
    );
    let events = events.into_inner();
    assert!(
        !events.iter().any(|e| matches!(e, Event::Error { .. })),
        "{events:?}"
    );
    let snapshots: Vec<&Snapshot> = events
        .iter()
        .filter_map(|e| match e {
            Event::Snapshot { snapshot, .. } => Some(snapshot),
            _ => None,
        })
        .collect();
    assert!(
        matches!(snapshots[0], Snapshot::Settings(_)),
        "{snapshots:?}"
    );
    assert!(
        matches!(snapshots[1], Snapshot::Popup(None)),
        "{snapshots:?}"
    );
    assert_eq!(snapshots.len(), 2, "no build view without a build");

    let view = s.settings();
    // the seed's values with the two the test changes below still at their defaults
    assert_values(&view, &rows_of_seed, "seed");
    for key in HIDDEN {
        assert!(view.rows.iter().all(|r| r.key != key), "{key} is a row");
    }
    for r in &view.rows {
        assert!(!r.overridden, "{}", r.key);
        assert_eq!(r.shared, r.value, "{}", r.key);
    }
    // the effects R13 lists
    let effect = |key: &str| row(&view, key).effect;
    assert_eq!(effect("dpiScaleOverridePercent"), SettingEffect::Ui);
    assert_eq!(effect("showTitlebarName"), SettingEffect::Ui);
    assert_eq!(effect("nodePowerTheme"), SettingEffect::Ui);
    assert_eq!(effect("edgeSearchHighlight"), SettingEffect::Ui);
    assert_eq!(effect("showAnimations"), SettingEffect::NoEffect);
    assert_eq!(
        effect("invertSliderScrollDirection"),
        SettingEffect::NoEffect
    );
    assert_eq!(
        effect("disableScrollControlInteraction"),
        SettingEffect::NoEffect
    );
    assert_eq!(effect("colorPositive"), SettingEffect::PobText);
    assert_eq!(effect("buildPath"), SettingEffect::PobText);

    // ---- Escape on the window is PoB's Cancel; the DPI dropdown reopens the window ----
    let popup = open_options(&mut s);
    // each row names the window's own control that edits it, and has a label; the sort mode
    // is set from the build list, not here (Task 16's non-default check)
    for r in &view.rows {
        assert!(!r.label.is_empty(), "{}: no label", r.key);
        if r.key == "buildSortMode" {
            assert_eq!(r.control, "");
        } else {
            assert!(
                popup.controls.iter().any(|c| c.name == r.control),
                "{}: the window has no control {:?}",
                r.key,
                r.control
            );
        }
    }
    for key in HIDDEN {
        assert!(
            popup.controls.iter().all(|c| c.name != key),
            "{key} is in the window"
        );
        let err = s
            .run(Command::SetPopupControl {
                name: key.into(),
                value: PopupInput::Check(false),
            })
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("PoB doesn't offer that here"),
            "{key}: {err:#}"
        );
    }
    // the screen is 1920x1080: PoB lays the window out in one column, without a scroll bar
    let thousands = control(&popup, "showThousandsSeparators");
    assert_eq!(thousands.kind, PopupControlKind::Check);
    assert!(thousands.checked, "the seed has them on");
    assert_eq!(
        thousands.pos[0],
        control(&popup, "connectionProtocol").pos[0],
        "one column"
    );
    let dpi = control(&popup, "dpiScaleOverride");
    assert_eq!(dpi.choices[dpi.selected], "150%");
    assert_eq!(s.settings_bytes(), seeded_bytes, "opening the window wrote");
    let index = dpi.choices.iter().position(|c| c == "175%").unwrap();
    set(
        &mut s,
        "dpiScaleOverride",
        PopupInput::Choice {
            index,
            label: "175%".into(),
        },
    );
    let popup = s
        .popup()
        .expect("the dropdown reopened the window, which is still the session");
    assert_eq!(popup.kind, PopupKind::Options);
    let dpi = control(&popup, "dpiScaleOverride");
    assert_eq!(dpi.choices[dpi.selected], "175%");
    assert_eq!(row(&s.settings(), "dpiScaleOverridePercent").value, "175");
    s.run(Command::ClosePopup).unwrap();
    assert!(s.popup().is_none());
    assert_eq!(
        row(&s.settings(), "dpiScaleOverridePercent").value,
        "150",
        "Cancel puts it back"
    );
    assert_eq!(s.settings_bytes(), seeded_bytes, "cancelling wrote");
    assert_eq!(s.open_popups(), 0);

    // ---- shared: the session sets two settings and presses Save ----
    open_options(&mut s);
    set(&mut s, "showThousandsSeparators", PopupInput::Check(false));
    set(&mut s, "defaultCharLevel", PopupInput::Text("90".into()));
    assert_eq!(s.settings_bytes(), seeded_bytes, "setting controls wrote");
    press(&mut s, "save");
    assert!(
        s.popup().is_none(),
        "Save closes the window and ends the session"
    );
    let mut attributes = seed.clone();
    attributes.insert("showThousandsSeparators".into(), "false".into());
    attributes.insert("defaultCharLevel".into(), "90".into());
    let xml = String::from_utf8(s.settings_bytes()).unwrap();
    // every attribute, the hidden three and the stored protocol included
    assert_eq!(misc_attributes(&xml), attributes, "Settings.xml after Save");
    let theirs = upstream_settings(
        &s.user,
        &[
            "showThousandsSeparators",
            "defaultCharLevel",
            "colorPositive",
            "betaTest",
            "showPublicBuilds",
        ],
    );
    assert_eq!(theirs["showThousandsSeparators"], serde_json::json!(false));
    assert_eq!(theirs["defaultCharLevel"], serde_json::json!(90));
    assert_eq!(theirs["colorPositive"], serde_json::json!("^x112233"));
    assert_eq!(theirs["betaTest"], serde_json::json!(true));
    assert_eq!(theirs["showPublicBuilds"], serde_json::json!(false));
    // the pinned folder's manifest: Save would have switched its branch
    assert!(
        s.manifest() == (manifest_bytes.clone(), manifest_time),
        "manifest.xml changed"
    );
    // every row, at a value away from PoB's default
    let mut after_save = rows_of_seed.clone();
    after_save.insert("showThousandsSeparators", "false".into());
    after_save.insert("defaultCharLevel", "90".into());
    let view = s.settings();
    assert_values(&view, &after_save, "after Save");
    for (key, default) in &defaults {
        assert_ne!(
            &row(&view, key).value,
            default,
            "{key} is compared at its default"
        );
    }
    assert!(view.saved_overrides.is_empty(), "no override was in play");
    assert_eq!(s.open_popups(), 0);

    // ---- overridden: the app's own values over the shared ones ----
    let before = s.settings_bytes();
    let overrides: BTreeMap<String, String> = [
        ("showThousandsSeparators".to_string(), "true".to_string()),
        ("colorPositive".to_string(), "^x1A2B3C".to_string()),
    ]
    .into();
    let affected = s
        .run(Command::SetOverrides {
            values: overrides.clone(),
        })
        .unwrap();
    assert!(affected.settings && !affected.popup);
    assert_eq!(s.settings_bytes(), before, "setting overrides wrote");
    let view = s.settings();
    let thousands = row(&view, "showThousandsSeparators");
    assert!(thousands.overridden);
    assert_eq!(
        (thousands.shared.as_str(), thousands.value.as_str()),
        ("false", "true")
    );
    let positive = row(&view, "colorPositive");
    assert!(positive.overridden);
    assert_eq!(
        (positive.shared.as_str(), positive.value.as_str()),
        ("^x112233", "^x1A2B3C")
    );
    assert_eq!(view.rows.iter().filter(|r| r.overridden).count(), 2);
    assert!(s.eval::<bool>("return pob.main.main.showThousandsSeparators == true"));
    assert!(s.eval::<bool>("return colorCodes.POSITIVE == '^x1A2B3C'"));
    // the session shows the override: PoB's checkbox reads checked
    let popup = open_options(&mut s);
    assert!(control(&popup, "showThousandsSeparators").checked);
    assert_eq!(control(&popup, "colorPositive").text, "0x1A2B3C");
    // the user changes an overridden row and a shared one, and saves
    set(&mut s, "colorPositive", PopupInput::Text("0x445566".into()));
    set(&mut s, "defaultGemQuality", PopupInput::Text("21".into()));
    press(&mut s, "save");
    assert!(s.popup().is_none());
    let view = s.settings();
    assert_eq!(
        view.saved_overrides,
        BTreeMap::from([("colorPositive".to_string(), "0x445566".to_string())]),
        "the UI is handed the overridden row the user changed, and only that"
    );
    attributes.insert("defaultGemQuality".into(), "21".into());
    let xml = String::from_utf8(s.settings_bytes()).unwrap();
    assert_eq!(
        misc_attributes(&xml),
        attributes,
        "overrides stayed out of Settings.xml"
    );
    assert!(s.eval::<bool>("return pob.main.main.showThousandsSeparators == true"));
    assert!(s.eval::<bool>("return pob.main.main.colorPositive == '0x445566'"));
    let positive = row(&view, "colorPositive");
    assert_eq!(
        (positive.shared.as_str(), positive.value.as_str()),
        ("^x112233", "0x445566")
    );
    assert_eq!(row(&view, "defaultGemQuality").value, "21");
    assert!(!row(&view, "defaultGemQuality").overridden);
    assert!(s.settings().saved_overrides.is_empty(), "taken once");
    // the UI writes auto-pob.toml and sends the map again: nothing changes, nothing is written
    let saved = s.settings_bytes();
    s.run(Command::SetOverrides {
        values: [
            ("showThousandsSeparators".to_string(), "true".to_string()),
            ("colorPositive".to_string(), "0x445566".to_string()),
        ]
        .into(),
    })
    .unwrap();
    assert_eq!(s.settings_bytes(), saved);
    assert_eq!(row(&s.settings(), "colorPositive").value, "0x445566");

    // ---- the saves PoB makes by itself keep the shared values on disk (Review Focus 3) ----
    // the token and sign-out paths call this; Shutdown calls it last
    s.eval::<()>("pob.main.main:SaveSettings()");
    let xml = String::from_utf8(s.settings_bytes()).unwrap();
    assert_eq!(
        misc_attributes(&xml),
        attributes,
        "SaveSettings by PoB's own paths"
    );
    s.eval::<()>("pob.main.main:Shutdown()");
    let xml = String::from_utf8(s.settings_bytes()).unwrap();
    assert_eq!(misc_attributes(&xml), attributes, "Shutdown");
    assert!(
        s.eval::<bool>("return pob.main.main.showThousandsSeparators == true"),
        "restored"
    );
    assert!(
        s.eval::<bool>("return pob.main.main.colorPositive == '0x445566'"),
        "restored"
    );

    // ---- override off: the shared values are back in PoB ----
    let saved = s.settings_bytes();
    s.run(Command::SetOverrides {
        values: BTreeMap::new(),
    })
    .unwrap();
    assert_eq!(s.settings_bytes(), saved);
    let view = s.settings();
    assert!(view.rows.iter().all(|r| !r.overridden));
    let mut expected = after_save.clone();
    expected.insert("defaultGemQuality", "21".into());
    assert_values(&view, &expected, "override off");
    assert!(s.eval::<bool>("return pob.main.main.showThousandsSeparators == false"));
    assert!(s.eval::<bool>("return colorCodes.POSITIVE == '^x112233'"));

    // ---- nothing leaked ----
    assert!(
        s.manifest() == (manifest_bytes, manifest_time),
        "manifest.xml changed"
    );
    assert_eq!(s.open_popups(), 0);
}

fn misc(s: &LongtailSession) -> BTreeMap<String, String> {
    misc_attributes(&String::from_utf8(s.settings_bytes()).unwrap())
}

#[test]
#[ignore = "needs the pinned PoB install"]
fn options_cancel_sessions_and_unsaved_edits() {
    let _guard = lock();
    let seed = Mutex::new(BTreeMap::new());
    let mut s = LongtailSession::start("options-cancel", &|user| {
        let attributes = seed_attributes(user);
        let xml = settings_xml(&attributes);
        *seed.lock().unwrap() = attributes;
        xml
    });
    let seed = seed.into_inner().unwrap();
    let seeded_bytes = s.settings_bytes();

    // ---- I1: Cancel after the protocol dropdown leaves the stored protocol alone ----
    let popup = open_options(&mut s);
    let protocol = control(&popup, "connectionProtocol");
    assert_eq!(
        protocol.choices[protocol.selected], "IPv6",
        "the seed stores 2"
    );
    set(
        &mut s,
        "connectionProtocol",
        PopupInput::Choice {
            index: 1,
            label: "IPv4".into(),
        },
    );
    s.run(Command::ClosePopup).unwrap();
    assert!(
        s.eval::<bool>("return pob.main.connectionProtocol == 2"),
        "live value"
    );
    assert_eq!(row(&s.settings(), "connectionProtocol").value, "2");
    // a later untouched Save must not write the abandoned pick
    open_options(&mut s);
    press(&mut s, "save");
    assert_eq!(misc(&s)["connectionProtocol"], "2");
    assert_eq!(misc(&s), seed, "an untouched Save changes nothing");

    // ---- I3: an override set while the window is open survives its Cancel ----
    open_options(&mut s);
    s.run(Command::SetOverrides {
        values: [("defaultCharLevel".to_string(), "77".to_string())].into(),
    })
    .unwrap();
    assert!(s.popup().is_some(), "SetOverrides keeps the session");
    press(&mut s, "cancel");
    assert!(s.popup().is_none());
    let view = s.settings();
    assert!(row(&view, "defaultCharLevel").overridden);
    assert_eq!(row(&view, "defaultCharLevel").value, "77");
    assert!(view.saved_overrides.is_empty());
    s.eval::<()>("pob.main.main:SaveSettings()");
    assert_eq!(misc(&s), seed, "the override stayed out of the file");
    assert!(
        s.settings().saved_overrides.is_empty(),
        "and was not taken for an edit"
    );

    // ---- I2: a build change leaves the Options window open (R19) ----
    open_options(&mut s);
    set(&mut s, "defaultGemQuality", PopupInput::Text("5".into()));
    s.run(Command::NewBuild).unwrap();
    assert_eq!(
        s.popup().expect("Options survives NewBuild").kind,
        PopupKind::Options
    );
    assert_eq!(s.open_popups(), 0);
    assert!(
        s.eval::<bool>("return pob.main.main.defaultGemQuality == 5"),
        "edit still live"
    );

    // ---- I4: a save PoB makes by itself never writes the window's edits (R29) ----
    set(&mut s, "showWarnings", PopupInput::Check(true));
    s.eval::<()>("pob.main.main:SaveSettings()");
    assert_eq!(misc(&s), seed, "an unsaved edit reached Settings.xml");
    assert!(s.popup().is_none(), "the window was cancelled");
    assert!(s.eval::<bool>("return glue.popup_take_lost()"));
    assert!(
        s.eval::<bool>("return pob.main.main.defaultGemQuality == 20"),
        "cancelled"
    );
    assert!(
        s.eval::<bool>("return pob.main.main.showWarnings == false"),
        "cancelled"
    );
    // Shutdown saves too
    open_options(&mut s);
    set(&mut s, "defaultGemQuality", PopupInput::Text("6".into()));
    s.eval::<()>("pob.main.main:Shutdown()");
    assert_eq!(misc(&s), seed, "Shutdown wrote an unsaved edit");
    assert!(s.popup().is_none());
    // an overridden row edited in the window: the save neither writes it nor adopts it
    s.run(Command::SetOverrides {
        values: [("showWarnings".to_string(), "true".to_string())].into(),
    })
    .unwrap();
    open_options(&mut s);
    set(&mut s, "showWarnings", PopupInput::Check(false));
    s.eval::<()>("pob.main.main:SaveSettings()");
    assert_eq!(misc(&s), seed);
    let view = s.settings();
    assert!(
        view.saved_overrides.is_empty(),
        "{:?}",
        view.saved_overrides
    );
    assert_eq!(row(&view, "showWarnings").value, "true");
    // opening Options over Options cancels the first window
    open_options(&mut s);
    set(&mut s, "defaultGemQuality", PopupInput::Text("7".into()));
    open_options(&mut s);
    s.run(Command::ClosePopup).unwrap();
    assert!(s.eval::<bool>("return pob.main.main.defaultGemQuality == 20"));
    assert_eq!(s.open_popups(), 0);
    let _ = seeded_bytes;
}
