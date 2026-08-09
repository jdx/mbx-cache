use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};

use crate::{
    auth::{Access, Authorizer},
    metadata::{CommitOutcome, MetadataStore},
    model::{
        ActionResult, Algorithm, Digest, Directory, RustcAction, RustcMetadata, TaskAction,
        TaskMetadata,
    },
    storage::{BlobStore, PutOutcome},
};

#[derive(Clone)]
pub struct AppState {
    pub blobs: Arc<dyn BlobStore>,
    pub metadata: Arc<dyn MetadataStore>,
    pub auth: Authorizer,
    pub max_blob_bytes: u64,
    metrics: Arc<Metrics>,
}

impl AppState {
    pub fn new(
        blobs: Arc<dyn BlobStore>,
        metadata: Arc<dyn MetadataStore>,
        auth: Authorizer,
        max_blob_bytes: u64,
    ) -> Self {
        Self {
            blobs,
            metadata,
            auth,
            max_blob_bytes,
            metrics: Arc::new(Metrics::default()),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let limit = usize::try_from(state.max_blob_bytes).unwrap_or(usize::MAX);
    Router::new()
        .route("/v1/status", get(status))
        .route("/v1/capabilities", get(capabilities))
        .route(
            "/v1/blobs/{algorithm}/{hash}/{size}",
            get(get_blob).put(put_blob),
        )
        .route("/v1/blobs:missing", post(missing_blobs))
        .route(
            "/v1/action-results/{algorithm}/{hash}/{size}",
            get(get_action_result).put(put_action_result),
        )
        .route("/metrics", get(metrics))
        .layer(RequestBodyLimitLayer::new(limit))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn status() -> impl IntoResponse {
    Json(serde_json::json!({"status":"ok","protocol":1}))
}

async fn capabilities(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "protocol":{"major":1,"minor":0},
        "digest_algorithms":["blake3","sha256"],
        "compressors":["identity"],
        "action_kinds":{
            "rustc":{"action_schema":1,"metadata_schema":1},
            "task":{"action_schema":1,"metadata_schema":1}
        },
        "features":{"batch":true,"resumable_uploads":false,"delegated_transfers":false},
        "limits":{"max_batch_items":10000,"max_inline_blob_bytes":1048576,"max_blob_bytes":state.max_blob_bytes}
    }))
}

async fn get_blob(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(parts): Path<(String, String, u64)>,
) -> Result<Response, ApiError> {
    let namespace = state.auth.authorize(&headers, Access::Read).await?;
    let digest = parse_digest(parts)?;
    if !state
        .metadata
        .blob_visible(&namespace, &digest)
        .await
        .map_err(ApiError::internal)?
    {
        return Err(ApiError::not_found("blob not found"));
    }
    let blob = state
        .blobs
        .get(&digest)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("blob not found"))?;
    state.metrics.blob_hits.fetch_add(1, Ordering::Relaxed);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, blob.size)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::ETAG, format!("\"{}\"", digest.hash))
        .header("digest", format!("{}={}", digest.algorithm, digest.hash))
        .body(Body::from_stream(blob.stream))
        .map_err(ApiError::internal)
}

async fn put_blob(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(parts): Path<(String, String, u64)>,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let namespace = state.auth.authorize(&headers, Access::Write).await?;
    require_immutable_precondition(&headers)?;
    let digest = parse_digest(parts)?;
    if digest.size > state.max_blob_bytes {
        return Err(ApiError::too_large("blob exceeds configured limit"));
    }
    let temp = NamedTempFile::new().map_err(ApiError::internal)?;
    let path = temp.path().to_owned();
    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(ApiError::internal)?;
    let mut stream = body.into_data_stream();
    let mut size = 0_u64;
    let mut blake3 = blake3::Hasher::new();
    let mut sha256 = sha2::Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| ApiError::bad_request(error.to_string()))?;
        size = size
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| ApiError::too_large("blob is too large"))?;
        if size > digest.size || size > state.max_blob_bytes {
            return Err(ApiError::too_large(
                "blob exceeds declared or configured size",
            ));
        }
        match digest.algorithm {
            Algorithm::Blake3 => {
                blake3.update(&chunk);
            }
            Algorithm::Sha256 => {
                sha256.update(&chunk);
            }
        }
        file.write_all(&chunk).await.map_err(ApiError::internal)?;
    }
    file.flush().await.map_err(ApiError::internal)?;
    let actual_hash = match digest.algorithm {
        Algorithm::Blake3 => blake3.finalize().to_hex().to_string(),
        Algorithm::Sha256 => hex::encode(sha256.finalize()),
    };
    if size != digest.size || actual_hash != digest.hash {
        return Err(ApiError::bad_request(
            "content does not match the requested digest",
        ));
    }
    let outcome = state
        .blobs
        .put(&digest, &path)
        .await
        .map_err(ApiError::internal)?;
    state
        .metadata
        .register_blob(&namespace, &digest)
        .await
        .map_err(ApiError::internal)?;
    state.metrics.blob_uploads.fetch_add(1, Ordering::Relaxed);
    Ok(match outcome {
        PutOutcome::Created => StatusCode::CREATED,
        PutOutcome::AlreadyExists => StatusCode::NO_CONTENT,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MissingRequest {
    digests: Vec<Digest>,
}

#[derive(Serialize)]
struct MissingResponse {
    missing: Vec<Digest>,
}

async fn missing_blobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MissingRequest>,
) -> Result<Json<MissingResponse>, ApiError> {
    let namespace = state.auth.authorize(&headers, Access::Read).await?;
    if request.digests.len() > 10_000 {
        return Err(ApiError::bad_request(
            "at most 10000 digests may be checked",
        ));
    }
    let mut missing = Vec::new();
    for digest in request.digests {
        validate_digest(&digest)?;
        if !state
            .metadata
            .blob_visible(&namespace, &digest)
            .await
            .map_err(ApiError::internal)?
        {
            missing.push(digest);
        }
    }
    Ok(Json(MissingResponse { missing }))
}

async fn get_action_result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(parts): Path<(String, String, u64)>,
) -> Result<Json<ActionResult>, ApiError> {
    let namespace = state.auth.authorize(&headers, Access::Read).await?;
    let action = parse_action_digest(parts)?;
    match state
        .metadata
        .get(&namespace, &action)
        .await
        .map_err(ApiError::internal)?
    {
        Some(result) => {
            state.metrics.action_hits.fetch_add(1, Ordering::Relaxed);
            Ok(Json(result))
        }
        None => {
            state.metrics.action_misses.fetch_add(1, Ordering::Relaxed);
            Err(ApiError::not_found("action result not found"))
        }
    }
}

async fn put_action_result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(parts): Path<(String, String, u64)>,
    Json(result): Json<ActionResult>,
) -> Result<StatusCode, ApiError> {
    let namespace = state.auth.authorize(&headers, Access::Write).await?;
    require_immutable_precondition(&headers)?;
    let action = parse_action_digest(parts)?;
    if result.version != 1 || result.action != action {
        return Err(ApiError::bad_request(
            "action result does not match request",
        ));
    }
    let action_kind = validate_action_descriptor(&state, &namespace, &action).await?;
    validate_action_result_shape(&result, action_kind)?;
    if let Some(metadata) = &result.metadata {
        validate_client_metadata(&state, &namespace, metadata, action_kind).await?;
    }
    if let Some(root) = &result.output_root {
        validate_tree(&state, &namespace, root).await?;
    }
    let outcome = state
        .metadata
        .commit(&namespace, &action, &result)
        .await
        .map_err(ApiError::internal)?;
    state.metrics.action_commits.fetch_add(1, Ordering::Relaxed);
    match outcome {
        CommitOutcome::Created => Ok(StatusCode::CREATED),
        CommitOutcome::AlreadyExists => Ok(StatusCode::NO_CONTENT),
        CommitOutcome::Conflict => Err(ApiError::conflict(
            "an immutable action result already exists",
        )),
    }
}

async fn require_blob(
    state: &AppState,
    namespace: &str,
    digest: &Digest,
    label: &str,
) -> Result<(), ApiError> {
    validate_digest(digest)?;
    if state
        .metadata
        .blob_visible(namespace, digest)
        .await
        .map_err(ApiError::internal)?
    {
        Ok(())
    } else {
        Err(ApiError::unprocessable(format!("{label} is missing")))
    }
}

async fn validate_tree(state: &AppState, namespace: &str, root: &Digest) -> Result<(), ApiError> {
    let mut pending = vec![(root.clone(), HashSet::new())];
    let mut seen = HashSet::new();
    while let Some((digest, mut ancestors)) = pending.pop() {
        if !ancestors.insert(digest.clone()) {
            return Err(ApiError::unprocessable("directory graph contains a cycle"));
        }
        if !seen.insert(digest.clone()) {
            continue;
        }
        if seen.len() > 100_000 {
            return Err(ApiError::unprocessable("directory graph is too large"));
        }
        if digest.size > 16 * 1024 * 1024 {
            return Err(ApiError::unprocessable("directory object is too large"));
        }
        if !state
            .metadata
            .blob_visible(namespace, &digest)
            .await
            .map_err(ApiError::internal)?
        {
            return Err(ApiError::unprocessable("directory object is missing"));
        }
        let mut blob = state
            .blobs
            .get(&digest)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::unprocessable("directory object is missing"))?;
        let mut bytes = Vec::with_capacity(digest.size as usize);
        while let Some(chunk) = blob.stream.next().await {
            bytes.extend_from_slice(&chunk.map_err(ApiError::internal)?);
        }
        let directory: Directory = serde_json::from_slice(&bytes)
            .map_err(|_| ApiError::unprocessable("directory object is invalid"))?;
        if serde_json::to_vec(&directory).map_err(ApiError::internal)? != bytes {
            return Err(ApiError::unprocessable(
                "directory object is not canonical JSON",
            ));
        }
        if directory.version != 1 {
            return Err(ApiError::unprocessable(
                "unsupported directory object version",
            ));
        }
        validate_directory_entries(&directory)?;
        for file in directory.files {
            require_blob(state, namespace, &file.digest, "file blob").await?;
        }
        pending.extend(
            directory
                .directories
                .into_iter()
                .map(|node| (node.digest, ancestors.clone())),
        );
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActionKind {
    Rustc,
    Task,
}

fn validate_action_result_shape(
    result: &ActionResult,
    action_kind: ActionKind,
) -> Result<(), ApiError> {
    if action_kind == ActionKind::Rustc
        && (result.metadata.is_none() || result.output_root.is_none())
    {
        return Err(ApiError::unprocessable(
            "rustc action results require metadata and an output root",
        ));
    }
    Ok(())
}

async fn validate_action_descriptor(
    state: &AppState,
    namespace: &str,
    digest: &Digest,
) -> Result<ActionKind, ApiError> {
    let value = read_canonical_object(state, namespace, digest, "action descriptor").await?;
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::unprocessable("action descriptor must be a JSON object"))?;
    let kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ApiError::unprocessable("action descriptor kind is required"))?
        .to_owned();
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ApiError::unprocessable("action descriptor version is required"))?;
    match kind.as_str() {
        "task" => validate_task_action(value, version),
        "rustc" => validate_rustc_action(value, version),
        _ => Err(ApiError::unprocessable(format!(
            "unsupported action kind {kind:?}"
        ))),
    }
}

fn validate_task_action(value: serde_json::Value, version: u64) -> Result<ActionKind, ApiError> {
    if version != 1 {
        return Err(ApiError::unprocessable(format!(
            "unsupported task action schema {version}"
        )));
    }
    let object = value
        .as_object()
        .expect("action descriptors are checked to be objects");
    for field in [
        "version",
        "kind",
        "task",
        "phase",
        "run",
        "args",
        "shell",
        "outputs",
        "root",
        "source_hash",
        "environment",
        "vars",
        "tools",
        "os",
        "arch",
    ] {
        if !object.contains_key(field) {
            return Err(ApiError::unprocessable(format!(
                "task action field {field:?} is required"
            )));
        }
    }
    let action = serde_json::from_value::<TaskAction>(value)
        .map_err(|error| ApiError::unprocessable(format!("invalid task action: {error}")))?;
    if !action.validate() {
        return Err(ApiError::unprocessable("invalid task action values"));
    }
    Ok(ActionKind::Task)
}

fn validate_rustc_action(value: serde_json::Value, version: u64) -> Result<ActionKind, ApiError> {
    if version != 1 {
        return Err(ApiError::unprocessable(format!(
            "unsupported rustc action schema {version}"
        )));
    }
    let action = serde_json::from_value::<RustcAction>(value)
        .map_err(|error| ApiError::unprocessable(format!("invalid rustc action: {error}")))?;
    if !action.validate() {
        return Err(ApiError::unprocessable("invalid rustc action values"));
    }
    Ok(ActionKind::Rustc)
}

async fn validate_client_metadata(
    state: &AppState,
    namespace: &str,
    digest: &Digest,
    action_kind: ActionKind,
) -> Result<(), ApiError> {
    let value = read_canonical_object(state, namespace, digest, "client metadata").await?;
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::unprocessable("client metadata must be a JSON object"))?;
    let kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ApiError::unprocessable("client metadata kind is required"))?;
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ApiError::unprocessable("client metadata version is required"))?;
    let expected_kind = match action_kind {
        ActionKind::Rustc => "rustc",
        ActionKind::Task => "task",
    };
    if kind != expected_kind {
        return Err(ApiError::unprocessable(
            "client metadata kind does not match action kind",
        ));
    }
    match action_kind {
        ActionKind::Task => validate_task_metadata(value, version),
        ActionKind::Rustc => validate_rustc_metadata(state, namespace, value, version).await,
    }
}

fn validate_task_metadata(value: serde_json::Value, version: u64) -> Result<(), ApiError> {
    if version != 1 {
        return Err(ApiError::unprocessable(format!(
            "unsupported task metadata schema {version}"
        )));
    }
    let metadata: TaskMetadata = serde_json::from_value(value)
        .map_err(|error| ApiError::unprocessable(format!("invalid task metadata: {error}")))?;
    if !metadata.validate() {
        return Err(ApiError::unprocessable("invalid task metadata values"));
    }
    for root in metadata.roots {
        validate_task_root(&root)?;
    }
    Ok(())
}

async fn validate_rustc_metadata(
    state: &AppState,
    namespace: &str,
    value: serde_json::Value,
    version: u64,
) -> Result<(), ApiError> {
    if version != 1 {
        return Err(ApiError::unprocessable(format!(
            "unsupported rustc metadata schema {version}"
        )));
    }
    let metadata: RustcMetadata = serde_json::from_value(value)
        .map_err(|error| ApiError::unprocessable(format!("invalid rustc metadata: {error}")))?;
    if !metadata.validate() {
        return Err(ApiError::unprocessable("invalid rustc metadata values"));
    }
    require_blob(state, namespace, &metadata.stdout, "rustc stdout blob").await?;
    require_blob(state, namespace, &metadata.stderr, "rustc stderr blob").await?;
    Ok(())
}

fn validate_task_root(root: &str) -> Result<(), ApiError> {
    if root.is_empty()
        || root.starts_with('/')
        || root.contains(['\\', '\0'])
        || root
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(ApiError::unprocessable(
            "task metadata root must be a safe relative path",
        ));
    }
    Ok(())
}

async fn read_canonical_object(
    state: &AppState,
    namespace: &str,
    digest: &Digest,
    label: &str,
) -> Result<serde_json::Value, ApiError> {
    validate_digest(digest)?;
    if digest.size > 16 * 1024 * 1024 {
        return Err(ApiError::unprocessable(format!("{label} is too large")));
    }
    if !state
        .metadata
        .blob_visible(namespace, digest)
        .await
        .map_err(ApiError::internal)?
    {
        return Err(ApiError::unprocessable(format!("{label} blob is missing")));
    }
    let mut blob = state
        .blobs
        .get(digest)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::unprocessable(format!("{label} blob is missing")))?;
    let mut bytes = Vec::with_capacity(digest.size as usize);
    while let Some(chunk) = blob.stream.next().await {
        bytes.extend_from_slice(&chunk.map_err(ApiError::internal)?);
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::unprocessable(format!("{label} is invalid JSON")))?;
    if !value.is_object() || serde_json::to_vec(&value).map_err(ApiError::internal)? != bytes {
        return Err(ApiError::unprocessable(format!(
            "{label} is not canonical JSON"
        )));
    }
    Ok(value)
}

fn validate_directory_entries(directory: &Directory) -> Result<(), ApiError> {
    let mut names = HashSet::new();
    for node in &directory.directories {
        validate_entry(&mut names, &node.name, node.mode)?;
    }
    for node in &directory.files {
        validate_entry(&mut names, &node.name, node.mode)?;
        let _ = node.executable;
    }
    for node in &directory.symlinks {
        validate_entry(&mut names, &node.name, node.mode)?;
        if node.target.is_empty() || node.target.contains('\0') {
            return Err(ApiError::unprocessable("invalid symlink target"));
        }
    }
    Ok(())
}

fn validate_entry(names: &mut HashSet<String>, name: &str, mode: u32) -> Result<(), ApiError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
        || mode > 0o7777
    {
        return Err(ApiError::unprocessable("invalid directory entry"));
    }
    if !names.insert(name.to_owned()) {
        return Err(ApiError::unprocessable("duplicate directory entry"));
    }
    Ok(())
}

async fn metrics(State(state): State<AppState>) -> String {
    format!(
        concat!(
            "# TYPE mise_cache_blob_hits_total counter\nmise_cache_blob_hits_total {}\n",
            "# TYPE mise_cache_blob_uploads_total counter\nmise_cache_blob_uploads_total {}\n",
            "# TYPE mise_cache_action_hits_total counter\nmise_cache_action_hits_total {}\n",
            "# TYPE mise_cache_action_misses_total counter\nmise_cache_action_misses_total {}\n",
            "# TYPE mise_cache_action_commits_total counter\nmise_cache_action_commits_total {}\n"
        ),
        state.metrics.blob_hits.load(Ordering::Relaxed),
        state.metrics.blob_uploads.load(Ordering::Relaxed),
        state.metrics.action_hits.load(Ordering::Relaxed),
        state.metrics.action_misses.load(Ordering::Relaxed),
        state.metrics.action_commits.load(Ordering::Relaxed)
    )
}

fn parse_digest((algorithm, hash, size): (String, String, u64)) -> Result<Digest, ApiError> {
    let algorithm = algorithm.parse().map_err(ApiError::bad_request)?;
    let digest = Digest {
        algorithm,
        hash,
        size,
    };
    validate_digest(&digest)?;
    Ok(digest)
}

fn parse_action_digest(parts: (String, String, u64)) -> Result<Digest, ApiError> {
    let digest = parse_digest(parts)?;
    if digest.algorithm != Algorithm::Blake3 {
        return Err(ApiError::bad_request("action result keys must use blake3"));
    }
    Ok(digest)
}

fn validate_digest(digest: &Digest) -> Result<(), ApiError> {
    if digest.validate() {
        Ok(())
    } else {
        Err(ApiError::bad_request("invalid digest"))
    }
}

fn require_immutable_precondition(headers: &HeaderMap) -> Result<(), ApiError> {
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        == Some("*")
    {
        Ok(())
    } else {
        Err(ApiError::precondition("If-None-Match: * is required"))
    }
}

#[derive(Default)]
struct Metrics {
    blob_hits: AtomicU64,
    blob_uploads: AtomicU64,
    action_hits: AtomicU64,
    action_misses: AtomicU64,
    action_commits: AtomicU64,
}

pub struct ApiError {
    status: StatusCode,
    message: String,
    advertise_protocol: bool,
}

impl ApiError {
    fn new(status: StatusCode, message: impl ToString) -> Self {
        Self {
            status,
            message: message.to_string(),
            advertise_protocol: false,
        }
    }
    pub fn bad_request(message: impl ToString) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    pub fn unauthorized(message: impl ToString) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }
    pub fn forbidden(message: impl ToString) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }
    pub fn not_found(message: impl ToString) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
    pub fn conflict(message: impl ToString) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }
    pub fn too_large(message: impl ToString) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, message)
    }
    pub fn unprocessable(message: impl ToString) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, message)
    }
    pub fn precondition(message: impl ToString) -> Self {
        Self::new(StatusCode::PRECONDITION_FAILED, message)
    }
    pub fn upgrade_required() -> Self {
        Self {
            status: StatusCode::UPGRADE_REQUIRED,
            message: "mise cache protocol version 1 is required".into(),
            advertise_protocol: true,
        }
    }
    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(%error, "request failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response =
            (self.status, Json(serde_json::json!({"error":self.message}))).into_response();
        if self.advertise_protocol {
            response
                .headers_mut()
                .insert("mise-cache-protocol", header::HeaderValue::from_static("1"));
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{metadata::MemoryMetadata, storage::FilesystemStore};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn test_app() -> (Router, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let blobs = Arc::new(FilesystemStore::new(directory.path()).await.unwrap());
        let metadata = Arc::new(MemoryMetadata::default());
        let auth = Authorizer::new(None, None, true).await.unwrap();
        (
            router(AppState::new(blobs, metadata, auth, 1024 * 1024)),
            directory,
        )
    }

    fn request(method: &str, uri: String, body: Body) -> axum::http::Request<Body> {
        axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("mise-cache-protocol", "1")
            .header("mise-cache-namespace", "test/project")
            .header(header::IF_NONE_MATCH, "*")
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap()
    }

    fn digest(bytes: &[u8]) -> Digest {
        Digest {
            algorithm: Algorithm::Blake3,
            hash: blake3::hash(bytes).to_hex().to_string(),
            size: bytes.len() as u64,
        }
    }

    fn canonical(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    fn task_action(version: u8) -> Vec<u8> {
        canonical(serde_json::json!({
            "arch":"x86_64",
            "args":[],
            "command_inputs":[],
            "dependency_keys":[],
            "environment":{},
            "kind":"task",
            "os":"linux",
            "outputs":["target/debug/widget"],
            "phase":"normal",
            "root":".",
            "run":["cargo build"],
            "shell":null,
            "source_hash":"blake3:source",
            "task":"build",
            "tools":["core:rust@1.92.0"],
            "vars":{},
            "version":version
        }))
    }

    fn task_metadata(kind: &str, version: u8) -> Vec<u8> {
        canonical(serde_json::json!({
            "execution_duration_ns":1,
            "kind":kind,
            "output":[{"line":"built widget","stream":"stdout"}],
            "restored_bytes":42,
            "roots":["target/debug/widget"],
            "task_identity":"build:.",
            "version":version
        }))
    }

    fn rustc_action(input: &Digest, version: u8) -> Vec<u8> {
        canonical(serde_json::json!({
            "adapter_version":1,
            "arguments":[
                "--crate-name=widget",
                "--crate-type=lib",
                "--emit=metadata,link",
                "--out-dir=${target}/debug/deps"
            ],
            "compiler":{
                "host":"x86_64-unknown-linux-gnu",
                "rustc_version":"1.97.1 (test)",
                "toolchain":"core:rust@1.97.1"
            },
            "environment":{"CARGO_PKG_VERSION":"1.0.0"},
            "inputs":[{"digest":input,"path":"${workspace}/src/lib.rs"}],
            "kind":"rustc",
            "version":version
        }))
    }

    fn rustc_metadata(stdout: &Digest, stderr: &Digest, version: u8) -> Vec<u8> {
        canonical(serde_json::json!({
            "kind":"rustc",
            "stderr":stderr,
            "stdout":stdout,
            "version":version
        }))
    }

    fn output_directory(artifact: &Digest) -> Vec<u8> {
        canonical(serde_json::json!({
            "directories":[],
            "files":[{
                "digest":artifact,
                "executable":false,
                "mode":420,
                "name":"libwidget.rlib"
            }],
            "symlinks":[],
            "version":1
        }))
    }

    async fn upload_blob(app: &Router, bytes: &[u8]) -> Digest {
        let digest = digest(bytes);
        let uri = format!(
            "/v1/blobs/{}/{}/{}",
            digest.algorithm, digest.hash, digest.size
        );
        assert_eq!(
            app.clone()
                .oneshot(request("PUT", uri, Body::from(bytes.to_vec())))
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
        digest
    }

    #[tokio::test]
    async fn streams_and_validates_blobs() {
        let (app, _directory) = test_app().await;
        let bytes = b"cached output";
        let digest = digest(bytes);
        let uri = format!(
            "/v1/blobs/{}/{}/{}",
            digest.algorithm, digest.hash, digest.size
        );
        let response = app
            .clone()
            .oneshot(request("PUT", uri.clone(), Body::from(bytes.as_slice())))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let response = app
            .oneshot(request("GET", uri, Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            bytes.as_slice()
        );
    }

    #[tokio::test]
    async fn publishes_action_result_only_after_references_exist() {
        let (app, _directory) = test_app().await;
        let action = upload_blob(&app, &task_action(1)).await;
        let metadata = upload_blob(&app, &task_metadata("task", 1)).await;

        let result_uri = format!(
            "/v1/action-results/{}/{}/{}",
            action.algorithm, action.hash, action.size
        );
        let result = ActionResult {
            action,
            metadata: Some(metadata),
            output_root: None,
            version: 1,
        };
        let body = serde_json::to_vec(&result).unwrap();
        assert_eq!(
            app.clone()
                .oneshot(request("PUT", result_uri.clone(), Body::from(body)))
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
        let response = app
            .oneshot(request("GET", result_uri, Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["version"], 1);
        assert!(body.get("result").is_none());
        assert!(body.get("signatures").is_none());
    }

    #[tokio::test]
    async fn advertises_action_schemas() {
        let (app, _directory) = test_app().await;
        let response = app
            .oneshot(request("GET", "/v1/capabilities".into(), Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["action_kinds"]["task"]["action_schema"], 1);
        assert_eq!(body["action_kinds"]["task"]["metadata_schema"], 1);
        assert_eq!(body["action_kinds"]["rustc"]["action_schema"], 1);
        assert_eq!(body["action_kinds"]["rustc"]["metadata_schema"], 1);
        assert!(body["features"].get("signed_results").is_none());
    }

    #[tokio::test]
    async fn publishes_rustc_results_without_uploading_source_inputs() {
        let (app, _directory) = test_app().await;
        // Action input digests identify local source content; only result
        // metadata, diagnostics, and output artifacts are CAS references.
        let source = digest(b"pub fn widget() {}\n");
        let action = upload_blob(&app, &rustc_action(&source, 1)).await;
        let stdout = upload_blob(&app, b"").await;
        let stderr = upload_blob(&app, b"warning: cached diagnostic\n").await;
        let metadata = upload_blob(&app, &rustc_metadata(&stdout, &stderr, 1)).await;
        let artifact = upload_blob(&app, b"rlib artifact").await;
        let output_root = upload_blob(&app, &output_directory(&artifact)).await;
        let result_uri = format!(
            "/v1/action-results/{}/{}/{}",
            action.algorithm, action.hash, action.size
        );
        let body = serde_json::to_vec(&ActionResult {
            action,
            metadata: Some(metadata),
            output_root: Some(output_root),
            version: 1,
        })
        .unwrap();

        assert_eq!(
            app.oneshot(request("PUT", result_uri, Body::from(body)))
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }

    #[tokio::test]
    async fn rejects_incomplete_rustc_results() {
        let (app, _directory) = test_app().await;
        let source = digest(b"pub fn widget() {}\n");
        let action = upload_blob(&app, &rustc_action(&source, 1)).await;
        let result_uri = format!(
            "/v1/action-results/{}/{}/{}",
            action.algorithm, action.hash, action.size
        );
        let body = serde_json::to_vec(&ActionResult {
            action,
            metadata: None,
            output_root: None,
            version: 1,
        })
        .unwrap();

        assert_eq!(
            app.oneshot(request("PUT", result_uri, Body::from(body)))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn rejects_missing_rustc_diagnostic_blobs() {
        let (app, _directory) = test_app().await;
        let source = digest(b"pub fn widget() {}\n");
        let action = upload_blob(&app, &rustc_action(&source, 1)).await;
        let stdout = upload_blob(&app, b"").await;
        let missing_stderr = digest(b"missing diagnostic\n");
        let metadata = upload_blob(&app, &rustc_metadata(&stdout, &missing_stderr, 1)).await;
        let artifact = upload_blob(&app, b"rlib artifact").await;
        let output_root = upload_blob(&app, &output_directory(&artifact)).await;
        let result_uri = format!(
            "/v1/action-results/{}/{}/{}",
            action.algorithm, action.hash, action.size
        );
        let body = serde_json::to_vec(&ActionResult {
            action,
            metadata: Some(metadata),
            output_root: Some(output_root),
            version: 1,
        })
        .unwrap();

        assert_eq!(
            app.oneshot(request("PUT", result_uri, Body::from(body)))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn rejects_invalid_rustc_action_inputs() {
        let (app, _directory) = test_app().await;
        let source = digest(b"pub fn widget() {}\n");
        let mut action: serde_json::Value =
            serde_json::from_slice(&rustc_action(&source, 1)).unwrap();
        action["inputs"][0]["path"] = "../src/lib.rs".into();
        let action = upload_blob(&app, &canonical(action)).await;
        let result_uri = format!(
            "/v1/action-results/{}/{}/{}",
            action.algorithm, action.hash, action.size
        );
        let body = serde_json::to_vec(&ActionResult {
            action,
            metadata: None,
            output_root: None,
            version: 1,
        })
        .unwrap();

        let response = app
            .oneshot(request("PUT", result_uri, Body::from(body)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["error"], "invalid rustc action values");
    }

    #[tokio::test]
    async fn rejects_unknown_rustc_action_fields() {
        let (app, _directory) = test_app().await;
        let source = digest(b"pub fn widget() {}\n");
        let mut action: serde_json::Value =
            serde_json::from_slice(&rustc_action(&source, 1)).unwrap();
        action["unknown"] = true.into();
        let action = upload_blob(&app, &canonical(action)).await;
        let result_uri = format!(
            "/v1/action-results/{}/{}/{}",
            action.algorithm, action.hash, action.size
        );
        let body = serde_json::to_vec(&ActionResult {
            action,
            metadata: None,
            output_root: None,
            version: 1,
        })
        .unwrap();

        let response = app
            .oneshot(request("PUT", result_uri, Body::from(body)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .starts_with("invalid rustc action:")
        );
    }

    #[tokio::test]
    async fn rejects_unknown_rustc_metadata_fields() {
        let (app, _directory) = test_app().await;
        let source = digest(b"pub fn widget() {}\n");
        let action = upload_blob(&app, &rustc_action(&source, 1)).await;
        let stdout = upload_blob(&app, b"").await;
        let stderr = upload_blob(&app, b"warning\n").await;
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&rustc_metadata(&stdout, &stderr, 1)).unwrap();
        metadata["unknown"] = true.into();
        let metadata = upload_blob(&app, &canonical(metadata)).await;
        let artifact = upload_blob(&app, b"rlib artifact").await;
        let output_root = upload_blob(&app, &output_directory(&artifact)).await;
        let result_uri = format!(
            "/v1/action-results/{}/{}/{}",
            action.algorithm, action.hash, action.size
        );
        let body = serde_json::to_vec(&ActionResult {
            action,
            metadata: Some(metadata),
            output_root: Some(output_root),
            version: 1,
        })
        .unwrap();

        let response = app
            .oneshot(request("PUT", result_uri, Body::from(body)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .starts_with("invalid rustc metadata:")
        );
    }

    #[tokio::test]
    async fn rejects_unsupported_action_schema() {
        let (app, _directory) = test_app().await;
        let action = upload_blob(&app, &task_action(2)).await;
        let result_uri = format!(
            "/v1/action-results/{}/{}/{}",
            action.algorithm, action.hash, action.size
        );
        let body = serde_json::to_vec(&ActionResult {
            action,
            metadata: None,
            output_root: None,
            version: 1,
        })
        .unwrap();
        assert_eq!(
            app.oneshot(request("PUT", result_uri, Body::from(body)))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn rejects_metadata_kind_mismatch() {
        let (app, _directory) = test_app().await;
        let action = upload_blob(&app, &task_action(1)).await;
        let metadata = upload_blob(&app, &task_metadata("rustc", 1)).await;
        let result_uri = format!(
            "/v1/action-results/{}/{}/{}",
            action.algorithm, action.hash, action.size
        );
        let body = serde_json::to_vec(&ActionResult {
            action,
            metadata: Some(metadata),
            output_root: None,
            version: 1,
        })
        .unwrap();
        assert_eq!(
            app.oneshot(request("PUT", result_uri, Body::from(body)))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn rejects_sha256_action_keys() {
        let (app, _directory) = test_app().await;
        let action = task_action(1);
        let hash = hex::encode(sha2::Sha256::digest(&action));
        let result_uri = format!("/v1/action-results/sha256/{hash}/{}", action.len());
        let body = serde_json::to_vec(&ActionResult {
            action: Digest {
                algorithm: Algorithm::Sha256,
                hash,
                size: action.len() as u64,
            },
            metadata: None,
            output_root: None,
            version: 1,
        })
        .unwrap();
        assert_eq!(
            app.oneshot(request("PUT", result_uri, Body::from(body)))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}
