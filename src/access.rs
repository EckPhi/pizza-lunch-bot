use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

pub const DENIED: &str = "Pizza Lunch access is restricted.";

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Rule {
    #[default]
    Default,
    Allow,
    Block,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Entry {
    name: String,
    rule: Rule,
    groups: BTreeSet<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Directory {
    groups: BTreeMap<i64, Entry>,
    users: BTreeMap<i64, Entry>,
    groups_allowlist: bool,
    users_allowlist: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Group,
    User,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Change {
    SetRule { kind: Kind, id: String, rule: Rule },
    SetAllowlist { kind: Kind, enabled: bool },
}

pub struct Access {
    path: PathBuf,
    directory: Directory,
}

impl Access {
    pub fn load(path: PathBuf) -> Result<Self> {
        let directory = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Directory::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, directory })
    }

    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    fn commit(&mut self, directory: Directory) -> Result<()> {
        let mut temp = self.path.as_os_str().to_owned();
        temp.push(".tmp");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        serde_json::to_writer(&mut file, &directory)?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(temp, &self.path)?;
        self.directory = directory;
        Ok(())
    }

    pub fn allows_group(&self, group: i64) -> bool {
        allows(
            self.directory.groups.get(&group),
            self.directory.groups_allowlist,
        )
    }

    pub fn allows(&self, group: Option<i64>, user: Option<i64>) -> bool {
        group.is_none_or(|id| self.allows_group(id))
            && user.is_some_and(|id| {
                allows(
                    self.directory.users.get(&id),
                    self.directory.users_allowlist,
                )
            })
    }

    pub fn observe(&mut self, group: Option<(i64, &str)>, user: Option<(i64, &str)>) -> Result<()> {
        let group_id = group.map(|(id, _)| id.to_string());
        let new_group = group.is_some_and(|(id, name)| {
            self.directory
                .groups
                .get(&id)
                .is_none_or(|entry| !name.is_empty() && entry.name != name)
        });
        let new_user = user.is_some_and(|(id, name)| {
            self.directory.users.get(&id).is_none_or(|entry| {
                (!name.is_empty() && entry.name != name)
                    || group_id
                        .as_ref()
                        .is_some_and(|id| !entry.groups.contains(id))
            })
        });
        if !new_group && !new_user {
            return Ok(());
        }
        let mut directory = self.directory.clone();
        if let Some((id, name)) = group {
            let entry = directory.groups.entry(id).or_default();
            if !name.is_empty() {
                entry.name = name.into();
            }
        }
        if let Some((id, name)) = user {
            let entry = directory.users.entry(id).or_default();
            if !name.is_empty() {
                entry.name = name.into();
            }
            entry.groups.extend(group_id);
        }
        self.commit(directory)
    }

    pub fn apply(&mut self, change: Change) -> Result<bool> {
        let mut directory = self.directory.clone();
        match change {
            Change::SetRule { kind, id, rule } => {
                let entries = match kind {
                    Kind::Group => &mut directory.groups,
                    Kind::User => &mut directory.users,
                };
                let Some(entry) = id.parse::<i64>().ok().and_then(|id| entries.get_mut(&id)) else {
                    return Ok(false);
                };
                entry.rule = rule;
            }
            Change::SetAllowlist { kind, enabled } => match kind {
                Kind::Group => directory.groups_allowlist = enabled,
                Kind::User => directory.users_allowlist = enabled,
            },
        }
        self.commit(directory)?;
        Ok(true)
    }
}

fn allows(entry: Option<&Entry>, allowlist: bool) -> bool {
    match entry.map_or(Rule::Default, |entry| entry.rule) {
        Rule::Default => !allowlist,
        Rule::Allow => true,
        Rule::Block => false,
    }
}

pub fn group(chat: &Value) -> Option<(i64, &str)> {
    let id = chat["id"].as_i64()?;
    matches!(chat["type"].as_str(), Some("group" | "supergroup"))
        .then_some((id, chat["title"].as_str().unwrap_or_default()))
}

pub fn user_id(user: &Value) -> Option<i64> {
    user["id"]
        .as_i64()
        .filter(|id| *id > 0 && user["is_bot"] != true)
}
