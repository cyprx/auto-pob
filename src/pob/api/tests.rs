//! Contract tests for the Lua API: each test runs a Lua snippet against the registered globals,
//! without a window or GPU, and checks the behavior PoB relies on.
use std::{cell::RefCell, path::Path, rc::Rc};

use mlua::{Lua, Value};

use crate::{
    app::pob_font_definitions,
    color::Srgba,
    fonts::Fonts,
    pob::{Context, api::register_globals, subscript::SubscriptManager},
    renderer::textures::TextureManager,
};

fn test_lua(script_dir: &Path) -> Lua {
    let lua = unsafe { Lua::unsafe_new() };
    let fonts = Rc::new(RefCell::new(Fonts::new(pob_font_definitions())));
    let textures = Rc::new(RefCell::new(TextureManager::new()));
    let mut ctx = Context::new(script_dir, fonts, textures);
    ctx.recorder.reset();
    lua.set_app_data(ctx);
    lua.set_app_data(SubscriptManager::default());
    register_globals(&lua).unwrap();
    lua
}

/// Number of draw commands recorded on each layer, in layer order.
fn recorded_layers(lua: &Lua) -> Vec<((i32, i32), usize)> {
    let ctx = lua.app_data_ref::<Context>().unwrap();
    let (_, layers) = ctx.recorder.finish();
    layers
        .iter()
        .filter(|(_, cmds)| !cmds.is_empty())
        .map(|(layer, cmds)| (*layer, cmds.len()))
        .collect()
}

#[test]
fn deflate_then_inflate_round_trips() {
    let lua = test_lua(Path::new("."));
    let (zlib, gzip, gzip_magic): (bool, bool, bool) = lua
        .load(
            r#"
            local text = string.rep("Path of Building ", 100) .. "\0binary\255"
            local gzipped = Deflate(text, true)
            return Inflate(Deflate(text)) == text, Inflate(gzipped) == text,
                   gzipped:sub(1, 2) == "\31\139"
            "#,
        )
        .eval()
        .unwrap();
    assert!(zlib, "zlib round trip");
    assert!(gzip, "gzip round trip");
    assert!(gzip_magic, "Deflate(text, true) produces gzip");
}

#[test]
fn inflate_of_garbage_returns_nil_and_message() {
    let lua = test_lua(Path::new("."));
    let (result, message): (Value, String) = lua
        .load(r#"return Inflate("definitely not zlib")"#)
        .eval()
        .unwrap();
    assert!(result.is_nil());
    assert!(!message.is_empty());
}

#[test]
fn script_and_user_paths_come_from_script_dir() {
    let lua = test_lua(Path::new("/opt/pob"));
    let (script, user): (String, String) = lua
        .load("return GetScriptPath(), GetUserPath()")
        .eval()
        .unwrap();
    assert_eq!(script, "/opt/pob");
    assert_eq!(user, "/opt/pob/userdata");
}

#[test]
fn make_and_remove_dir_return_true_or_nil_and_message() {
    let dir = std::env::temp_dir().join(format!("auto-pob-test-{}", std::process::id()));
    let dir = dir.to_str().unwrap();
    let lua = test_lua(Path::new("."));
    lua.globals().set("dir", dir).unwrap();

    let made: bool = lua.load(r#"return MakeDir(dir .. "/a/b")"#).eval().unwrap();
    assert!(made);

    // non-recursive removal of a non-empty directory fails
    let (removed, message): (Value, String) = lua.load("return RemoveDir(dir)").eval().unwrap();
    assert!(removed.is_nil());
    assert!(!message.is_empty());

    let removed: bool = lua.load("return RemoveDir(dir, true)").eval().unwrap();
    assert!(removed);
    assert!(!Path::new(dir).exists());
}

#[test]
fn draw_string_records_one_command_per_glyph_on_current_layer() {
    let lua = test_lua(Path::new("."));
    lua.load(
        r#"
        SetDrawLayer(3, 7)
        DrawString(0, 0, "LEFT", 16, "VAR", "abc")
        "#,
    )
    .exec()
    .unwrap();
    assert_eq!(recorded_layers(&lua), vec![((3, 7), 3)]);
}

#[test]
fn color_escape_in_draw_string_changes_draw_color() {
    let lua = test_lua(Path::new("."));
    let (r, g, b): (f32, f32, f32) = lua
        .load(
            r#"
            SetDrawColor(1, 1, 1)
            DrawString(0, 0, "LEFT", 16, "VAR", "plain ^1red")
            local r, g, b = GetDrawColor()
            return r, g, b
            "#,
        )
        .eval()
        .unwrap();
    let red: [f32; 4] = Srgba::from_escape_code("^1").into();
    assert_eq!([r, g, b], [red[0], red[1], red[2]]);
}

#[test]
fn draw_string_width_grows_with_text_and_ignores_escapes() {
    let lua = test_lua(Path::new("."));
    let (short, long, escaped): (f64, f64, f64) = lua
        .load(
            r#"
            return DrawStringWidth(16, "VAR", "abc"),
                   DrawStringWidth(16, "VAR", "abcabc"),
                   DrawStringWidth(16, "VAR", "^1abc")
            "#,
        )
        .eval()
        .unwrap();
    assert!(short > 0.0);
    assert!(long > short);
    assert_eq!(escaped, short);
}

#[test]
fn main_object_callbacks_receive_self_and_custom_callbacks_shadow_them() {
    let lua = test_lua(Path::new("."));
    lua.load(
        r#"
        local main = { name = "main" }
        function main:OnFrame() return self.name end
        SetMainObject(main)
        "#,
    )
    .exec()
    .unwrap();
    let from_main: String = crate::pob::api::call_callback(&lua, "OnFrame", ()).unwrap();
    assert_eq!(from_main, "main");

    lua.load(r#"SetCallback("OnFrame", function() return "custom" end)"#)
        .exec()
        .unwrap();
    let from_custom: String = crate::pob::api::call_callback(&lua, "OnFrame", ()).unwrap();
    assert_eq!(from_custom, "custom");
}

#[test]
fn errors_in_callbacks_reach_rust() {
    let lua = test_lua(Path::new("."));
    lua.load(
        r#"
        local main = {}
        function main:OnFrame() error("boom") end
        SetMainObject(main)
        "#,
    )
    .exec()
    .unwrap();
    let err = crate::pob::api::on_frame(&lua).unwrap_err();
    assert!(err.to_string().contains("boom"), "{err}");
}

#[test]
fn missing_callback_is_an_error() {
    let lua = test_lua(Path::new("."));
    lua.load("SetMainObject({})").exec().unwrap();
    let err = crate::pob::api::on_frame(&lua).unwrap_err();
    assert!(err.to_string().contains("OnFrame"), "{err}");
}
