use curl::easy::{Easy, IpResolve};
use mlua::{
    Error as LuaError, IntoLua, Lua, LuaString, MultiValue, RegistryKey, Result as LuaResult,
    UserData, UserDataMethods,
};
use std::time::{Duration, Instant};

use crate::{
    modules::luacurl::constants,
    net::{CancelFlag, Canned, NetConfig, Recorded, Transport},
};

/// The error object `perform` returns, like Lua-cURL's: `err:msg()` is curl's text.
struct CurlError {
    msg: String,
    code: i64,
}

impl UserData for CurlError {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("msg", |_, this, ()| Ok(this.msg.clone()));
        methods.add_method("no", |_, this, ()| Ok(this.code));
        methods.add_meta_method("__tostring", |_, this, ()| Ok(this.msg.clone()));
    }
}

/// curl's code for `CURLE_OPERATION_TIMEDOUT`.
const CURLE_OPERATION_TIMEDOUT: i64 = 28;
/// curl's code for `CURLE_COULDNT_CONNECT`, standing in for canned failures.
const CURLE_COULDNT_CONNECT: i64 = 7;
const CURLE_ABORTED_BY_CALLBACK: i64 = 42;

/// What `getinfo` answers after a canned `perform`.
#[derive(Default)]
struct CannedInfo {
    status: u32,
    redirect_url: Option<String>,
    size: f64,
}

/// How often a canned delay checks the cancel flag.
const CANNED_SLICE: Duration = Duration::from_millis(20);

/// Helper for constructing a `mlua::Error::BadArgument`
fn bad_argument(to: impl Into<String>, pos: usize, cause: &'static str) -> LuaError {
    #[allow(clippy::arc_with_non_send_sync)]
    LuaError::BadArgument {
        to: Some(to.into()),
        pos: pos + 1,
        name: None,
        cause: std::sync::Arc::new(LuaError::runtime(cause)),
    }
}

/// A Lua callback set via `setopt_*function`.
///
/// `context` is set when an object is passed to the callback setter. In that case `function` is
/// the object's method and is invoked as `function(context, data)`.
struct Callback {
    function: RegistryKey,
    context: Option<RegistryKey>,
}

impl Callback {
    fn resolve(&self, lua: &Lua) -> LuaResult<(mlua::Function, Option<mlua::Value>)> {
        let function = lua.registry_value(&self.function)?;
        let context = self
            .context
            .as_ref()
            .map(|key| lua.registry_value::<mlua::Value>(key))
            .transpose()?;

        Ok((function, context))
    }

    fn remove(self, lua: &Lua) -> LuaResult<()> {
        lua.remove_registry_value(self.function)?;

        if let Some(key) = self.context {
            lua.remove_registry_value(key)?;
        }

        Ok(())
    }
}

/// Interprets write/header callback return values like Lua-cURL does.
fn callback_size(data_len: usize, results: &mlua::MultiValue) -> usize {
    let Some(value) = results.front() else {
        return data_len;
    };

    match value {
        mlua::Value::Nil => 0,
        mlua::Value::Integer(n) => (*n).max(0) as usize,
        mlua::Value::Number(n) if *n >= 0.0 => *n as usize,
        mlua::Value::Number(_) => 0,
        mlua::Value::Boolean(false) => 0,
        _ => data_len,
    }
}

type Resolved = Option<(mlua::Function, Option<mlua::Value>)>;

/// Calls a write or header callback with `data`.
fn call_callback(
    function: &mlua::Function,
    context: &Option<mlua::Value>,
    argument: LuaString,
) -> LuaResult<MultiValue> {
    match context {
        Some(context) => function.call::<MultiValue>((context.clone(), argument)),
        None => function.call::<MultiValue>(argument),
    }
}

#[derive(Default)]
pub struct EasyHandle {
    easy: Option<Easy>,
    write_callback: Option<Callback>,
    header_callback: Option<Callback>,
    // what the canned transport records
    url: Option<String>,
    post: bool,
    post_fields: Vec<u8>,
    headers: Vec<String>,
    proxy: Option<String>,
    ip_resolve: Option<i64>,
    canned: Option<CannedInfo>,
}

impl EasyHandle {
    pub fn new() -> Self {
        Self {
            easy: Some(Easy::new()),
            ..Self::default()
        }
    }

    fn curl(&mut self) -> LuaResult<&mut Easy> {
        self.easy
            .as_mut()
            .ok_or_else(|| LuaError::runtime("curl handle is closed"))
    }

    pub fn close(&mut self, lua: &Lua) -> LuaResult<()> {
        if let Some(callback) = self.write_callback.take() {
            callback.remove(lua)?;
        }

        if let Some(callback) = self.header_callback.take() {
            callback.remove(lua)?;
        }

        self.easy = None;
        Ok(())
    }

    // TODO: handling of `value` might not be 100% correct w.r.t. casting and mapping
    // of number values to arguments of `easy` methods. extract to helper functions.
    pub fn set_option(&mut self, option: i64, value: mlua::Value) -> LuaResult<()> {
        let easy = self
            .easy
            .as_mut()
            .ok_or_else(|| LuaError::runtime("curl handle is closed"))?;

        match option {
            constants::OPT_ACCEPT_ENCODING => {
                let encoding = match value {
                    mlua::Value::String(s) => s.to_str(),
                    _ => Err(bad_argument("setopt", 1, "string expected")),
                }?;
                easy.accept_encoding(&encoding)
                    .map_err(LuaError::external)?;
            }
            constants::OPT_FOLLOWLOCATION => {
                let enable = match value {
                    mlua::Value::Boolean(b) => Ok(b),
                    mlua::Value::Integer(n) => Ok(n != 0),
                    mlua::Value::Number(n) => Ok(n != 0.0),
                    _ => Err(bad_argument("setopt", 1, "number or boolean expected")),
                }?;
                easy.follow_location(enable).map_err(LuaError::external)?;
            }
            constants::OPT_HTTPHEADER => {
                let headers = match value {
                    mlua::Value::Table(t) => Ok(t),
                    _ => Err(bad_argument("setopt", 1, "array expected")),
                }?;

                let mut list = curl::easy::List::new();
                let mut recorded = Vec::new();
                for header in headers.sequence_values::<String>() {
                    let header = header?;
                    list.append(&header).map_err(LuaError::external)?;
                    recorded.push(header);
                }
                self.headers = recorded;

                easy.http_headers(list).map_err(LuaError::external)?;
            }
            constants::OPT_IPRESOLVE => {
                let resolve = match value {
                    mlua::Value::Integer(0) => Ok(IpResolve::Any),
                    mlua::Value::Integer(1) => Ok(IpResolve::V4),
                    mlua::Value::Integer(2) => Ok(IpResolve::V6),
                    _ => Err(bad_argument("setopt", 1, "number or boolean expected")),
                }?;
                easy.ip_resolve(resolve).map_err(LuaError::external)?;
                self.ip_resolve = match value {
                    mlua::Value::Integer(n) => Some(n),
                    _ => None,
                };
            }
            constants::OPT_POST => {
                let enable = match value {
                    mlua::Value::Boolean(b) => Ok(b),
                    mlua::Value::Integer(n) => Ok(n != 0),
                    mlua::Value::Number(n) => Ok(n != 0.0),
                    _ => Err(bad_argument("setopt", 1, "number or boolean expected")),
                }?;
                easy.post(enable).map_err(LuaError::external)?;
                self.post = enable;
            }
            constants::OPT_POSTFIELDS => {
                let data = match value {
                    mlua::Value::String(s) => Ok(s.as_bytes()),
                    _ => Err(bad_argument("setopt", 1, "string expected")),
                }?;
                easy.post_fields_copy(&data).map_err(LuaError::external)?;
                self.post_fields = data.to_vec();
            }
            constants::OPT_PROXY => {
                let proxy = match value {
                    mlua::Value::String(s) => s.to_str(),
                    _ => Err(bad_argument("setopt", 1, "string expected")),
                }?;
                easy.proxy(&proxy).map_err(LuaError::external)?;
                self.proxy = Some(proxy.to_string());
            }
            constants::OPT_SSL_VERIFYHOST => {
                let verify = match value {
                    mlua::Value::Boolean(b) => Ok(b),
                    mlua::Value::Integer(n) => Ok(n != 0),
                    mlua::Value::Number(n) => Ok(n != 0.0),
                    _ => Err(bad_argument("setopt", 1, "number or boolean expected")),
                }?;
                easy.ssl_verify_host(verify).map_err(LuaError::external)?;
            }
            constants::OPT_SSL_VERIFYPEER => {
                let verify = match value {
                    mlua::Value::Boolean(b) => Ok(b),
                    mlua::Value::Integer(n) => Ok(n != 0),
                    mlua::Value::Number(n) => Ok(n != 0.0),
                    _ => Err(bad_argument("setopt", 1, "number or boolean expected")),
                }?;
                easy.ssl_verify_peer(verify).map_err(LuaError::external)?;
            }
            constants::OPT_USERAGENT => {
                let useragent = match value {
                    mlua::Value::String(s) => s.to_str(),
                    _ => Err(bad_argument("setopt", 1, "string expected")),
                }?;
                easy.useragent(&useragent).map_err(LuaError::external)?;
            }
            constants::OPT_URL => {
                let url = match value {
                    mlua::Value::String(s) => s.to_str(),
                    _ => Err(bad_argument("setopt", 1, "string expected")),
                }?;
                easy.url(&url).map_err(LuaError::external)?;
                self.url = Some(url.to_string());
            }
            _ => unimplemented!("Unsupported `setopt` option: {option}"),
        }

        Ok(())
    }

    fn set_callback(
        lua: &Lua,
        slot: &mut Option<Callback>,
        value: mlua::Value,
        method: &str,
    ) -> LuaResult<()> {
        let new_callback = match value {
            mlua::Value::Nil => None,
            mlua::Value::Function(function) => Some(Callback {
                function: lua.create_registry_value(function)?,
                context: None,
            }),
            // Lua-cURL can also accept objects with a method of the given name,
            // e.g. a file handle returned by `io.open` with a `write` method.
            // looks a bit awkwards but it works...
            value @ (mlua::Value::Table(_) | mlua::Value::UserData(_)) => {
                let function: mlua::Function = lua
                    .load("local object, key = ...; return object[key]")
                    .call((value.clone(), method))?;

                Some(Callback {
                    function: lua.create_registry_value(function)?,
                    context: Some(lua.create_registry_value(value)?),
                })
            }
            _ => return Err(LuaError::runtime("invalid object type")),
        };

        if let Some(old_callback) = slot.take() {
            old_callback.remove(lua)?;
        }

        *slot = new_callback;

        Ok(())
    }

    fn perform(&mut self, lua: &Lua) -> LuaResult<MultiValue> {
        let write_fn = self
            .write_callback
            .as_ref()
            .map(|callback| callback.resolve(lua))
            .transpose()?;

        let header_fn = self
            .header_callback
            .as_ref()
            .map(|callback| callback.resolve(lua))
            .transpose()?;

        let config = lua
            .app_data_ref::<NetConfig>()
            .map(|config| config.clone())
            .unwrap_or_default();
        let cancel = lua
            .app_data_ref::<CancelFlag>()
            .map(|flag| flag.clone())
            .unwrap_or_default();

        let outcome = match &config.transport {
            Transport::Curl => self.perform_curl(lua, &config, cancel, write_fn, header_fn)?,
            Transport::Canned(_) => {
                self.perform_canned(lua, &config, cancel, write_fn, header_fn)?
            }
        };

        match outcome {
            Ok(()) => Ok(MultiValue::from_vec(vec![mlua::Value::Boolean(true)])),
            Err(error) => Ok(MultiValue::from_vec(vec![
                mlua::Value::Nil,
                mlua::Value::UserData(lua.create_userdata(error)?),
            ])),
        }
    }

    fn perform_curl(
        &mut self,
        lua: &Lua,
        config: &NetConfig,
        cancel: CancelFlag,
        write_fn: Resolved,
        header_fn: Resolved,
    ) -> LuaResult<Result<(), CurlError>> {
        let easy = self.curl()?;
        if let Some(connect_timeout) = config.connect_timeout {
            easy.connect_timeout(connect_timeout)
                .map_err(LuaError::external)?;
        }
        if let Some(timeout) = config.timeout {
            easy.timeout(timeout).map_err(LuaError::external)?;
        }
        easy.progress(true).map_err(LuaError::external)?;

        let transfer_result = {
            let mut transfer = easy.transfer();

            // curl calls this at least once a second, whatever the transfer is doing
            transfer
                .progress_function(move |_, _, _, _| !cancel.is_set())
                .map_err(LuaError::external)?;

            if let Some((function, context)) = write_fn {
                transfer
                    .write_function(move |data| {
                        let Ok(argument) = lua.create_string(data) else {
                            return Ok(0);
                        };

                        match call_callback(&function, &context, argument) {
                            Ok(results) => Ok(callback_size(data.len(), &results)),
                            Err(_) => Ok(0),
                        }
                    })
                    .map_err(LuaError::external)?;
            }

            if let Some((function, context)) = header_fn {
                transfer
                    .header_function(move |data| {
                        let Ok(argument) = lua.create_string(data) else {
                            return false;
                        };

                        call_callback(&function, &context, argument)
                            .map(|results| callback_size(data.len(), &results) == data.len())
                            .unwrap_or(false)
                    })
                    .map_err(LuaError::external)?;
            }

            transfer.perform()
        };

        Ok(transfer_result.map_err(|e| CurlError {
            msg: e.description().to_string(),
            code: e.code() as i64,
        }))
    }

    fn perform_canned(
        &mut self,
        lua: &Lua,
        config: &NetConfig,
        cancel: CancelFlag,
        write_fn: Resolved,
        header_fn: Resolved,
    ) -> LuaResult<Result<(), CurlError>> {
        let Transport::Canned(web) = &config.transport else {
            unreachable!("perform_canned runs only for the canned transport");
        };
        self.canned = Some(CannedInfo::default());

        let url = self.url.clone().unwrap_or_default();
        let request = Recorded {
            method: if self.post { "POST" } else { "GET" }.to_string(),
            url: url.clone(),
            headers: self.headers.clone(),
            body: if self.post {
                self.post_fields.clone()
            } else {
                Vec::new()
            },
            proxy: self.proxy.clone(),
            ip_resolve: self.ip_resolve,
        };
        let Some(Canned {
            status,
            headers,
            body,
            delay,
            fail,
        }) = web.serve(request)
        else {
            return Ok(Err(CurlError {
                msg: format!("no canned response for {url}"),
                code: CURLE_COULDNT_CONNECT,
            }));
        };

        // the delay stands for a slow server: it runs into the timeout or the cancel flag
        let times_out_at = config.timeout.filter(|t| delay > *t);
        let start = Instant::now();
        loop {
            let elapsed = start.elapsed();
            if cancel.is_set() {
                return Ok(Err(CurlError {
                    msg: "Callback aborted".into(),
                    code: CURLE_ABORTED_BY_CALLBACK,
                }));
            }
            if times_out_at.is_some_and(|t| elapsed >= t) {
                return Ok(Err(CurlError {
                    msg: "Timeout was reached".into(),
                    code: CURLE_OPERATION_TIMEDOUT,
                }));
            }
            if elapsed >= delay {
                break;
            }
            let until = times_out_at.unwrap_or(delay);
            std::thread::sleep(CANNED_SLICE.min(until.saturating_sub(elapsed)));
        }

        if let Some(message) = fail {
            return Ok(Err(CurlError {
                msg: message,
                code: CURLE_COULDNT_CONNECT,
            }));
        }

        self.canned = Some(CannedInfo {
            status,
            redirect_url: headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                .map(|(_, value)| value.clone()),
            size: body.len() as f64,
        });

        if let Some((function, context)) = header_fn {
            let mut lines = vec![format!("HTTP/1.1 {status}\r\n")];
            lines.extend(headers.iter().map(|(n, v)| format!("{n}: {v}\r\n")));
            lines.push("\r\n".to_string());
            for line in lines {
                let accepted = lua
                    .create_string(&line)
                    .and_then(|argument| call_callback(&function, &context, argument))
                    .map(|results| callback_size(line.len(), &results) == line.len())
                    .unwrap_or(false);
                if !accepted {
                    return Ok(Err(CurlError {
                        msg: "Failed writing header".into(),
                        code: 23,
                    }));
                }
            }
        }

        if let Some((function, context)) = write_fn
            && !body.is_empty()
        {
            let written = lua
                .create_string(&body)
                .and_then(|argument| call_callback(&function, &context, argument))
                .map(|results| callback_size(body.len(), &results))
                .unwrap_or(0);
            if written != body.len() {
                return Ok(Err(CurlError {
                    msg: "Failed writing received data to disk/application".into(),
                    code: 23,
                }));
            }
        }

        Ok(Ok(()))
    }

    fn get_info(&mut self, lua: &Lua, info: i64) -> LuaResult<mlua::Value> {
        if let Some(canned) = &self.canned {
            return match info {
                constants::INFO_REDIRECT_URL => canned.redirect_url.clone().into_lua(lua),
                constants::INFO_RESPONSE_CODE => canned.status.into_lua(lua),
                constants::INFO_SIZE_DOWNLOAD => canned.size.into_lua(lua),
                _ => unimplemented!("Unsupported `getinfo` key: {info}"),
            };
        }
        let easy = self.curl()?;

        match info {
            constants::INFO_REDIRECT_URL => {
                let url = easy.redirect_url().map_err(LuaError::external)?;
                url.into_lua(lua)
            }
            constants::INFO_RESPONSE_CODE => {
                let code = easy.response_code().map_err(LuaError::external)?;
                code.into_lua(lua)
            }
            constants::INFO_SIZE_DOWNLOAD => {
                let size = easy.download_size().map_err(LuaError::external)?;
                size.into_lua(lua)
            }
            _ => unimplemented!("Unsupported `getinfo` key: {info}"),
        }
    }
}

impl UserData for EasyHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("escape", |lua, _, value: LuaString| {
            let input = value.as_bytes();
            let mut easy = Easy::new();
            let escaped = easy.url_encode(input.as_ref());
            lua.create_string(escaped)
        });

        methods.add_method_mut("close", |lua, this, ()| this.close(lua));

        methods.add_method_mut("setopt", |_, this, (option, value): (i64, mlua::Value)| {
            this.set_option(option, value)
        });

        methods.add_method_mut("setopt_url", |_, this, value: mlua::Value| {
            this.set_option(constants::OPT_URL, value)
        });
        methods.add_method_mut("setopt_useragent", |_, this, value: mlua::Value| {
            this.set_option(constants::OPT_USERAGENT, value)
        });

        methods.add_method_mut("setopt_writefunction", |lua, this, value: mlua::Value| {
            Self::set_callback(lua, &mut this.write_callback, value, "write")
        });

        methods.add_method_mut("setopt_headerfunction", |lua, this, value: mlua::Value| {
            Self::set_callback(lua, &mut this.header_callback, value, "header")
        });

        methods.add_method_mut("perform", |lua, this, ()| this.perform(lua));

        methods.add_method_mut("getinfo", |lua, this, info: i64| this.get_info(lua, info));

        methods.add_method_mut("getinfo_response_code", |lua, this, ()| {
            this.get_info(lua, constants::INFO_RESPONSE_CODE)
        });
    }
}
