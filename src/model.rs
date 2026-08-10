use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Digest {
    pub algorithm: Algorithm,
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    Blake3,
    Sha256,
}

impl fmt::Display for Algorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Blake3 => "blake3",
            Self::Sha256 => "sha256",
        })
    }
}

impl std::str::FromStr for Algorithm {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "blake3" => Ok(Self::Blake3),
            "sha256" => Ok(Self::Sha256),
            _ => Err("unsupported digest algorithm"),
        }
    }
}

impl Digest {
    pub fn validate(&self) -> bool {
        self.hash.len() == 64
            && self
                .hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.algorithm, self.hash, self.size)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskActionManifest {
    pub predictions: Vec<TaskActionPrediction>,
    pub task: String,
    pub version: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskActionPrediction {
    pub action: Digest,
    pub adapter: String,
    pub invocation: Digest,
    pub payload: String,
}

#[derive(Serialize)]
struct TaskActionManifestSelector<'a> {
    kind: &'static str,
    task: &'a str,
    version: u8,
}

impl TaskActionManifest {
    pub fn validate(&self) -> bool {
        let mut invocations = HashSet::new();
        self.version == 1
            && valid_task_identity(&self.task)
            && self.predictions.len() <= 16 * 1024
            && self.predictions.iter().all(|prediction| {
                prediction.validate() && invocations.insert(&prediction.invocation)
            })
    }

    pub fn selector_digest(&self) -> Digest {
        let selector = serde_json::to_vec(&TaskActionManifestSelector {
            kind: "task_action_manifest",
            task: &self.task,
            version: 1,
        })
        .expect("manifest selector must serialize");
        Digest {
            algorithm: Algorithm::Blake3,
            hash: blake3::hash(&selector).to_hex().to_string(),
            size: selector.len() as u64,
        }
    }
}

impl TaskActionPrediction {
    fn validate(&self) -> bool {
        self.action.algorithm == Algorithm::Blake3
            && self.action.validate()
            && self.invocation.algorithm == Algorithm::Blake3
            && self.invocation.validate()
            && !self.adapter.is_empty()
            && self
                .adapter
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            && self.payload.len() <= 256 * 1024
            && serde_json::from_str::<serde_json::Value>(&self.payload).is_ok()
    }
}

fn valid_task_identity(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionResult {
    pub action: Digest,
    pub metadata: Option<Digest>,
    pub output_root: Option<Digest>,
    pub version: u8,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAction {
    pub version: u8,
    pub kind: String,
    pub task: String,
    pub phase: TaskPhase,
    pub run: Vec<TaskRunEntry>,
    pub args: Vec<String>,
    pub shell: Option<String>,
    pub outputs: Vec<String>,
    pub root: String,
    pub source_hash: String,
    #[serde(default)]
    pub dependency_keys: Vec<String>,
    pub environment: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub command_inputs: Vec<TaskCommandInput>,
    pub vars: BTreeMap<String, String>,
    pub tools: Vec<String>,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPhase {
    Normal,
    Post,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TaskRunEntry {
    Script(String),
    Single(TaskRunSingle),
    Group(TaskRunGroup),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRunSingle {
    pub task: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRunGroup {
    pub tasks: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCommandInput {
    pub command: String,
    pub stdout_hash: String,
    pub stderr_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskMetadata {
    pub version: u8,
    pub kind: String,
    pub task_identity: String,
    pub roots: Vec<String>,
    pub output: Vec<TaskOutput>,
    pub restored_bytes: u64,
    pub execution_duration_ns: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskOutput {
    pub stream: TaskOutputStream,
    pub line: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustcAction {
    pub version: u8,
    pub kind: String,
    pub adapter_version: u8,
    pub compiler: RustcCompiler,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, Option<String>>,
    pub inputs: Vec<RustcInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustcCompiler {
    pub toolchain: String,
    pub rustc_version: String,
    pub host: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustcInput {
    pub path: String,
    /// Identifies local input content for the action key. This is not a CAS
    /// reference: compiler source inputs are never uploaded to the service.
    pub digest: Digest,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustcMetadata {
    pub version: u8,
    pub kind: String,
    pub stdout: Digest,
    pub stderr: Digest,
}

fn valid_string(value: &str) -> bool {
    !value.contains('\0')
}

fn valid_strings(values: &[String]) -> bool {
    values.iter().all(|value| valid_string(value))
}

fn valid_string_map(values: &BTreeMap<String, String>) -> bool {
    values
        .iter()
        .all(|(key, value)| valid_string(key) && valid_string(value))
}

impl TaskAction {
    pub fn validate(&self) -> bool {
        self.version == 1
            && self.kind == "task"
            && valid_string(&self.task)
            && matches!(self.phase, TaskPhase::Normal | TaskPhase::Post)
            && self.run.iter().all(TaskRunEntry::validate)
            && valid_strings(&self.args)
            && self.shell.as_deref().is_none_or(valid_string)
            && valid_strings(&self.outputs)
            && valid_string(&self.root)
            && valid_string(&self.source_hash)
            && valid_strings(&self.dependency_keys)
            && self
                .environment
                .iter()
                .all(|(key, value)| valid_string(key) && value.as_deref().is_none_or(valid_string))
            && self.command_inputs.iter().all(TaskCommandInput::validate)
            && valid_string_map(&self.vars)
            && valid_strings(&self.tools)
            && valid_string(&self.os)
            && valid_string(&self.arch)
    }
}

impl TaskRunEntry {
    fn validate(&self) -> bool {
        match self {
            Self::Script(script) => valid_string(script),
            Self::Single(entry) => {
                valid_string(&entry.task)
                    && valid_strings(&entry.args)
                    && valid_string_map(&entry.env)
            }
            Self::Group(entry) => valid_strings(&entry.tasks),
        }
    }
}

impl TaskCommandInput {
    fn validate(&self) -> bool {
        valid_string(&self.command)
            && valid_string(&self.stdout_hash)
            && valid_string(&self.stderr_hash)
    }
}

impl TaskMetadata {
    pub fn validate(&self) -> bool {
        // Serde's u64 deserialization is the schema validation for these numeric fields.
        let _ = (self.restored_bytes, self.execution_duration_ns);
        self.version == 1
            && self.kind == "task"
            && valid_string(&self.task_identity)
            && valid_strings(&self.roots)
            && self.output.iter().all(TaskOutput::validate)
    }
}

impl TaskOutput {
    fn validate(&self) -> bool {
        matches!(
            self.stream,
            TaskOutputStream::Stdout | TaskOutputStream::Stderr
        ) && valid_string(&self.line)
    }
}

impl RustcAction {
    pub fn validate(&self) -> bool {
        let mut input_paths = HashSet::new();
        self.version == 1
            && self.kind == "rustc"
            && self.adapter_version > 0
            && self.compiler.validate()
            && valid_strings(&self.arguments)
            && self.environment.iter().all(|(key, value)| {
                !key.is_empty() && valid_string(key) && value.as_deref().is_none_or(valid_string)
            })
            && !self.inputs.is_empty()
            && self
                .inputs
                .iter()
                .all(|input| input.validate() && input_paths.insert(&input.path))
    }
}

impl RustcCompiler {
    fn validate(&self) -> bool {
        [&self.toolchain, &self.rustc_version, &self.host]
            .into_iter()
            .all(|value| !value.is_empty() && valid_string(value))
    }
}

impl RustcInput {
    fn validate(&self) -> bool {
        valid_normalized_path(&self.path) && self.digest.validate()
    }
}

impl RustcMetadata {
    pub fn validate(&self) -> bool {
        self.version == 1
            && self.kind == "rustc"
            && self.stdout.validate()
            && self.stderr.validate()
    }
}

fn valid_normalized_path(path: &str) -> bool {
    let Some((placeholder, suffix)) = path
        .strip_prefix("${")
        .and_then(|path| path.split_once('}'))
    else {
        return false;
    };
    if placeholder.is_empty()
        || !placeholder
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return false;
    }
    suffix.is_empty()
        || suffix.strip_prefix('/').is_some_and(|suffix| {
            !suffix.is_empty()
                && !suffix.contains(['\\', '\0'])
                && suffix
                    .split('/')
                    .all(|component| !component.is_empty() && component != "." && component != "..")
        })
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Directory {
    pub directories: Vec<DirectoryNode>,
    pub files: Vec<FileNode>,
    pub symlinks: Vec<SymlinkNode>,
    pub version: u8,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryNode {
    pub digest: Digest,
    pub mode: u32,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileNode {
    pub digest: Digest,
    pub executable: bool,
    pub mode: u32,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymlinkNode {
    pub mode: u32,
    pub name: String,
    pub target: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_lowercase_hex_digests() {
        let valid = Digest {
            algorithm: Algorithm::Blake3,
            hash: "a".repeat(64),
            size: 42,
        };
        assert!(valid.validate());
        let invalid = Digest {
            hash: "A".repeat(64),
            ..valid
        };
        assert!(!invalid.validate());
    }
}
