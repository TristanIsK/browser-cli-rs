use std::{
    collections::VecDeque,
    fs,
    path::Path,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tungstenite::Message;

use crate::{Error, Result};

mod proxy;

pub struct Cdp {
    socket: proxy::Socket,
    next_id: u64,
    target_session_id: String,
    events: VecDeque<Value>,
}

pub struct WaitTextOptions<'a> {
    pub text: &'a str,
    pub selector: Option<&'a str>,
    pub state: &'a str,
    pub exact: bool,
    pub case_sensitive: bool,
    pub include_hidden: bool,
    pub timeout: Duration,
    pub poll: Duration,
}

impl Cdp {
    /// Connect to the first page, creating a blank page if necessary.
    ///
    /// This preserves the original single-page default. Use `connect_to_target`
    /// to address a particular tab without depending on target enumeration order.
    pub fn connect(url: &str) -> Result<Self> {
        Self::connect_with_target(url, None)
    }

    /// Connect to an existing page in this browser session.
    ///
    /// Missing, closed, or non-page targets return an error; they never fall back
    /// to another page or cause a blank page to be created.
    pub fn connect_to_target(url: &str, target_id: &str) -> Result<Self> {
        if target_id.trim().is_empty() {
            return Err(Error::Config("target ID must not be empty".into()));
        }
        Self::connect_with_target(url, Some(target_id))
    }

    fn connect_with_target(url: &str, requested_target: Option<&str>) -> Result<Self> {
        let socket = proxy::connect(url)?;
        let mut client = Self {
            socket,
            next_id: 1,
            target_session_id: String::new(),
            events: VecDeque::new(),
        };
        let targets = client.command_root("Target.getTargets", json!({}))?;
        let target_id = select_page_target(&targets, requested_target)?;
        let target_id = match target_id {
            Some(id) => id,
            None => client
                .command_root("Target.createTarget", json!({"url":"about:blank"}))?
                .get("targetId")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Cdp("Target.createTarget response missing targetId".into()))?
                .to_owned(),
        };
        let attached = client.command_root(
            "Target.attachToTarget",
            json!({"targetId": target_id, "flatten": true}),
        )?;
        client.target_session_id = attached
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Cdp("Target.attachToTarget response missing sessionId".into()))?
            .to_owned();
        client.command("Page.enable", json!({}))?;
        client.command("Runtime.enable", json!({}))?;
        Ok(client)
    }

    pub fn command(&mut self, method: &str, params: Value) -> Result<Value> {
        let session_id = self.target_session_id.clone();
        self.send(method, params, Some(&session_id))
    }

    pub fn command_root(&mut self, method: &str, params: Value) -> Result<Value> {
        self.send(method, params, None)
    }

    fn send(&mut self, method: &str, params: Value, session_id: Option<&str>) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let mut request = json!({"id": id, "method": method, "params": params});
        if let Some(session_id) = session_id {
            request["sessionId"] = Value::String(session_id.into());
        }
        self.socket
            .send(Message::Text(request.to_string().into()))?;
        loop {
            let raw = self.socket.read()?;
            if !raw.is_text() {
                continue;
            }
            let message: Value = serde_json::from_str(raw.to_text()?)?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(Error::Cdp(
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown CDP error")
                            .into(),
                    ));
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
            if message.get("method").is_some() {
                self.events.push_back(message);
            }
        }
    }

    pub fn navigate(&mut self, url: &str, timeout: Duration) -> Result<Value> {
        let result = self.command("Page.navigate", json!({"url": url}))?;
        let started = Instant::now();
        loop {
            let state = self.evaluate("document.readyState")?;
            if state
                .as_str()
                .is_some_and(|v| v == "interactive" || v == "complete")
            {
                break;
            }
            if started.elapsed() >= timeout {
                return Err(Error::Timeout(format!("navigation to {url}")));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(
            json!({"navigation": result, "url": self.evaluate("location.href")?, "title": self.evaluate("document.title")?}),
        )
    }

    pub fn evaluate(&mut self, expression: &str) -> Result<Value> {
        let result = self.command(
            "Runtime.evaluate",
            json!({"expression": expression, "awaitPromise": true, "returnByValue": true}),
        )?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(Error::Cdp(
                exception
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("JavaScript evaluation failed")
                    .into(),
            ));
        }
        Ok(result
            .get("result")
            .and_then(|v| v.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub fn click(&mut self, selector: &str) -> Result<Value> {
        self.evaluate(&format!("(()=>{{const e=document.querySelector({});if(!e)throw new Error('selector not found');e.scrollIntoView({{block:'center'}});e.click();return true}})()", serde_json::to_string(selector)?))
    }

    pub fn fill(&mut self, selector: &str, value: &str) -> Result<Value> {
        self.evaluate(&format!("(()=>{{const e=document.querySelector({});if(!e)throw new Error('selector not found');const s=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value')?.set;s?s.call(e,{}):e.value={};e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return true}})()", serde_json::to_string(selector)?, serde_json::to_string(value)?, serde_json::to_string(value)?))
    }

    pub fn wait_selector(&mut self, selector: &str, timeout: Duration) -> Result<Value> {
        let started = Instant::now();
        loop {
            let found = self.evaluate(&format!(
                "Boolean(document.querySelector({}))",
                serde_json::to_string(selector)?
            ))?;
            if found == Value::Bool(true) {
                return Ok(json!({"found": true, "selector": selector}));
            }
            if started.elapsed() >= timeout {
                return Err(Error::Timeout(format!("selector {selector}")));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn wait_text(&mut self, options: WaitTextOptions<'_>) -> Result<Value> {
        let started = Instant::now();
        loop {
            let selector = options.selector.unwrap_or("body");
            let candidates = self.evaluate(&format!(
                "(()=>{{const visible=e=>{}||(!!(e.offsetWidth||e.offsetHeight||e.getClientRects().length)&&getComputedStyle(e).visibility!=='hidden');return [...document.querySelectorAll({})].filter(visible).map(e=>({{tag:e.tagName.toLowerCase(),id:e.id||null,text:e.innerText||e.textContent||'',name:e.getAttribute('aria-label')||e.getAttribute('title')||e.getAttribute('alt')||''}}))}})()",
                options.include_hidden,
                serde_json::to_string(selector)?
            ))?;
            let items = candidates.as_array().cloned().unwrap_or_default();
            let element = items
                .iter()
                .find(|item| {
                    ["text", "name"].iter().any(|field| {
                        item.get(field)
                            .and_then(Value::as_str)
                            .is_some_and(|candidate| {
                                text_matches(
                                    candidate,
                                    options.text,
                                    options.exact,
                                    options.case_sensitive,
                                )
                            })
                    })
                })
                .cloned();
            let matched = element.is_some();
            let reached = if options.state == "absent" {
                !matched
            } else {
                matched
            };
            let waited_ms = started.elapsed().as_millis();
            if reached || started.elapsed() >= options.timeout {
                return Ok(json!({
                    "found": matched,
                    "matched": matched,
                    "state": options.state,
                    "text": options.text,
                    "selector": options.selector,
                    "exact": options.exact,
                    "case_sensitive": options.case_sensitive,
                    "include_hidden": options.include_hidden,
                    "waited_ms": waited_ms,
                    "timeout_ms": options.timeout.as_millis(),
                    "poll_ms": options.poll.as_millis(),
                    "candidate_count": items.len(),
                    "element": element,
                    "timed_out": !reached,
                }));
            }
            std::thread::sleep(options.poll.max(Duration::from_millis(25)));
        }
    }

    pub fn screenshot(&mut self, path: &Path, full_page: bool) -> Result<Value> {
        let params = if full_page {
            let metrics = self.command("Page.getLayoutMetrics", json!({}))?;
            let size = metrics.get("cssContentSize").cloned().unwrap_or(json!({}));
            json!({"format":"png","captureBeyondViewport":true,"clip":{"x":0,"y":0,"width":size.get("width").and_then(Value::as_f64).unwrap_or(1280.0),"height":size.get("height").and_then(Value::as_f64).unwrap_or(720.0),"scale":1}})
        } else {
            json!({"format":"png"})
        };
        let result = self.command("Page.captureScreenshot", params)?;
        let data = result
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Cdp("screenshot response missing data".into()))?;
        let bytes = STANDARD
            .decode(data)
            .map_err(|e| Error::Cdp(format!("invalid screenshot data: {e}")))?;
        fs::write(path, &bytes)?;
        Ok(json!({"path": path, "bytes": bytes.len(), "format": "png"}))
    }

    pub fn pdf(&mut self, path: &Path, print_background: bool) -> Result<Value> {
        let result = self.command(
            "Page.printToPDF",
            json!({"printBackground": print_background, "preferCSSPageSize": true}),
        )?;
        let data = result
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Cdp("PDF response missing data".into()))?;
        let bytes = STANDARD
            .decode(data)
            .map_err(|e| Error::Cdp(format!("invalid PDF data: {e}")))?;
        fs::write(path, &bytes)?;
        Ok(json!({"path": path, "bytes": bytes.len(), "format": "pdf"}))
    }

    pub fn snapshot(&mut self) -> Result<Value> {
        self.evaluate("(()=>({url:location.href,title:document.title,text:document.body?.innerText||'',html:document.documentElement?.outerHTML||'',interactive:[...document.querySelectorAll('a,button,input,select,textarea,[role=button]')].slice(0,200).map((e,i)=>({index:i,tag:e.tagName.toLowerCase(),text:(e.innerText||e.value||e.getAttribute('aria-label')||'').trim().slice(0,240),id:e.id||null,name:e.getAttribute('name'),role:e.getAttribute('role')}))}))()")
    }
}

fn select_page_target(targets: &Value, requested: Option<&str>) -> Result<Option<String>> {
    let pages = targets.get("targetInfos").and_then(Value::as_array);
    if let Some(id) = requested {
        let pages = pages
            .ok_or_else(|| Error::Cdp("Target.getTargets response missing targetInfos".into()))?;
        let target = pages
            .iter()
            .find(|item| item["targetId"].as_str() == Some(id));
        match target {
            Some(item) if item["type"] == "page" => Ok(Some(id.to_owned())),
            Some(_) => Err(Error::Config(format!(
                "target {id} is not a page; inspect `session targets` and select a page target"
            ))),
            None => Err(Error::NotFound(format!(
                "page target {id} is not available in this session; inspect `session targets`"
            ))),
        }
    } else {
        Ok(pages
            .and_then(|items| items.iter().find(|item| item["type"] == "page"))
            .and_then(|item| item["targetId"].as_str())
            .map(str::to_owned))
    }
}

fn text_matches(candidate: &str, query: &str, exact: bool, case_sensitive: bool) -> bool {
    let normalize = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut haystack = normalize(candidate);
    let mut needle = normalize(query);
    if !case_sensitive {
        haystack = haystack.to_lowercase();
        needle = needle.to_lowercase();
    }
    if exact {
        haystack == needle
    } else {
        haystack.contains(&needle)
    }
}

#[cfg(test)]
mod tests {
    use super::{select_page_target, text_matches};
    use crate::Error;
    use serde_json::json;

    #[test]
    fn explicit_page_selection_does_not_depend_on_order() {
        let home = json!({"type":"page","targetId":"home"});
        let result = json!({"type":"page","targetId":"result"});
        for items in [vec![home.clone(), result.clone()], vec![result, home]] {
            let targets = json!({"targetInfos":items});
            assert_eq!(
                select_page_target(&targets, Some("result")).unwrap(),
                Some("result".into())
            );
        }
    }

    #[test]
    fn missing_or_non_page_target_never_falls_back() {
        let targets = json!({"targetInfos":[
            {"type":"service_worker","targetId":"worker"},
            {"type":"page","targetId":"home"}
        ]});
        assert!(matches!(
            select_page_target(&targets, Some("closed")),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            select_page_target(&targets, Some("worker")),
            Err(Error::Config(_))
        ));
        assert!(matches!(
            select_page_target(&json!({"targetInfos":[]}), Some("missing")),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn unspecified_target_preserves_first_page_and_empty_browser_defaults() {
        let targets = json!({"targetInfos":[
            {"type":"service_worker","targetId":"worker"},
            {"type":"page","targetId":"home"},
            {"type":"page","targetId":"result"}
        ]});
        assert_eq!(
            select_page_target(&targets, None).unwrap(),
            Some("home".into())
        );
        assert_eq!(
            select_page_target(&json!({"targetInfos":[]}), None).unwrap(),
            None
        );
    }

    #[test]
    fn malformed_explicit_target_listing_is_rejected() {
        assert!(matches!(
            select_page_target(&json!({}), Some("page")),
            Err(Error::Cdp(_))
        ));
        assert!(matches!(
            select_page_target(&json!({"targetInfos":null}), Some("page")),
            Err(Error::Cdp(_))
        ));
    }

    #[test]
    fn empty_explicit_target_is_rejected_before_connecting() {
        for id in ["", "   "] {
            assert!(matches!(
                super::Cdp::connect_to_target("not-a-websocket", id),
                Err(Error::Config(_))
            ));
        }
    }

    #[test]
    fn wait_text_defaults_to_case_insensitive_contains() {
        assert!(text_matches(
            "  Saved   successfully ",
            "saved",
            false,
            false
        ));
        assert!(!text_matches("Saved successfully", "saved", true, false));
        assert!(text_matches("  Saved  ", "saved", true, false));
        assert!(!text_matches("Saved", "saved", true, true));
    }
}
