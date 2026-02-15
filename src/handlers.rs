use crate::logging::log_event;
use crate::p2p::{log_mesh_state, MyBehaviour, MyBehaviourEvent};
use libp2p::{
    gossipsub, mdns,
    swarm::{Swarm, SwarmEvent},
    PeerId,
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

pub struct DialState {
    pending_dials: HashSet<PeerId>,
    last_dial_attempt: HashMap<PeerId, Instant>,
    min_dial_interval: Duration,
}

impl DialState {
    pub fn new() -> Self {
        Self {
            pending_dials: HashSet::new(),
            last_dial_attempt: HashMap::new(),
            min_dial_interval: Duration::from_secs(10),
        }
    }
}

pub async fn handle_stdin_line(
    line: String,
    swarm: &mut Swarm<MyBehaviour>,
    chat_topic: &gossipsub::IdentTopic,
    log_tx: &UnboundedSender<String>,
) {
    log_event(log_tx, format!("Считываем ввод: {}", line));

    let topic_hash = chat_topic.hash();
    let (mesh_peer_count, known_peer_count) = {
        let behaviour = swarm.behaviour();
        (
            behaviour.gossipsub.mesh_peers(&topic_hash).count(),
            behaviour.gossipsub.all_peers().count(),
        )
    };

    log_event(
        log_tx,
        format!(
            "Перед publish: mesh={} known={} для {:?}",
            mesh_peer_count, known_peer_count, topic_hash
        ),
    );

    let mut mesh_peer_count = mesh_peer_count;
    if mesh_peer_count == 0 && known_peer_count > 0 {
        log_event(
            log_tx,
            "Mesh=0 при известных пирах, ждём heartbeat перед publish".to_owned(),
        );
        tokio::time::sleep(Duration::from_millis(350)).await;
        mesh_peer_count = swarm.behaviour().gossipsub.mesh_peers(&topic_hash).count();
        log_event(
            log_tx,
            format!(
                "После ожидания перед publish: mesh={} known={}",
                mesh_peer_count, known_peer_count
            ),
        );
    }

    if mesh_peer_count == 0 {
        println!(
            "⚠️ В mesh ещё нет подписанных пиров для {:?}. Публикация, скорее всего, не дойдёт.",
            topic_hash
        );
    }

    let result = swarm
        .behaviour_mut()
        .gossipsub
        .publish(chat_topic.clone(), line.as_bytes());

    match result {
        Ok(_) => {
            println!("📤 Отправлено!");
            log_event(log_tx, format!("Отправлено сообщение: {}", line));
        }
        Err(gossipsub::PublishError::InsufficientPeers) => {
            println!("📡 Мало пиров для Mesh, пробуем принудительную активацию...");
            log_event(
                log_tx,
                format!(
                    "InsufficientPeers (mesh={}). Добавляем явных пиров.",
                    mesh_peer_count
                ),
            );

            let peers: Vec<PeerId> = swarm
                .behaviour_mut()
                .gossipsub
                .all_peers()
                .map(|p| *p.0)
                .collect();
            for peer_id in &peers {
                swarm.behaviour_mut().gossipsub.add_explicit_peer(peer_id);
            }

            log_mesh_state(log_tx, swarm, &chat_topic.hash(), "После добавления явных пиров");

            if let Err(err) = swarm
                .behaviour_mut()
                .gossipsub
                .publish(chat_topic.clone(), line.as_bytes())
            {
                println!("⚠️ Даже после активации: {:?}. Жди прогрева...", err);
                log_event(log_tx, format!("Повторная отправка не удалась: {:?}", err));
            } else {
                println!("📤 Протолкнули через Flood!");
                log_event(log_tx, "Удачная Flood-публикация".to_owned());
            }
        }
        Err(err) => {
            println!("❌ Ошибка: {:?}", err);
            log_event(log_tx, format!("Ошибка publish: {:?}", err));
        }
    }
}

pub fn handle_swarm_event(
    event: SwarmEvent<MyBehaviourEvent>,
    swarm: &mut Swarm<MyBehaviour>,
    chat_topic: &gossipsub::IdentTopic,
    log_tx: &UnboundedSender<String>,
    dial_state: &mut DialState,
) {
    match event {
        SwarmEvent::NewListenAddr { address, .. } => {
            println!("🎧 Слушаю на: {}", address);
            log_event(log_tx, format!("Новый адрес прослушивания: {}", address));
        }
        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            println!("✅ Есть контакт с: {}", peer_id);
            log_event(log_tx, format!("Контакт с {}", peer_id));
            dial_state.pending_dials.remove(&peer_id);
            log_mesh_state(log_tx, swarm, &chat_topic.hash(), "После соединения");
        }
        SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
            log_event(
                log_tx,
                format!("Соединение закрыто с {}: {:?}", peer_id, cause),
            );
        }
        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            if let Some(peer_id) = peer_id {
                dial_state.pending_dials.remove(&peer_id);
                log_event(
                    log_tx,
                    format!("Ошибка исходящего соединения с {}: {:?}", peer_id, error),
                );
            } else {
                log_event(
                    log_tx,
                    format!("Ошибка исходящего соединения без peer_id: {:?}", error),
                );
            }
        }
        SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(event)) => match event {
            gossipsub::Event::Message {
                propagation_source,
                message,
                ..
            } => {
                let text = String::from_utf8_lossy(&message.data);
                println!("📩 Сообщение от {}: {}", propagation_source, text);
                log_event(
                    log_tx,
                    format!("Получено сообщение от {}: {}", propagation_source, text),
                );
                log_mesh_state(log_tx, swarm, &chat_topic.hash(), "После получения сообщения");
            }
            gossipsub::Event::Subscribed {
                peer_id,
                topic: event_topic,
            } => {
                println!("🛰️ Подписка: {} -> {}", peer_id, event_topic);
                log_event(
                    log_tx,
                    format!("Подписка на {:?} от {}", event_topic, peer_id),
                );
                log_mesh_state(log_tx, swarm, &event_topic, "После подписки");
            }
            gossipsub::Event::Unsubscribed { peer_id, topic } => {
                println!("🚫 Отписка: {} от {}", peer_id, topic);
                log_event(log_tx, format!("Отписка от {:?} от {}", topic, peer_id));
            }
            gossipsub::Event::GossipsubNotSupported { peer_id } => {
                println!("🤷‍♂️ {} не поддерживает Gossipsub", peer_id);
                log_event(log_tx, format!("{} не поддерживает Gossipsub", peer_id));
            }
        },
        SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(event)) => match event {
            mdns::Event::Discovered(list) => {
                for (peer_id, addr) in list {
                    println!("🔍 mDNS нашёл {} по {}", peer_id, addr);
                    log_event(log_tx, format!("mDNS обнаружил {} по {}", peer_id, addr));

                    if swarm.is_connected(&peer_id) {
                        log_event(
                            log_tx,
                            format!("Пропускаем dial к {}: уже есть соединение", peer_id),
                        );
                    } else if dial_state.pending_dials.contains(&peer_id) {
                        log_event(
                            log_tx,
                            format!(
                                "Пропускаем dial к {}: исходящее соединение уже в процессе",
                                peer_id
                            ),
                        );
                    } else {
                        if let Some(last) = dial_state.last_dial_attempt.get(&peer_id) {
                            if last.elapsed() < dial_state.min_dial_interval {
                                log_event(
                                    log_tx,
                                    format!(
                                        "Пропускаем dial к {}: недавно уже пытались ({} ms назад)",
                                        peer_id,
                                        last.elapsed().as_millis()
                                    ),
                                );
                                continue;
                            }
                        }

                        dial_state.pending_dials.insert(peer_id);
                        dial_state.last_dial_attempt.insert(peer_id, Instant::now());
                        match swarm.dial(addr.clone()) {
                            Ok(_) => log_event(
                                log_tx,
                                format!("Инициирован dial к {} по {}", peer_id, addr),
                            ),
                            Err(err) => {
                                dial_state.pending_dials.remove(&peer_id);
                                log_event(
                                    log_tx,
                                    format!("Не удалось dial к {} по {}: {:?}", peer_id, addr, err),
                                )
                            }
                        }
                    }
                }
                log_mesh_state(log_tx, swarm, &chat_topic.hash(), "После mDNS");
            }
            mdns::Event::Expired(list) => {
                for (peer_id, addr) in list {
                    println!("⌛️ mDNS потерял {} с {}", peer_id, addr);
                    log_event(log_tx, format!("mDNS потерял {} с {}", peer_id, addr));
                    dial_state.pending_dials.remove(&peer_id);
                    dial_state.last_dial_attempt.remove(&peer_id);
                }
            }
        },
        _ => {}
    }
}
