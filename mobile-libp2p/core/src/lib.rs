use libp2p::futures::StreamExt;
use libp2p::multiaddr::Protocol;
use libp2p::{
    core::upgrade,
    gossipsub,
    identity::Keypair,
    kad::{store::MemoryStore, Behaviour as KadBehaviour, Config as KadConfig, Event as KadEvent},
    noise,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, Transport,
};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(NetworkBehaviour)]
struct MobileBehaviour {
    gossipsub: gossipsub::Behaviour,
    kad: KadBehaviour<MemoryStore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireEnvelope {
    Chat { username: String, text: String },
    Presence {
        username: String,
        status: String,
        ts: u64,
    },
    Profile { username: String },
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub username: String,
    pub bootstrap_peers: Vec<String>,
    pub topic: String,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            username: "mobile-user".to_owned(),
            bootstrap_peers: Vec::new(),
            topic: "grok-chat".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeCommand {
    SendChat { text: String },
    UpdateProfile { username: String },
    RequestPresence,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreEvent {
    Started { peer_id: String },
    ChatMessage { from: String, text: String },
    Presence { from: String, status: String, ts: u64 },
    Profile { from: String, username: String },
    Error { message: String },
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("runtime error: {0}")]
    Runtime(String),
}

pub struct MobileCore {
    config: NodeConfig,
}

impl MobileCore {
    pub fn new(config: NodeConfig) -> Result<Self, CoreError> {
        if config.topic.trim().is_empty() {
            return Err(CoreError::InvalidConfig("topic must not be empty".to_owned()));
        }

        Ok(Self { config })
    }

    pub fn config(&self) -> &NodeConfig {
        &self.config
    }

    pub fn run(
        &self,
        command_rx: Receiver<BridgeCommand>,
        event_tx: Sender<CoreEvent>,
    ) -> Result<(), CoreError> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|err| CoreError::Runtime(format!("tokio init failed: {err}")))?;

        runtime.block_on(self.run_async(command_rx, event_tx))
    }

    async fn run_async(
        &self,
        command_rx: Receiver<BridgeCommand>,
        event_tx: Sender<CoreEvent>,
    ) -> Result<(), CoreError> {
        let id_keys = Keypair::generate_ed25519();
        let local_peer_id = PeerId::from(id_keys.public());

        let transport = tcp::tokio::Transport::default()
            .upgrade(upgrade::Version::V1)
            .authenticate(
                noise::Config::new(&id_keys)
                    .map_err(|err| CoreError::Runtime(format!("noise init failed: {err}")))?,
            )
            .multiplex(yamux::Config::default())
            .boxed();

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_millis(500))
            .validation_mode(gossipsub::ValidationMode::Permissive)
            .flood_publish(true)
            .build()
            .map_err(|err| CoreError::Runtime(format!("gossipsub config failed: {err}")))?;

        let mut gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(id_keys),
            gossipsub_config,
        )
        .map_err(|err| CoreError::Runtime(format!("gossipsub init failed: {err}")))?;

        let topic = gossipsub::IdentTopic::new(self.config.topic.clone());
        gossipsub
            .subscribe(&topic)
            .map_err(|err| CoreError::Runtime(format!("subscribe failed: {err}")))?;

        let mut kad_config = KadConfig::default();
        kad_config.set_query_timeout(Duration::from_secs(15));
        let store = MemoryStore::new(local_peer_id);
        let mut kad = KadBehaviour::with_config(local_peer_id, store, kad_config);

        for bootstrap in &self.config.bootstrap_peers {
            let Ok(addr) = bootstrap.parse::<Multiaddr>() else {
                let _ = event_tx.send(CoreEvent::Error {
                    message: format!("invalid bootstrap address: {bootstrap}"),
                });
                continue;
            };

            let mut found_peer: Option<PeerId> = None;
            for component in addr.iter() {
                if let Protocol::P2p(peer_id) = component {
                    found_peer = Some(peer_id);
                }
            }

            if let Some(peer_id) = found_peer {
                kad.add_address(&peer_id, addr.clone());
            }
        }

        let mut swarm = Swarm::new(
            transport,
            MobileBehaviour { gossipsub, kad },
            local_peer_id,
            libp2p::swarm::Config::with_tokio_executor(),
        );

        swarm
            .listen_on(
                "/ip4/0.0.0.0/tcp/0"
                    .parse()
                    .map_err(|err| CoreError::Runtime(format!("listen addr parse failed: {err}")))?,
            )
            .map_err(|err| CoreError::Runtime(format!("listen failed: {err}")))?;

        for bootstrap in &self.config.bootstrap_peers {
            if let Ok(addr) = bootstrap.parse::<Multiaddr>() {
                let _ = swarm.dial(addr);
            }
        }

        let _ = swarm.behaviour_mut().kad.bootstrap();

        let _ = event_tx.send(CoreEvent::Started {
            peer_id: local_peer_id.to_string(),
        });

        let profile = WireEnvelope::Profile {
            username: self.config.username.clone(),
        };
        if let Ok(payload) = serde_json::to_vec(&profile) {
            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), payload);
        }

        let mut presence_tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            while let Ok(command) = command_rx.try_recv() {
                match command {
                    BridgeCommand::SendChat { text } => {
                        if text.trim().is_empty() {
                            continue;
                        }

                        let envelope = WireEnvelope::Chat {
                            username: self.config.username.clone(),
                            text,
                        };

                        match serde_json::to_vec(&envelope) {
                            Ok(payload) => {
                                if let Err(err) =
                                    swarm.behaviour_mut().gossipsub.publish(topic.clone(), payload)
                                {
                                    let _ = event_tx.send(CoreEvent::Error {
                                        message: format!("publish failed: {err:?}"),
                                    });
                                }
                            }
                            Err(err) => {
                                let _ = event_tx.send(CoreEvent::Error {
                                    message: format!("serialize chat failed: {err}"),
                                });
                            }
                        }
                    }
                    BridgeCommand::UpdateProfile { username } => {
                        let envelope = WireEnvelope::Profile {
                            username: username.clone(),
                        };

                        if let Ok(payload) = serde_json::to_vec(&envelope) {
                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), payload);
                        }

                        let _ = event_tx.send(CoreEvent::Profile {
                            from: local_peer_id.to_string(),
                            username,
                        });
                    }
                    BridgeCommand::RequestPresence => {
                        let presence = WireEnvelope::Presence {
                            username: self.config.username.clone(),
                            status: "online".to_owned(),
                            ts: now_ts(),
                        };
                        if let Ok(payload) = serde_json::to_vec(&presence) {
                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), payload);
                        }
                    }
                    BridgeCommand::Shutdown => return Ok(()),
                }
            }

            tokio::select! {
                _ = presence_tick.tick() => {
                    let presence = WireEnvelope::Presence {
                        username: self.config.username.clone(),
                        status: "online".to_owned(),
                        ts: now_ts(),
                    };

                    if let Ok(payload) = serde_json::to_vec(&presence) {
                        let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), payload);
                    }
                }
                event = swarm.select_next_some() => {
                    match event {
                        SwarmEvent::NewListenAddr { address, .. } => {
                            let _ = event_tx.send(CoreEvent::Presence {
                                from: local_peer_id.to_string(),
                                status: format!("listening:{address}"),
                                ts: now_ts(),
                            });
                        }
                        SwarmEvent::Behaviour(MobileBehaviourEvent::Gossipsub(gossipsub::Event::Message { propagation_source, message, .. })) => {
                            if let Ok(envelope) = serde_json::from_slice::<WireEnvelope>(&message.data) {
                                match envelope {
                                    WireEnvelope::Chat { username, text } => {
                                        let _ = event_tx.send(CoreEvent::ChatMessage {
                                            from: username,
                                            text,
                                        });
                                    }
                                    WireEnvelope::Presence { username, status, ts } => {
                                        let _ = event_tx.send(CoreEvent::Presence {
                                            from: username,
                                            status,
                                            ts,
                                        });
                                    }
                                    WireEnvelope::Profile { username } => {
                                        let _ = event_tx.send(CoreEvent::Profile {
                                            from: propagation_source.to_string(),
                                            username,
                                        });
                                    }
                                }
                            }
                        }
                        SwarmEvent::Behaviour(MobileBehaviourEvent::Kad(KadEvent::OutboundQueryProgressed { result, .. })) => {
                            let _ = event_tx.send(CoreEvent::Presence {
                                from: local_peer_id.to_string(),
                                status: format!("kad:{result:?}"),
                                ts: now_ts(),
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
