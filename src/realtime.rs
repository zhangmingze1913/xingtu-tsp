use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::{
    cluster::Cluster,
    iggy_bus::IggyBus,
    models::{Alert, Command},
    walrus_bus::WalrusBus,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RealtimeEvent {
    Telemetry {
        vehicle_id: String,
        plate_no: String,
        ts: String,
        lat: f64,
        lon: f64,
        speed: f64,
        heading: f64,
        ignition: bool,
        soc: f64,
        odometer_km: f64,
        status: String,
    },
    Alert {
        alert: Alert,
    },
    Command {
        command: Command,
    },
    Fleet {
        online: i64,
        moving: i64,
        ts: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct EventEnvelope {
    origin: String,
    event: RealtimeEvent,
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<RealtimeEvent>,
    remote_tx: Option<mpsc::UnboundedSender<String>>,
    iggy: Option<IggyBus>,
    walrus: Option<WalrusBus>,
    node_id: String,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            remote_tx: None,
            iggy: None,
            walrus: None,
            node_id: "local".to_string(),
        }
    }

    /// Redis Pub/Sub event fanout used by the first cluster profile.
    pub fn with_cluster(capacity: usize, cluster: &Cluster) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        if !cluster.enabled() {
            return Self {
                tx,
                remote_tx: None,
                iggy: None,
                walrus: None,
                node_id: cluster.node_id().to_string(),
            };
        }

        let node_id = cluster.node_id().to_string();
        let (remote_tx, mut remote_rx) = mpsc::unbounded_channel::<String>();
        let publisher = cluster.clone();
        tokio::spawn(async move {
            while let Some(payload) = remote_rx.recv().await {
                publisher.publish_event(&payload).await;
            }
        });

        let local_tx = tx.clone();
        let self_node = node_id.clone();
        cluster.spawn_event_subscriber(move |payload| {
            deliver_payload(payload, &self_node, &local_tx);
        });

        Self {
            tx,
            remote_tx: Some(remote_tx),
            iggy: None,
            walrus: None,
            node_id,
        }
    }

    /// Iggy persistent event log fanout. Redis remains responsible for
    /// leader leases and gateway session ownership in this mode.
    pub fn with_cluster_and_iggy(
        capacity: usize,
        cluster: &Cluster,
        iggy: IggyBus,
        mut incoming: mpsc::UnboundedReceiver<String>,
    ) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        let node_id = cluster.node_id().to_string();
        let local_tx = tx.clone();
        let self_node = node_id.clone();
        tokio::spawn(async move {
            while let Some(payload) = incoming.recv().await {
                deliver_payload(payload, &self_node, &local_tx);
            }
        });

        Self {
            tx,
            remote_tx: None,
            iggy: Some(iggy),
            walrus: None,
            node_id,
        }
    }

    pub fn attach_walrus(&mut self, walrus: WalrusBus) {
        self.walrus = Some(walrus);
    }

    pub fn publish(&self, event: RealtimeEvent) {
        let _ = self.tx.send(event.clone());
        let envelope = EventEnvelope {
            origin: self.node_id.clone(),
            event,
        };
        let payload = match serde_json::to_string(&envelope) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, "failed to encode cluster event");
                return;
            }
        };
        if let Some(iggy) = &self.iggy {
            iggy.publish(payload.clone());
        } else if let Some(remote_tx) = &self.remote_tx {
            let _ = remote_tx.send(payload.clone());
        }
        if let Some(walrus) = &self.walrus {
            walrus.publish(payload);
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RealtimeEvent> {
        self.tx.subscribe()
    }
}

fn deliver_payload(payload: String, self_node: &str, local_tx: &broadcast::Sender<RealtimeEvent>) {
    let Ok(envelope) = serde_json::from_str::<EventEnvelope>(&payload) else {
        tracing::debug!("ignored malformed cluster event");
        return;
    };
    if envelope.origin != self_node {
        let _ = local_tx.send(envelope.event);
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(512)
    }
}
