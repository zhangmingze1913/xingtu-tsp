//! File-backed OpenRaft state machine.
//!
//! Persists the small control-plane state (`last_applied`, membership and
//! client statuses) plus snapshots. The state is intentionally tiny; business
//! data remains in PostgreSQL/MySQL.

use std::{
    io::{self, Cursor},
    path::{Path, PathBuf},
    sync::Arc,
};

use futures_util::{Stream, TryStreamExt};
use openraft::{
    alias::{LogIdOf, SnapshotMetaOf, SnapshotOf, StoredMembershipOf},
    storage::{EntryResponder, RaftSnapshotBuilder, RaftStateMachine},
    EntryPayload, OptionalSend,
};
use openraft_legacy::network_v1::SnapshotReceiverFactory;
use openraft_memstore::{ClientResponse, MemStoreSnapshot, MemStoreStateMachine, TypeConfig};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct FileStateMachine {
    state_path: PathBuf,
    sm: Arc<RwLock<MemStoreStateMachine>>,
    current_snapshot: Arc<RwLock<Option<MemStoreSnapshot>>>,
}

#[derive(Clone)]
pub struct FileSnapshotBuilder(FileStateMachine);

impl FileStateMachine {
    pub fn open(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let state_path = dir.join("raft-state.json");
        let sm = match std::fs::read(&state_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                MemStoreStateMachine::default()
            }
            Err(error) => return Err(error),
        };
        Ok(Self {
            state_path,
            sm: Arc::new(RwLock::new(sm)),
            current_snapshot: Arc::new(RwLock::new(None)),
        })
    }

    async fn persist(&self, sm: &MemStoreStateMachine) -> io::Result<()> {
        let bytes = serde_json::to_vec(sm)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let tmp = self.state_path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &self.state_path)?;
        Ok(())
    }
}

impl RaftSnapshotBuilder<TypeConfig> for FileSnapshotBuilder {
    type SnapshotData = Cursor<Vec<u8>>;

    async fn build_snapshot(
        &mut self,
    ) -> Result<SnapshotOf<TypeConfig, Cursor<Vec<u8>>>, io::Error> {
        let sm = self.0.sm.read().await;
        let data = serde_json::to_vec(&*sm)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let meta = SnapshotMetaOf::<TypeConfig> {
            last_log_id: sm.last_applied_log,
            last_membership: sm.last_membership.clone(),
        };
        *self.0.current_snapshot.write().await = Some(MemStoreSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        });
        Ok(SnapshotOf::<TypeConfig, Cursor<Vec<u8>>> {
            meta,
            snapshot: Cursor::new(data),
        })
    }
}

impl RaftStateMachine<TypeConfig> for FileStateMachine {
    type SnapshotData = Cursor<Vec<u8>>;
    type SnapshotBuilder = FileSnapshotBuilder;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogIdOf<TypeConfig>>, StoredMembershipOf<TypeConfig>), io::Error> {
        let sm = self.sm.read().await;
        Ok((sm.last_applied_log, sm.last_membership.clone()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where
        Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend,
    {
        let mut sm = self.sm.write().await;
        while let Some((entry, responder)) = entries.try_next().await? {
            sm.last_applied_log = Some(entry.log_id);
            let response = match entry.payload {
                EntryPayload::Blank => ClientResponse(None),
                EntryPayload::Normal(ref data) => ClientResponse(
                    sm.client_status
                        .insert(data.client.clone(), data.status.clone()),
                ),
                EntryPayload::Membership(ref membership) => {
                    sm.last_membership = StoredMembershipOf::<TypeConfig>::new(
                        Some(entry.log_id),
                        membership.clone(),
                    );
                    ClientResponse(None)
                }
            };
            if let Some(responder) = responder {
                responder.send(response);
            }
        }
        self.persist(&sm).await
    }

    async fn try_create_snapshot_builder(&mut self, _force: bool) -> Option<Self::SnapshotBuilder> {
        Some(FileSnapshotBuilder(self.clone()))
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        FileSnapshotBuilder(self.clone())
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMetaOf<TypeConfig>,
        snapshot: Self::SnapshotData,
    ) -> Result<(), io::Error> {
        let data = snapshot.into_inner();
        let sm: MemStoreStateMachine = serde_json::from_slice(&data)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        {
            let mut current = self.sm.write().await;
            *current = sm.clone();
            self.persist(&current).await?;
        }
        *self.current_snapshot.write().await = Some(MemStoreSnapshot {
            meta: meta.clone(),
            data,
        });
        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<SnapshotOf<TypeConfig, Self::SnapshotData>>, io::Error> {
        Ok(self
            .current_snapshot
            .read()
            .await
            .as_ref()
            .map(|snapshot| SnapshotOf::<TypeConfig, Cursor<Vec<u8>>> {
                meta: snapshot.meta.clone(),
                snapshot: Cursor::new(snapshot.data.clone()),
            }))
    }
}

impl SnapshotReceiverFactory<TypeConfig> for FileStateMachine {
    type SnapshotReceiver = Cursor<Vec<u8>>;

    async fn begin_receiving_snapshot(&mut self) -> Result<Self::SnapshotReceiver, io::Error> {
        Ok(Cursor::new(Vec::new()))
    }
}
