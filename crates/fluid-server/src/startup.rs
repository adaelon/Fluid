//! Desktop startup coordination: stable Fluid identity, default-port reuse and
//! conflict avoidance. The active server remains a normal console process; this
//! module only decides whether this invocation serves or hands off to one that is
//! already listening.

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::project_reader::ProjectReader;
use crate::reading_state::ReadingStateStore;

pub const DEFAULT_PORT: u16 = 7878;
pub const IDENTITY_PATH: &str = "/api/identity";
const IDENTITY_APP: &str = "fluid";
const IDENTITY_PROTOCOL_VERSION: u32 = 1;
const PROBE_TIMEOUT: Duration = Duration::from_millis(750);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FluidIdentity {
    app: String,
    protocol_version: u32,
}

impl FluidIdentity {
    pub fn current() -> Self {
        Self {
            app: IDENTITY_APP.into(),
            protocol_version: IDENTITY_PROTOCOL_VERSION,
        }
    }

    fn is_current(&self) -> bool {
        self.app == IDENTITY_APP && self.protocol_version == IDENTITY_PROTOCOL_VERSION
    }
}

pub struct BoundServer {
    pub listener: TcpListener,
    pub url: String,
    /// Set only when the default port belonged to a non-Fluid process.
    pub fallback_from: Option<u16>,
}

pub enum StartupSelection {
    Serve(BoundServer),
    Reuse { url: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupProjectSource {
    Explicit,
    Recent,
    None,
}

pub struct StartupProjectDecision {
    pub project: Option<ProjectReader>,
    pub source: StartupProjectSource,
    /// Non-fatal persistence/history failures that the caller should log. They
    /// never turn an automatic no-argument launch into a startup failure.
    pub diagnostics: Vec<String>,
}

impl StartupProjectDecision {
    fn empty(diagnostics: Vec<String>) -> Self {
        Self {
            project: None,
            source: StartupProjectSource::None,
            diagnostics,
        }
    }
}

/// Choose the project for a newly served process. Explicit user intent is
/// strict and wins without consulting history. Automatic history is best-effort:
/// an unreadable index or unavailable recent root is diagnosed and degrades to
/// the vacuum state without deleting the persisted record.
pub fn select_startup_project(
    explicit_project: Option<PathBuf>,
    reading_state: Option<&ReadingStateStore>,
) -> io::Result<StartupProjectDecision> {
    if let Some(path) = explicit_project {
        return ProjectReader::new(path).map(|project| StartupProjectDecision {
            project: Some(project),
            source: StartupProjectSource::Explicit,
            diagnostics: Vec::new(),
        });
    }

    let Some(reading_state) = reading_state else {
        return Ok(StartupProjectDecision::empty(Vec::new()));
    };
    let loaded = match reading_state.load_index() {
        Ok(loaded) => loaded,
        Err(error) => {
            return Ok(StartupProjectDecision::empty(vec![format!(
                "cannot read recent-project index: {error}"
            )]))
        }
    };
    let diagnostics = loaded
        .warnings
        .iter()
        .map(|warning| {
            format!(
                "ignored recent-project index {}: {}",
                warning.file, warning.message
            )
        })
        .collect::<Vec<_>>();
    let Some(recent_root) = loaded.value.and_then(|index| index.recent_project_root) else {
        return Ok(StartupProjectDecision::empty(diagnostics));
    };
    match ProjectReader::new(PathBuf::from(&recent_root)) {
        Ok(project) => Ok(StartupProjectDecision {
            project: Some(project),
            source: StartupProjectSource::Recent,
            diagnostics,
        }),
        Err(error) => {
            let mut diagnostics = diagnostics;
            diagnostics.push(format!(
                "ignored unavailable recent project {recent_root:?}: {error}"
            ));
            Ok(StartupProjectDecision::empty(diagnostics))
        }
    }
}

/// Persist the root selected for a newly served process without making index IO
/// part of startup success. `Some` is a diagnostic for the caller to log.
pub fn record_startup_project(
    reading_state: Option<&ReadingStateStore>,
    project: Option<&ProjectReader>,
) -> Option<String> {
    let (Some(reading_state), Some(project)) = (reading_state, project) else {
        return None;
    };
    reading_state
        .save_recent_project(project.root())
        .err()
        .map(|error| format!("project opened but recent-project index was not updated: {error}"))
}

/// Select the listener for one invocation. An explicit port is strict. With no
/// explicit port, bind the default; if occupied, reuse a compatible Fluid or ask
/// the OS for an ephemeral port when the occupant is something else.
pub async fn select_listener(explicit_port: Option<u16>) -> io::Result<StartupSelection> {
    select_listener_for(DEFAULT_PORT, explicit_port, PROBE_TIMEOUT).await
}

async fn select_listener_for(
    default_port: u16,
    explicit_port: Option<u16>,
    probe_timeout: Duration,
) -> io::Result<StartupSelection> {
    if let Some(port) = explicit_port {
        return bind_server(port, None).await.map(StartupSelection::Serve);
    }

    match bind_server(default_port, None).await {
        Ok(server) => Ok(StartupSelection::Serve(server)),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            let url = loopback_url(default_port);
            if is_fluid_instance(&url, probe_timeout).await {
                Ok(StartupSelection::Reuse { url })
            } else {
                bind_server(0, Some(default_port))
                    .await
                    .map(StartupSelection::Serve)
            }
        }
        Err(error) => Err(error),
    }
}

async fn bind_server(port: u16, fallback_from: Option<u16>) -> io::Result<BoundServer> {
    let requested = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(requested).await?;
    let actual = listener.local_addr()?;
    Ok(BoundServer {
        listener,
        url: loopback_url(actual.port()),
        fallback_from,
    })
}

fn loopback_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

async fn is_fluid_instance(base_url: &str, timeout: Duration) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return false;
    };
    let identity_matches = client
        .get(format!("{base_url}{IDENTITY_PATH}"))
        .send()
        .await
        .ok()
        .filter(|response| response.status().is_success());
    if let Some(response) = identity_matches {
        if response
            .json::<FluidIdentity>()
            .await
            .is_ok_and(|identity| identity.is_current())
        {
            return true;
        }
    }

    // Releases before the identity endpoint are recognized only when two
    // independent, stable public surfaces agree: the SPA shell names Fluid and
    // the project-tree endpoint has Fluid's `{ files: [...] }` shape.
    let root_matches = match client.get(base_url).send().await {
        Ok(response) if response.status().is_success() => response
            .text()
            .await
            .is_ok_and(|html| html.contains("<title>Fluid</title>") && html.contains("id=\"app\"")),
        _ => false,
    };
    if !root_matches {
        return false;
    }
    match client
        .get(format!("{base_url}/api/project/tree"))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response
            .json::<serde_json::Value>()
            .await
            .is_ok_and(|body| body.get("files").is_some_and(serde_json::Value::is_array)),
        _ => false,
    }
}

/// Preserve the positional project argument when reusing an existing instance.
/// The path is canonicalized in this process so a relative path never gets
/// reinterpreted against the existing process's working directory.
pub async fn handoff_project(base_url: &str, project: &Path) -> anyhow::Result<()> {
    let project = std::fs::canonicalize(project)?;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .post(format!("{base_url}/api/project/open"))
        .json(&serde_json::json!({ "path": project.to_string_lossy() }))
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        anyhow::bail!("existing Fluid rejected project handoff ({status}): {detail}");
    }
    Ok(())
}

/// Reuse keeps the current root when this invocation has no positional project.
/// Returning whether a handoff occurred makes the zero-request branch directly
/// testable without changing the existing listener/reuse policy.
pub async fn handoff_project_if_present(
    base_url: &str,
    project: Option<&Path>,
) -> anyhow::Result<bool> {
    let Some(project) = project else {
        return Ok(false);
    };
    handoff_project(base_url, project).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::reading_state::ReadingStateStore;
    use axum::{
        extract::State,
        routing::{get, post},
        Json, Router,
    };

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "fluid-startup-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn make_dir(&self, name: &str) -> PathBuf {
            let path = self.path().join(name);
            std::fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn fixture_server(app: Router) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (port, task)
    }

    #[tokio::test]
    async fn free_default_port_is_bound_exactly() {
        let candidate = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = candidate.local_addr().unwrap().port();
        drop(candidate);

        let selected = select_listener_for(port, None, Duration::from_millis(100))
            .await
            .unwrap();
        let StartupSelection::Serve(server) = selected else {
            panic!("free default port must start a server");
        };
        assert_eq!(server.listener.local_addr().unwrap().port(), port);
        assert_eq!(server.url, loopback_url(port));
        assert_eq!(server.fallback_from, None);
    }

    #[tokio::test]
    async fn compatible_fluid_on_default_port_is_reused() {
        let app = Router::new().route(
            IDENTITY_PATH,
            get(|| async { Json(FluidIdentity::current()) }),
        );
        let (port, task) = fixture_server(app).await;

        let selected = select_listener_for(port, None, Duration::from_secs(1))
            .await
            .unwrap();
        let StartupSelection::Reuse { url } = selected else {
            panic!("compatible Fluid must be reused");
        };
        assert_eq!(url, loopback_url(port));
        task.abort();
    }

    #[tokio::test]
    async fn legacy_fluid_without_identity_is_reused() {
        let app = Router::new()
            .route(
                "/",
                get(|| async {
                    axum::response::Html(
                        "<!doctype html><title>Fluid</title><div id=\"app\"></div>",
                    )
                }),
            )
            .route(
                "/api/project/tree",
                get(|| async { Json(serde_json::json!({ "files": [] })) }),
            );
        let (port, task) = fixture_server(app).await;

        let selected = select_listener_for(port, None, Duration::from_secs(1))
            .await
            .unwrap();
        assert!(matches!(selected, StartupSelection::Reuse { .. }));
        task.abort();
    }

    #[tokio::test]
    async fn foreign_default_port_occupant_uses_an_ephemeral_port() {
        let (port, task) = fixture_server(Router::new()).await;

        let selected = select_listener_for(port, None, Duration::from_secs(1))
            .await
            .unwrap();
        let StartupSelection::Serve(server) = selected else {
            panic!("foreign occupant must not be reused");
        };
        let actual = server.listener.local_addr().unwrap().port();
        assert_ne!(actual, port);
        assert_ne!(actual, 0);
        assert_eq!(server.url, loopback_url(actual));
        assert_eq!(server.fallback_from, Some(port));
        task.abort();
    }

    #[tokio::test]
    async fn explicit_port_conflict_is_strict_even_for_fluid() {
        let app = Router::new().route(
            IDENTITY_PATH,
            get(|| async { Json(FluidIdentity::current()) }),
        );
        let (port, task) = fixture_server(app).await;

        let error = select_listener_for(port, Some(port), Duration::from_secs(1))
            .await
            .err()
            .expect("explicit conflict must fail");
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        task.abort();
    }

    #[test]
    fn explicit_project_wins_over_a_valid_recent_project() {
        let temp = TempDir::new("explicit-wins");
        let explicit = temp.make_dir("explicit");
        let recent = temp.make_dir("recent");
        let store = ReadingStateStore::new(temp.path().join("user-data"));
        store.save_recent_project(&recent).unwrap();

        let selected = select_startup_project(Some(explicit.clone()), Some(&store)).unwrap();

        assert_eq!(selected.source, StartupProjectSource::Explicit);
        assert_eq!(
            selected.project.unwrap().root(),
            explicit.canonicalize().unwrap()
        );
        assert!(selected.diagnostics.is_empty());
    }

    #[test]
    fn invalid_explicit_project_remains_a_strict_startup_error() {
        let temp = TempDir::new("invalid-explicit");
        let store = ReadingStateStore::new(temp.path().join("user-data"));

        assert!(
            select_startup_project(Some(temp.path().join("missing-explicit")), Some(&store))
                .is_err()
        );
    }

    #[test]
    fn valid_recent_project_is_selected_for_a_new_no_argument_server() {
        let temp = TempDir::new("recent-valid");
        let recent = temp.make_dir("recent");
        let store = ReadingStateStore::new(temp.path().join("user-data"));
        store.save_recent_project(&recent).unwrap();

        let selected = select_startup_project(None, Some(&store)).unwrap();

        assert_eq!(selected.source, StartupProjectSource::Recent);
        assert_eq!(
            selected.project.unwrap().root(),
            recent.canonicalize().unwrap()
        );
        assert!(selected.diagnostics.is_empty());
    }

    #[test]
    fn unavailable_recent_project_silently_degrades_without_deleting_the_index() {
        let temp = TempDir::new("recent-missing");
        let recent = temp.make_dir("recent");
        let store = ReadingStateStore::new(temp.path().join("user-data"));
        store.save_recent_project(&recent).unwrap();
        let index_before = std::fs::read(store.index_path()).unwrap();
        std::fs::remove_dir_all(&recent).unwrap();

        let selected = select_startup_project(None, Some(&store)).unwrap();

        assert_eq!(selected.source, StartupProjectSource::None);
        assert!(selected.project.is_none());
        assert_eq!(selected.diagnostics.len(), 1);
        assert_eq!(std::fs::read(store.index_path()).unwrap(), index_before);
    }

    #[test]
    fn missing_recent_index_starts_in_the_vacuum_state() {
        let temp = TempDir::new("no-index");
        let store = ReadingStateStore::new(temp.path().join("user-data"));

        let selected = select_startup_project(None, Some(&store)).unwrap();

        assert_eq!(selected.source, StartupProjectSource::None);
        assert!(selected.project.is_none());
        assert!(selected.diagnostics.is_empty());
    }

    #[test]
    fn successful_served_project_updates_the_recent_index_best_effort() {
        let temp = TempDir::new("record-startup");
        let previous = temp.make_dir("previous");
        let selected_root = temp.make_dir("selected");
        let store = ReadingStateStore::new(temp.path().join("user-data"));
        store.save_recent_project(&previous).unwrap();
        let selected = select_startup_project(Some(selected_root.clone()), Some(&store)).unwrap();

        assert!(record_startup_project(Some(&store), selected.project.as_ref()).is_none());
        assert_eq!(
            store
                .load_index()
                .unwrap()
                .value
                .unwrap()
                .recent_project_root,
            Some(selected_root.canonicalize().unwrap().display().to_string())
        );

        let failed_root = temp.make_dir("failed-update");
        let failed_project = ProjectReader::new(failed_root).unwrap();
        store.fail_next_atomic_replace_for_test();
        assert!(record_startup_project(Some(&store), Some(&failed_project)).is_some());
        assert_eq!(
            store
                .load_index()
                .unwrap()
                .value
                .unwrap()
                .recent_project_root,
            Some(selected_root.canonicalize().unwrap().display().to_string())
        );
    }

    #[tokio::test]
    async fn reuse_without_an_explicit_project_sends_no_handoff_request() {
        async fn count_handoff(State(count): State<Arc<AtomicUsize>>) {
            count.fetch_add(1, Ordering::SeqCst);
        }

        let count = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/api/project/open", post(count_handoff))
            .with_state(Arc::clone(&count));
        let (port, task) = fixture_server(app).await;

        let handed_off = handoff_project_if_present(&loopback_url(port), None)
            .await
            .unwrap();

        assert!(!handed_off);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        task.abort();
    }

    #[tokio::test]
    async fn project_handoff_sends_an_absolute_path() {
        async fn capture(
            State(seen): State<Arc<Mutex<Option<String>>>>,
            Json(body): Json<serde_json::Value>,
        ) {
            *seen.lock().unwrap() = body["path"].as_str().map(str::to_owned);
        }

        let seen = Arc::new(Mutex::new(None));
        let app = Router::new()
            .route("/api/project/open", post(capture))
            .with_state(seen.clone());
        let (port, task) = fixture_server(app).await;
        let project = std::env::current_dir().unwrap();

        handoff_project(&loopback_url(port), &project)
            .await
            .unwrap();
        let captured = seen.lock().unwrap().clone().expect("captured path");
        assert!(Path::new(&captured).is_absolute());
        assert_eq!(
            std::fs::canonicalize(captured).unwrap(),
            std::fs::canonicalize(project).unwrap()
        );
        task.abort();
    }
}
