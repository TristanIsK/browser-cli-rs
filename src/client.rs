use std::{
    env, thread,
    time::{Duration, Instant},
};

use reqwest::{
    Method,
    blocking::{Client as HttpClient, Response},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{
    Error, Result,
    models::{Context, ContextList, CreateSession, Session, SessionList},
};

const DEFAULT_BASE_URL: &str = "https://api.lexmount.cn";

#[derive(Debug, Clone)]
pub struct ClientBuilder {
    api_key: Option<String>,
    project_id: Option<String>,
    base_url: Option<String>,
    region: Option<String>,
    timeout: Duration,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            api_key: None,
            project_id: None,
            base_url: None,
            region: None,
            timeout: Duration::from_secs(60),
        }
    }
}

impl ClientBuilder {
    pub fn api_key(mut self, value: impl Into<String>) -> Self {
        self.api_key = Some(value.into());
        self
    }
    pub fn project_id(mut self, value: impl Into<String>) -> Self {
        self.project_id = Some(value.into());
        self
    }
    pub fn base_url(mut self, value: impl Into<String>) -> Self {
        self.base_url = Some(value.into());
        self
    }
    pub fn region(mut self, value: impl Into<String>) -> Self {
        self.region = Some(value.into());
        self
    }
    pub fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }

    pub fn build(self) -> Result<Client> {
        let credentials = crate::auth::load_credentials(None)
            .ok()
            .flatten()
            .filter(|value| {
                value.kind == "api_key" && !value.api_key.is_empty() && !value.project_id.is_empty()
            });
        let api_key = self
            .api_key
            .or_else(|| env::var("LEXMOUNT_API_KEY").ok())
            .or_else(|| credentials.as_ref().map(|c| c.api_key.clone()))
            .ok_or_else(|| {
                Error::Config("LEXMOUNT_API_KEY is not set; run `browser-cli auth login`".into())
            })?;
        let project_id = self
            .project_id
            .or_else(|| env::var("LEXMOUNT_PROJECT_ID").ok())
            .or_else(|| credentials.as_ref().map(|c| c.project_id.clone()))
            .ok_or_else(|| {
                Error::Config("LEXMOUNT_PROJECT_ID is not set; run `browser-cli auth login`".into())
            })?;
        let base_url = self
            .base_url
            .or_else(|| env::var("LEXMOUNT_BASE_URL").ok())
            .or_else(|| credentials.as_ref().map(|c| c.api_base_url.clone()))
            .unwrap_or_else(|| DEFAULT_BASE_URL.into())
            .trim_end_matches('/')
            .to_owned();
        let region = self.region.or_else(|| env::var("LEXMOUNT_REGION").ok());
        let http = HttpClient::builder().timeout(self.timeout).build()?;
        Ok(Client {
            api_key,
            project_id,
            base_url,
            region,
            http,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Client {
    api_key: String,
    project_id: String,
    base_url: String,
    region: Option<String>,
    http: HttpClient,
}

impl Client {
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }
    pub fn from_env() -> Result<Self> {
        Self::builder().build()
    }
    pub fn project_id(&self) -> &str {
        &self.project_id
    }
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn body_with_region(&self, mut body: Value) -> Value {
        if let (Some(region), Some(object)) = (&self.region, body.as_object_mut()) {
            object.insert("region_id".into(), Value::String(region.clone()));
        }
        body
    }

    fn send(&self, method: Method, path: &str, body: Option<Value>) -> Result<Response> {
        let url = format!("{}{}", self.base_url, path);
        let mut request = self
            .http
            .request(method, url)
            .header("x-api-key", &self.api_key)
            .header("x-project-id", &self.project_id)
            .header("accept", "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }
        Ok(request.send()?)
    }

    fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T> {
        let response = self.send(method, path, body)?;
        let status = response.status();
        let bytes = response.bytes()?;
        if !status.is_success() {
            let body: Option<Value> = serde_json::from_slice(&bytes).ok();
            let message = body
                .as_ref()
                .and_then(|v| v.get("message").or_else(|| v.get("error")))
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string())
                })
                .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
            return Err(match status.as_u16() {
                401 | 403 => Error::Authentication(message),
                404 => Error::NotFound(message),
                409 => Error::Conflict(message),
                code => Error::Api {
                    status: code,
                    message,
                    body,
                },
            });
        }
        if bytes.is_empty() {
            return serde_json::from_value(json!({})).map_err(Error::from);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn bytes(&self, method: Method, path: &str, body: Option<Value>) -> Result<Vec<u8>> {
        let response = self.send(method, path, body)?;
        let status = response.status();
        let bytes = response.bytes()?;
        if !status.is_success() {
            let body: Option<Value> = serde_json::from_slice(&bytes).ok();
            let message = body
                .as_ref()
                .and_then(|v| v.get("message").or_else(|| v.get("error")))
                .and_then(Value::as_str)
                .unwrap_or("download request failed")
                .to_owned();
            return Err(Error::Api {
                status: status.as_u16(),
                message,
                body,
            });
        }
        Ok(bytes.to_vec())
    }

    pub fn create_session(
        &self,
        request: CreateSession,
        poll_timeout: Duration,
    ) -> Result<Session> {
        let body = self.body_with_region(serde_json::to_value(request)?);
        let accepted: Value = self.json(Method::POST, "/instance/v2", Some(body))?;
        let session_id = accepted
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Api {
                status: 202,
                message: "response missing session_id".into(),
                body: Some(accepted.clone()),
            })?;
        let started = Instant::now();
        loop {
            let session = self.get_session(session_id)?;
            match session.status.as_str() {
                "active" => return self.hydrate_ws(session),
                "create_failed" => {
                    return Err(Error::Api {
                        status: 500,
                        message: session
                            .create_error
                            .clone()
                            .unwrap_or_else(|| "session creation failed".into()),
                        body: Some(serde_json::to_value(session)?),
                    });
                }
                "closed" => {
                    return Err(Error::Api {
                        status: 409,
                        message: "session closed while it was being created".into(),
                        body: None,
                    });
                }
                _ if started.elapsed() >= poll_timeout => {
                    return Err(Error::Timeout(format!(
                        "session {session_id} did not become active"
                    )));
                }
                _ => thread::sleep(Duration::from_secs(1)),
            }
        }
    }

    pub fn get_session(&self, session_id: &str) -> Result<Session> {
        let body = self.body_with_region(json!({"session_id": session_id}));
        self.json(Method::POST, "/instance/session", Some(body))
    }

    pub fn get_session_with_ws(&self, session_id: &str) -> Result<Session> {
        let session = self.get_session(session_id)?;
        self.hydrate_ws(session)
    }

    pub fn list_sessions(&self, status: Option<&str>) -> Result<SessionList> {
        let mut body = json!({});
        if let Some(status) = status {
            body["status"] = Value::String(status.into());
        }
        self.json(
            Method::POST,
            "/instance/v2/sessions",
            Some(self.body_with_region(body)),
        )
    }

    pub fn close_session(&self, session_id: &str) -> Result<Value> {
        let body = self.body_with_region(json!({"session_id": session_id}));
        self.json(Method::DELETE, "/instance", Some(body))
    }

    pub fn session_targets(&self, session_id: &str) -> Result<Value> {
        self.json(
            Method::GET,
            &format!(
                "/json?session_id={}",
                url::form_urlencoded::byte_serialize(session_id.as_bytes()).collect::<String>()
            ),
            None,
        )
    }

    pub fn list_downloads(&self, session_id: &str) -> Result<Value> {
        self.json(
            Method::POST,
            &format!("/instance/v1/sessions/{session_id}/downloads/list"),
            Some(json!({})),
        )
    }

    pub fn get_download(&self, session_id: &str, download_id: &str) -> Result<Vec<u8>> {
        self.bytes(
            Method::GET,
            &format!("/instance/v1/sessions/{session_id}/downloads/{download_id}"),
            None,
        )
    }

    pub fn archive_downloads(&self, session_id: &str) -> Result<Vec<u8>> {
        self.bytes(
            Method::GET,
            &format!("/instance/v1/sessions/{session_id}/downloads/archive"),
            None,
        )
    }

    pub fn delete_downloads(&self, session_id: &str) -> Result<Value> {
        self.json(
            Method::DELETE,
            &format!("/instance/v1/sessions/{session_id}/downloads"),
            Some(json!({})),
        )
    }

    pub fn create_context(
        &self,
        metadata: Option<Value>,
        description: Option<&str>,
    ) -> Result<Context> {
        let mut body = json!({"api_key": self.api_key, "project_id": self.project_id});
        if let Some(metadata) = metadata {
            body["metadata"] = metadata;
        }
        if let Some(description) = description {
            body["description"] = Value::String(description.into());
        }
        self.json(
            Method::POST,
            "/instance/v1/contexts/create-context",
            Some(body),
        )
    }

    pub fn list_contexts(&self, status: Option<&str>, limit: u64) -> Result<ContextList> {
        let mut body =
            json!({"api_key": self.api_key, "project_id": self.project_id, "limit": limit});
        if let Some(status) = status {
            body["status"] = Value::String(status.into());
        }
        self.json(
            Method::POST,
            "/instance/v1/contexts/list-contexts",
            Some(body),
        )
    }

    pub fn get_context(&self, context_id: &str) -> Result<Context> {
        #[derive(serde::Deserialize)]
        struct Wrapped {
            context: Context,
        }
        let body = json!({"api_key": self.api_key, "project_id": self.project_id});
        Ok(self
            .json::<Wrapped>(
                Method::POST,
                &format!("/instance/v1/contexts/{context_id}"),
                Some(body),
            )?
            .context)
    }

    pub fn fork_context(&self, context_id: &str) -> Result<Context> {
        let body = json!({"api_key": self.api_key, "project_id": self.project_id});
        self.json(
            Method::POST,
            &format!("/instance/v1/contexts/{context_id}/fork"),
            Some(body),
        )
    }

    pub fn delete_context(&self, context_id: &str) -> Result<Value> {
        let body = json!({"api_key": self.api_key, "project_id": self.project_id});
        self.json(
            Method::DELETE,
            &format!("/instance/v1/contexts/{context_id}"),
            Some(body),
        )
    }

    pub fn force_release_context(&self, context_id: &str) -> Result<Value> {
        let body = json!({"api_key": self.api_key, "project_id": self.project_id});
        self.json(
            Method::POST,
            &format!("/instance/v1/contexts/{context_id}/force-release"),
            Some(body),
        )
    }

    fn hydrate_ws(&self, mut session: Session) -> Result<Session> {
        if session.ws.as_deref().is_some_and(|v| !v.is_empty()) {
            return Ok(session);
        }
        let version: Value = self.json(
            Method::GET,
            &format!("/json/version?session_id={}", session.session_id),
            None,
        )?;
        session.ws = version
            .get("webSocketDebuggerUrlTransformed")
            .or_else(|| version.get("webSocketDebuggerUrl"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{
        Method::{GET, POST},
        MockServer,
    };

    fn client(server: &MockServer) -> Client {
        let mut client = Client::builder()
            .api_key("test-key")
            .project_id("test-project")
            .base_url(server.base_url())
            .region("office-test")
            .build()
            .unwrap();
        client.http = HttpClient::builder().no_proxy().build().unwrap();
        client
    }

    #[test]
    fn async_create_sends_auth_and_polls_until_active() {
        let server = MockServer::start();
        let create = server.mock(|when, then| {
            when.method(POST)
                .path("/instance/v2")
                .header("x-api-key", "test-key")
                .header("x-project-id", "test-project")
                .json_body_partial(r#"{"browser_mode":"normal","region_id":"office-test"}"#);
            then.status(202).json_body(json!({"session_id":"s1"}));
        });
        let get = server.mock(|when, then| {
            when.method(POST)
                .path("/instance/session")
                .json_body_partial(r#"{"session_id":"s1","region_id":"office-test"}"#);
            then.status(200)
                .json_body(json!({"session_id":"s1","status":"active","ws":"ws://example"}));
        });

        let session = client(&server)
            .create_session(CreateSession::default(), Duration::from_secs(1))
            .unwrap();
        assert_eq!(session.session_id, "s1");
        assert_eq!(session.ws.as_deref(), Some("ws://example"));
        create.assert_hits(1);
        get.assert_hits(1);
    }

    #[test]
    fn context_list_preserves_server_shape() {
        let server = MockServer::start();
        let list = server.mock(|when, then| {
            when.method(POST)
                .path("/instance/v1/contexts/list-contexts")
                .json_body_partial(r#"{"limit":50,"status":"locked"}"#);
            then.status(200).json_body(
                json!({"success":true,"contexts":[{"context_id":"ctx1","locked":"locked"}]}),
            );
        });
        let result = client(&server).list_contexts(Some("locked"), 50).unwrap();
        assert_eq!(result.contexts[0].context_id, "ctx1");
        list.assert_hits(1);
    }

    #[test]
    fn session_list_accepts_nullable_urls_and_duplicate_compatibility_fields() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/instance/v2/sessions");
            then.status(200).json_body(json!({
                "sessions": [{
                    "session_id": "s1",
                    "id": "s1",
                    "status": "closed",
                    "browser_mode": "normal",
                    "browser_type": "normal",
                    "inspect_url": null
                }]
            }));
        });

        let result = client(&server).list_sessions(None).unwrap();
        let session = &result.sessions[0];
        assert_eq!(session.session_id, "s1");
        assert_eq!(session.browser_mode, "normal");
        assert_eq!(session.inspect_url, None);
    }

    #[test]
    fn session_uses_legacy_compatibility_fields_as_fallbacks() {
        let session: Session = serde_json::from_value(json!({
            "id": "legacy-s1",
            "browser_type": "normal"
        }))
        .unwrap();

        assert_eq!(session.session_id, "legacy-s1");
        assert_eq!(session.browser_mode, "normal");
    }

    #[test]
    fn maps_authentication_errors_without_exposing_headers() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/instance/v2/sessions");
            then.status(401)
                .json_body(json!({"error":"invalid credentials"}));
        });
        let error = client(&server).list_sessions(None).unwrap_err();
        assert!(matches!(error, Error::Authentication(_)));
        assert!(!error.to_string().contains("test-key"));
    }

    #[test]
    fn downloads_binary_artifacts() {
        let server = MockServer::start();
        let download = server.mock(|when, then| {
            when.method(GET)
                .path("/instance/v1/sessions/s1/downloads/d1")
                .header("x-api-key", "test-key");
            then.status(200).body(vec![0_u8, 1, 2, 255]);
        });
        let result = client(&server).get_download("s1", "d1").unwrap();
        assert_eq!(result, vec![0_u8, 1, 2, 255]);
        download.assert_hits(1);
    }
}
