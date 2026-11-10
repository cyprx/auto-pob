//! Rust port of SimpleGraphic's `lzip`, a library for reading zip files.
//! <https://github.com/PathOfBuildingCommunity/PathOfBuilding-SimpleGraphic/blob/master/libs/LZip/lzip.cpp>

use std::{fs::File, io::Read};

use mlua::{Integer as LuaInteger, Lua, Result as LuaResult, Table, UserData, UserDataMethods};

use crate::pob::modules::luazip::archive::{ZipArchive, ZipFile};

mod archive;

pub fn create_module(lua: &Lua) -> LuaResult<Table> {
    let exports = lua.create_table()?;

    exports.set(
        "open",
        lua.create_function(|_, path: String| {
            let file = File::open(&path).map_err(mlua::Error::external)?;
            let archive = ZipArchive::new(file).map_err(mlua::Error::external)?;
            Ok(ZipArchiveHandle {
                archive: Some(archive),
            })
        })?,
    )?;

    Ok(exports)
}

fn to_index(i: LuaInteger, len: usize) -> Option<usize> {
    usize::try_from(i.checked_sub(1)?).ok().filter(|&i| i < len)
}

struct ZipArchiveHandle {
    archive: Option<ZipArchive<File>>,
}

impl ZipArchiveHandle {
    fn get(&self) -> LuaResult<&ZipArchive<File>> {
        self.archive
            .as_ref()
            .ok_or_else(|| mlua::Error::runtime("zip handle is closed"))
    }
}

impl UserData for ZipArchiveHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("Close", |_, this, ()| {
            this.archive = None;
            Ok(())
        });

        methods.add_method("GetNumFiles", |_, this, ()| Ok(this.get()?.entries().len()));

        methods.add_method("GetFileName", |lua, this, index: LuaInteger| {
            let entries = this.get()?.entries();
            let len = entries.len();
            match to_index(index, len) {
                Some(i) => Ok(Some(lua.create_string(entries[i].name())?)),
                None => Ok(None),
            }
        });

        methods.add_method("GetFileSize", |_, this, index: LuaInteger| {
            let entries = this.get()?.entries();
            let len = entries.len();
            match to_index(index, len) {
                Some(i) => Ok(Some(entries[i].uncompressed_size())),
                None => Ok(None),
            }
        });

        methods.add_method_mut("OpenFile", |_, this, index_or_name: mlua::Value| {
            let archive = this.get()?;
            let entries = archive.entries();

            let index = match index_or_name {
                mlua::Value::Integer(i) => {
                    let Some(index) = to_index(i, entries.len()) else {
                        return Err(mlua::Error::runtime("zip:OpenFile(): invalid index"));
                    };
                    index
                }
                mlua::Value::String(s) => {
                    let name = s.as_bytes();
                    let Some(index) = entries.iter().position(|e| e.name()[..] == *name) else {
                        return Ok(None);
                    };
                    index
                }
                _ => {
                    return Err(mlua::Error::runtime(
                        "Usage: zip:OpenFile(index) or zip:OpenFile(fileName)",
                    ));
                }
            };

            let file = archive.by_index(index).map_err(mlua::Error::external)?;
            Ok(Some(ZipFileHandle { file: Some(file) }))
        });
    }
}

struct ZipFileHandle {
    file: Option<ZipFile<File>>,
}

impl ZipFileHandle {
    fn get(&mut self) -> LuaResult<&ZipFile<File>> {
        self.file
            .as_ref()
            .ok_or_else(|| mlua::Error::runtime("zip file handle is closed"))
    }

    fn get_mut(&mut self) -> LuaResult<&mut ZipFile<File>> {
        self.file
            .as_mut()
            .ok_or_else(|| mlua::Error::runtime("zip file handle is closed"))
    }
}

impl UserData for ZipFileHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("Close", |_, this, ()| {
            this.file = None;
            Ok(())
        });

        methods.add_method_mut("Length", |_, this, ()| Ok(this.get()?.size()));

        methods.add_method_mut("Read", |lua, this, arg: mlua::Value| {
            let file = this.get_mut()?;
            let mut buf = Vec::new();

            match arg {
                mlua::Value::String(s) if s.as_bytes() == b"*a" => {
                    file.read_to_end(&mut buf).map_err(mlua::Error::external)?;
                }
                mlua::Value::Integer(n) if n >= 0 => {
                    if n == 0 {
                        return lua.create_string("");
                    }

                    file.take(n as u64)
                        .read_to_end(&mut buf)
                        .map_err(mlua::Error::external)?;
                }
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "zipFile:Read(): unrecognised format: {}",
                        other.to_string()?
                    )));
                }
            }

            lua.create_string(buf)
        });
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Cursor, Write},
        path::PathBuf,
    };

    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;

    /// Deletes the temp zip when dropped, even if the test panics.
    struct TempZip(PathBuf);

    impl Drop for TempZip {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// Writes a zip with `files` to a temp file and returns a Lua state with `lzip` and `path` set.
    fn lua_with_zip(test: &str, files: &[(&str, &str)]) -> (Lua, TempZip) {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in files {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(data.as_bytes()).unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();

        let path =
            std::env::temp_dir().join(format!("auto-pob-lzip-{test}-{}.zip", std::process::id()));
        std::fs::write(&path, bytes).unwrap();

        let lua = Lua::new();
        lua.globals()
            .set("lzip", create_module(&lua).unwrap())
            .unwrap();
        lua.globals().set("path", path.to_str().unwrap()).unwrap();
        (lua, TempZip(path))
    }

    #[test]
    fn lists_entries_with_one_based_indices() {
        let (lua, _zip) = lua_with_zip("list", &[("a.txt", "alpha"), ("dir/b.txt", "bravo!")]);
        let result: (usize, String, String, u64, Option<String>, Option<u64>) = lua
            .load(
                r#"
                local zip = lzip.open(path)
                return zip:GetNumFiles(), zip:GetFileName(1), zip:GetFileName(2),
                       zip:GetFileSize(2), zip:GetFileName(3), zip:GetFileSize(0)
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            result,
            (2, "a.txt".into(), "dir/b.txt".into(), 6, None, None)
        );
    }

    #[test]
    fn opens_files_by_index_or_name_and_reads_them() {
        let (lua, _zip) = lua_with_zip("read", &[("a.txt", "alpha"), ("b.txt", "bravo")]);
        let result: (String, String, u64, String, String, bool) = lua
            .load(
                r#"
                local zip = lzip.open(path)
                local by_index = zip:OpenFile(1):Read("*a")
                local file = zip:OpenFile("b.txt")
                local length = file:Length()
                local chunk1, chunk2, empty = file:Read(2), file:Read(10), file:Read(0)
                return by_index, chunk1 .. "|" .. chunk2, length, empty,
                       file:Read("*a"), zip:OpenFile("missing.txt") == nil
                "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            result,
            (
                "alpha".into(),
                "br|avo".into(),
                5,
                "".into(),
                "".into(),
                true
            )
        );
    }

    #[test]
    fn misuse_raises_lua_errors() {
        let (lua, _zip) = lua_with_zip("errors", &[("a.txt", "alpha")]);
        let errors: Vec<String> = lua
            .load(
                r#"
                local function err(f) local ok, e = pcall(f); assert(not ok); return tostring(e) end
                local zip = lzip.open(path)
                local file = zip:OpenFile(1)
                local errors = {
                    err(function() zip:OpenFile(5) end),
                    err(function() zip:OpenFile(true) end),
                    err(function() file:Read("*l") end),
                    err(function() lzip.open(path .. ".does-not-exist") end),
                }
                file:Close()
                zip:Close()
                errors[#errors + 1] = err(function() file:Read("*a") end)
                errors[#errors + 1] = err(function() zip:GetNumFiles() end)
                return errors
                "#,
            )
            .eval()
            .unwrap();
        let expected = [
            "invalid index",
            "Usage: zip:OpenFile",
            "unrecognised format",
            "No such file",
            "zip file handle is closed",
            "zip handle is closed",
        ];
        assert_eq!(errors.len(), expected.len());
        for (error, expected) in errors.iter().zip(expected) {
            assert!(
                error.contains(expected),
                "{error:?} should contain {expected:?}"
            );
        }
    }
}
