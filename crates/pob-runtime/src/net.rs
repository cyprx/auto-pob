//! Network configuration shared by the subscript runtime and its Lua modules: timeouts, abort
//! flags and the canned transport that tests use instead of the network.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// Called from subscript threads to wake whoever services the [`crate::subscript::SubscriptManager`].
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// How requests leave the process.
#[derive(Clone, Default)]
pub enum Transport {
    /// Real requests through libcurl.
    #[default]
    Curl,
    /// Replies served from a [`CannedWeb`]; nothing reaches the network.
    Canned(Arc<CannedWeb>),
}

/// Timeouts and transport of a subscript's network calls.
#[derive(Clone, Default)]
pub struct NetConfig {
    pub connect_timeout: Option<Duration>,
    pub timeout: Option<Duration>,
    pub transport: Transport,
}

impl NetConfig {
    /// The egui app's settings: 15 s to connect, 60 s in total, real curl.
    pub fn app() -> NetConfig {
        NetConfig {
            connect_timeout: Some(Duration::from_secs(15)),
            timeout: Some(Duration::from_secs(60)),
            transport: Transport::Curl,
        }
    }
}

/// Lua app data of a subscript state: set by [`crate::subscript::SubscriptManager::abort`].
#[derive(Clone, Default)]
pub struct CancelFlag(pub Arc<AtomicBool>);

impl CancelFlag {
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// One canned reply.
#[derive(Clone, Debug, Default)]
pub struct Canned {
    pub status: u32,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub delay: Duration,
    /// A curl error message, such as `Couldn't resolve host name`; the request then fails.
    pub fail: Option<String>,
}

impl Canned {
    pub fn ok(body: impl Into<Vec<u8>>) -> Canned {
        Canned::status(200, body)
    }

    pub fn status(code: u32, body: impl Into<Vec<u8>>) -> Canned {
        Canned {
            status: code,
            body: body.into(),
            ..Canned::default()
        }
    }

    pub fn fail(curl_message: &str) -> Canned {
        Canned {
            fail: Some(curl_message.to_string()),
            ..Canned::default()
        }
    }

    pub fn delayed(mut self, d: Duration) -> Canned {
        self.delay = d;
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Canned {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

/// A request as the canned transport saw it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recorded {
    pub method: String,
    pub url: String,
    pub headers: Vec<String>,
    pub body: Vec<u8>,
    pub proxy: Option<String>,
    pub ip_resolve: Option<i64>,
}

struct Route {
    method: String,
    url: String,
    body_prefix: Option<String>,
    reply: Canned,
}

/// A table of canned replies plus the log of what was asked.
#[derive(Default)]
pub struct CannedWeb {
    routes: Mutex<Vec<Route>>,
    log: Mutex<Vec<Recorded>>,
    unrouted: Mutex<Vec<String>>,
}

impl CannedWeb {
    /// Answers requests with exactly this method and URL.
    pub fn route(&self, method: &str, url: &str, reply: Canned) {
        self.routes.lock().unwrap().push(Route {
            method: method.to_string(),
            url: url.to_string(),
            body_prefix: None,
            reply,
        });
    }

    /// Answers POSTs to `url` whose body starts with `body_prefix`; the longest prefix wins.
    pub fn route_post(&self, url: &str, body_prefix: &str, reply: Canned) {
        self.routes.lock().unwrap().push(Route {
            method: "POST".to_string(),
            url: url.to_string(),
            body_prefix: Some(body_prefix.to_string()),
            reply,
        });
    }

    /// Every request made so far, in order, routed or not.
    pub fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }

    /// `"<METHOD> <url>"` of every request that had no route.
    pub fn unrouted(&self) -> Vec<String> {
        self.unrouted.lock().unwrap().clone()
    }

    /// Records `request` and finds its reply.
    pub(crate) fn serve(&self, request: Recorded) -> Option<Canned> {
        let reply = {
            let routes = self.routes.lock().unwrap();
            routes
                .iter()
                .filter(|r| r.method == request.method && r.url == request.url)
                .filter(|r| match &r.body_prefix {
                    Some(prefix) => request.body.starts_with(prefix.as_bytes()),
                    None => true,
                })
                .max_by_key(|r| r.body_prefix.as_ref().map_or(0, |p| p.len() + 1))
                .map(|r| r.reply.clone())
        };
        if reply.is_none() {
            self.unrouted
                .lock()
                .unwrap()
                .push(format!("{} {}", request.method, request.url));
        }
        self.log.lock().unwrap().push(request);
        reply
    }
}
