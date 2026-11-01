use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Session {
    pub id: String,
    pub started: DateTime<Utc>,
    pub ended: DateTime<Utc>,
    pub duration_ms: u64,
    pub command: Vec<String>,
    pub name: Option<String>,
    pub exit_code: Option<i32>,
    pub elevated: bool,
    pub host: String,
    pub manifest: Manifest,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Manifest {
    pub files: FileDiff,
    pub registry: RegistryDiff,
    pub services: ServiceDiff,
    pub tasks: TaskDiff,
    pub drivers: DriverDiff,
    pub network: Vec<NetEvent>,
    pub processes: Vec<ProcEvent>,
    pub warnings: Vec<String>,
}

impl Manifest {
    pub fn total_changes(&self) -> usize {
        self.files.created.len()
            + self.files.modified.len()
            + self.files.deleted.len()
            + self.registry.created_keys.len()
            + self.registry.created_values.len()
            + self.registry.modified_values.len()
            + self.registry.deleted_values.len()
            + self.services.created.len()
            + self.services.modified.len()
            + self.tasks.created.len()
            + self.drivers.created.len()
            + self.network.len()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct FileDiff {
    pub created: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
    pub deleted: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RegistryDiff {
    pub created_keys: Vec<String>,
    pub created_values: Vec<RegValue>,
    pub modified_values: Vec<RegChange>,
    pub deleted_values: Vec<RegValue>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegKind {
    Sz,
    ExpandSz,
    Dword,
    Qword,
    MultiSz,
    Binary,
}

impl RegKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RegKind::Sz => "REG_SZ",
            RegKind::ExpandSz => "REG_EXPAND_SZ",
            RegKind::Dword => "REG_DWORD",
            RegKind::Qword => "REG_QWORD",
            RegKind::MultiSz => "REG_MULTI_SZ",
            RegKind::Binary => "REG_BINARY",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RegValue {
    pub key: String,
    pub name: String,
    pub kind: RegKind,
    pub data: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RegChange {
    pub key: String,
    pub name: String,
    pub kind: RegKind,
    pub old: String,
    pub new: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ServiceDiff {
    pub created: Vec<ServiceInfo>,
    pub modified: Vec<ServiceChange>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServiceInfo {
    pub name: String,
    pub display: String,
    pub image: String,
    pub start: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServiceChange {
    pub name: String,
    pub field: String,
    pub old: String,
    pub new: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TaskDiff {
    pub created: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DriverDiff {
    pub created: Vec<ServiceInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetEvent {
    pub proto: String,
    pub local: String,
    pub remote: String,
    pub pid: u32,
    pub process: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProcEvent {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    pub exe: Option<PathBuf>,
}

/// Registry state: key path -> value name -> (kind, encoded data).
pub type RegSnapshot = BTreeMap<String, BTreeMap<String, (RegKind, String)>>;

/// Task directory state: relative task path -> (size, modified unix seconds).
pub type TaskSnapshot = BTreeMap<String, (u64, u64)>;
