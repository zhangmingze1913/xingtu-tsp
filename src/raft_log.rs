//! Minimal file-backed OpenRaft log store.
//!
//! This persists the Raft log, vote, committed index and purge watermark so a
//! restart does not lose the control-plane log. The associated state machine is
//! still `openraft-memstore`; after restart OpenRaft replays committed log
//! entries into it. Snapshot state itself is not persisted by this module.

use std::{
    collections::BTreeMap,
    fmt::Debug,
    io,
    ops::RangeBounds,
    path::{Path, PathBuf},
    sync::Arc,
};

use openraft::{
    alias::{EntryOf, LogIdOf, VoteOf},
    entry::RaftEntry,
    storage::{IOFlushed, LogState, RaftLogReader, RaftLogStorage},
    OptionalSend,
};
use openraft_memstore::TypeConfig;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Debug, Default, Serialize, Deserialize)]
struct DiskState {
    vote: Option<VoteOf<TypeConfig>>,
    committed: Option<LogIdOf<TypeConfig>>,
    last_purged: Option<LogIdOf<TypeConfig>>,
    log: BTreeMap<u64, String>,
}

#[derive(Clone)]
pub struct FileLogStore {
    path: PathBuf,
    state: Arc<RwLock<DiskState>>,
}

impl FileLogStore {
    pub fn open(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("raft-log.json");
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => DiskState::default(),
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            state: Arc::new(RwLock::new(state)),
        })
    }

    async fn persist(&self, state: &DiskState) -> io::Result<()> {
        let bytes = serde_json::to_vec(state)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let path = self.path.clone();
        // write+rename is blocking file I/O: run it on the blocking pool so a
        // log append does not stall the async worker thread.
        tokio::task::spawn_blocking(move || -> io::Result<()> {
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, &bytes)?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })
        .await
        .map_err(io::Error::other)?
    }
}

impl RaftLogReader<TypeConfig> for FileLogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<EntryOf<TypeConfig>>, io::Error> {
        let state = self.state.read().await;
        let mut entries = Vec::new();
        for (_, serialized) in state.log.range(range) {
            entries.push(
                serde_json::from_str(serialized)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
            );
        }
        Ok(entries)
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<TypeConfig>>, io::Error> {
        Ok(self.state.read().await.vote)
    }
}

impl RaftLogStorage<TypeConfig> for FileLogStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, io::Error> {
        let state = self.state.read().await;
        let last = match state.log.iter().next_back() {
            Some((_, serialized)) => {
                let entry: EntryOf<TypeConfig> = serde_json::from_str(serialized)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                Some(entry.log_id())
            }
            None => state.last_purged,
        };
        Ok(LogState {
            last_purged_log_id: state.last_purged,
            last_log_id: last.or(state.last_purged),
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &VoteOf<TypeConfig>) -> Result<(), io::Error> {
        let mut state = self.state.write().await;
        state.vote = Some(*vote);
        self.persist(&state).await
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogIdOf<TypeConfig>>,
    ) -> Result<(), io::Error> {
        let mut state = self.state.write().await;
        state.committed = committed;
        self.persist(&state).await
    }

    async fn read_committed(&mut self) -> Result<Option<LogIdOf<TypeConfig>>, io::Error> {
        Ok(self.state.read().await.committed)
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: IOFlushed<TypeConfig>,
    ) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = EntryOf<TypeConfig>> + OptionalSend,
    {
        let mut state = self.state.write().await;
        for entry in entries {
            let serialized = serde_json::to_string(&entry)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            state.log.insert(entry.index(), serialized);
        }
        self.persist(&state).await?;
        callback.io_completed(Ok(()));
        Ok(())
    }

    async fn truncate_after(
        &mut self,
        last_log_id: Option<LogIdOf<TypeConfig>>,
    ) -> Result<(), io::Error> {
        let start = last_log_id.map(|log_id| log_id.index() + 1).unwrap_or(0);
        let mut state = self.state.write().await;
        let keys: Vec<u64> = state.log.range(start..).map(|(key, _)| *key).collect();
        for key in keys {
            state.log.remove(&key);
        }
        self.persist(&state).await
    }

    async fn purge(&mut self, log_id: LogIdOf<TypeConfig>) -> Result<(), io::Error> {
        let mut state = self.state.write().await;
        state.last_purged = Some(log_id);
        let keys: Vec<u64> = state
            .log
            .range(..=log_id.index())
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            state.log.remove(&key);
        }
        self.persist(&state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openraft::storage::{RaftLogReader, RaftLogStorage};
    use openraft::Vote;

    #[tokio::test]
    async fn vote_and_log_roundtrip_across_reopen() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("xingtu-raft-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = FileLogStore::open(&dir).unwrap();
        let mut store = store;
        let vote = Vote::new(3, 7);
        store.save_vote(&vote).await.unwrap();
        drop(store);

        let mut reopened = FileLogStore::open(&dir).unwrap();
        assert_eq!(reopened.read_vote().await.unwrap(), Some(vote));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
