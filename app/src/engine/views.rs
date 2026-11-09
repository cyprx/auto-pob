//! Typed snapshots of PoB state, deserialised from Lua tables.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BuildView {
    pub name: String,
    #[serde(default)]
    pub path: Option<PathBuf>,
    pub class: String,
    #[serde(default)]
    pub ascendancy: Option<String>,
    pub level: u32,
    pub unsaved: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BuildListView {
    pub folder: String,
    #[serde(default)]
    pub entries: Vec<BuildListEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BuildListEntry {
    pub name: String,
    pub is_folder: bool,
    pub path: PathBuf,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub ascendancy: Option<String>,
    #[serde(default)]
    pub level: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StatsView {
    #[serde(default)]
    pub stats: Vec<StatRow>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StatRow {
    pub key: String,
    pub label: String,
    pub value: f64,
    pub text: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConfigView {
    #[serde(default)]
    pub sets: Vec<ConfigSet>,
    pub active_set: u32,
    pub custom_mods: String,
    #[serde(default)]
    pub sections: Vec<ConfigSection>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConfigSet {
    pub id: u32,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConfigSection {
    pub name: String,
    pub column: u32,
    #[serde(default)]
    pub options: Vec<ConfigOption>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConfigOption {
    pub var: String,
    pub kind: ConfigKind,
    #[serde(default)]
    pub allow_zero: bool,
    pub label: String,
    pub shown: bool,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub placeholder: Option<serde_json::Value>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub tooltip: Vec<TooltipLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigKind {
    Check,
    Count,
    Integer,
    Float,
    List,
    Text,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Choice {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum TooltipLine {
    Line { size: u32, text: String },
    Separator { separator: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::LuaSerdeExt;

    fn lua_value<T: serde::de::DeserializeOwned>(code: &str) -> T {
        let lua = mlua::Lua::new();
        let value: mlua::Value = lua.load(code).eval().unwrap();
        lua.from_value(value).unwrap()
    }

    #[test]
    fn config_view_from_lua_table() {
        let view: ConfigView = lua_value(
            r#"return {
              sets = { { id = 1, title = "Default" } },
              active_set = 1,
              custom_mods = "+1 to all",
              sections = { {
                name = "General", column = 1,
                options = {
                  { var = "usePowerCharges", kind = "check", allow_zero = false,
                    label = "Use power charges?", shown = true, value = true,
                    choices = {}, tooltip = {} },
                  { var = "enemyIsBoss", kind = "list", allow_zero = false,
                    label = "Enemy is a Boss?", shown = true, value = "None",
                    placeholder = 5,
                    choices = { { label = "No", value = "None" }, { label = "Yes", value = "Boss" } },
                    tooltip = { { size = 14, text = "Hello" }, { separator = 8 } } },
                } } },
            }"#,
        );
        let expected = ConfigView {
            sets: vec![ConfigSet {
                id: 1,
                title: "Default".into(),
            }],
            active_set: 1,
            custom_mods: "+1 to all".into(),
            sections: vec![ConfigSection {
                name: "General".into(),
                column: 1,
                options: vec![
                    ConfigOption {
                        var: "usePowerCharges".into(),
                        kind: ConfigKind::Check,
                        allow_zero: false,
                        label: "Use power charges?".into(),
                        shown: true,
                        value: Some(serde_json::json!(true)),
                        placeholder: None,
                        choices: vec![],
                        tooltip: vec![],
                    },
                    ConfigOption {
                        var: "enemyIsBoss".into(),
                        kind: ConfigKind::List,
                        allow_zero: false,
                        label: "Enemy is a Boss?".into(),
                        shown: true,
                        value: Some(serde_json::json!("None")),
                        placeholder: Some(serde_json::json!(5)),
                        choices: vec![
                            Choice {
                                label: "No".into(),
                                value: "None".into(),
                            },
                            Choice {
                                label: "Yes".into(),
                                value: "Boss".into(),
                            },
                        ],
                        tooltip: vec![
                            TooltipLine::Line {
                                size: 14,
                                text: "Hello".into(),
                            },
                            TooltipLine::Separator { separator: 8 },
                        ],
                    },
                ],
            }],
        };
        assert_eq!(view, expected);
    }

    #[test]
    fn optional_fields_may_be_missing() {
        let view: BuildView = lua_value(
            r#"return { name = "n", class = "Witch", level = 90,
                        unsaved = false, can_undo = false, can_redo = true }"#,
        );
        assert_eq!(view.path, None);
        assert_eq!(view.ascendancy, None);
        assert_eq!(view.level, 90);
        assert!(view.can_redo);
    }
}
