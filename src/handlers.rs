// Коротко про этот файл:
// 1) Обрабатывает ввод пользователя (stdin) и публикует сообщения.
// 2) Обрабатывает все события Swarm (соединения, gossipsub, mdns).
// 3) Контролирует dial-штормы через состояние DialState.
// Обработчики событий приложения:
// - ввод строки из консоли (публикация сообщения)
// - события Swarm (сеть, mDNS, gossipsub)
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

// Текущее состояние попыток dial, чтобы не устраивать шторм переподключений.
pub struct DialState {
    // Пиры, к которым уже идёт исходящее подключение прямо сейчас.
    pending_dials: HashSet<PeerId>,
    // Когда в последний раз пытались dial для конкретного пира.
    last_dial_attempt: HashMap<PeerId, Instant>,
    // Минимальная пауза между повторными попытками dial к одному и тому же пиру.
    min_dial_interval: Duration,
}

impl DialState {
    // Стартовое состояние throttling-механизма.
    pub fn new() -> Self {
        Self {
            pending_dials: HashSet::new(),
            last_dial_attempt: HashMap::new(),
            min_dial_interval: Duration::from_secs(10),
        }
    }
}

// Обработка одной введённой пользователем строки.
// Внутри: диагностика mesh -> попытка publish -> fallback при InsufficientPeers.
pub async fn handle_stdin_line(
    line: String,
    swarm: &mut Swarm<MyBehaviour>,
    chat_topic: &gossipsub::IdentTopic,
    log_tx: &UnboundedSender<String>,
) {
    // Логируем исходный ввод для трассировки.
    log_event(log_tx, format!("Считываем ввод: {}", line));

    // Берём текущую картину gossipsub перед публикацией.
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

    // Если mesh пуст, но известные пиры есть, даём сети чуть-чуть времени
    // (heartbeat), чтобы mesh успел обновиться.
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

    // Предупреждение для пользователя: шанс доставки может быть ниже.
    if mesh_peer_count == 0 {
        println!(
            "⚠️ В mesh ещё нет подписанных пиров для {:?}. Публикация, скорее всего, не дойдёт.",
            topic_hash
        );
    }

    // Основная попытка публикации сообщения.
    let result = swarm
        .behaviour_mut()
        .gossipsub
        .publish(chat_topic.clone(), line.as_bytes());

    match result {
        Ok(_) => {
            // Успешная отправка.
            println!("📤 Отправлено!");
            log_event(log_tx, format!("Отправлено сообщение: {}", line));
        }
        Err(gossipsub::PublishError::InsufficientPeers) => {
            // Если не хватает пиров в mesh, пробуем fallback:
            // временно добавить известных пиров как explicit и отправить повторно.
            println!("📡 Мало пиров для Mesh, пробуем принудительную активацию...");
            log_event(
                log_tx,
                format!(
                    "InsufficientPeers (mesh={}). Добавляем явных пиров.",
                    mesh_peer_count
                ),
            );

            // Собираем список известных пиров и добавляем их в explicit.
            let peers: Vec<PeerId> = swarm
                .behaviour_mut()
                .gossipsub
                .all_peers()
                .map(|p| *p.0)
                .collect();
            for peer_id in &peers {
                swarm.behaviour_mut().gossipsub.add_explicit_peer(peer_id);
            }

            // Логируем состояние после fallback-действий.
            log_mesh_state(log_tx, swarm, &chat_topic.hash(), "После добавления явных пиров");

            // Повторная попытка публикации.
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
            // Любая другая ошибка публикации.
            println!("❌ Ошибка: {:?}", err);
            log_event(log_tx, format!("Ошибка publish: {:?}", err));
        }
    }
}

// Центральный обработчик событий Swarm.
// Сюда попадают все сетевые события, discovery и gossipsub-ивенты.
pub fn handle_swarm_event(
    event: SwarmEvent<MyBehaviourEvent>,
    swarm: &mut Swarm<MyBehaviour>,
    chat_topic: &gossipsub::IdentTopic,
    log_tx: &UnboundedSender<String>,
    dial_state: &mut DialState,
) {
    match event {
        // Нода начала слушать новый адрес.
        SwarmEvent::NewListenAddr { address, .. } => {
            println!("🎧 Слушаю на: {}", address);
            log_event(log_tx, format!("Новый адрес прослушивания: {}", address));
        }
        // Соединение с пиром установлено.
        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            println!("✅ Есть контакт с: {}", peer_id);
            log_event(log_tx, format!("Контакт с {}", peer_id));
            // Больше не считаем этот dial pending.
            dial_state.pending_dials.remove(&peer_id);
            log_mesh_state(log_tx, swarm, &chat_topic.hash(), "После соединения");
        }
        // Соединение закрыто (нормально или с ошибкой).
        SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
            log_event(
                log_tx,
                format!("Соединение закрыто с {}: {:?}", peer_id, cause),
            );
        }
        // Ошибка исходящего подключения.
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
        // События gossipsub.
        SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(event)) => match event {
            gossipsub::Event::Message {
                propagation_source,
                message,
                ..
            } => {
                // Получили сообщение от другого пира.
                let text = String::from_utf8_lossy(&message.data);
                println!("📩 Сообщение от {}: {}", propagation_source, text);
                log_event(
                    log_tx,
                    format!("Получено сообщение от {}: {}", propagation_source, text),
                );
                log_mesh_state(log_tx, swarm, &chat_topic.hash(), "После получения сообщения");
            }
            // Пир подписался на топик.
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
            // Пир отписался от топика.
            gossipsub::Event::Unsubscribed { peer_id, topic } => {
                println!("🚫 Отписка: {} от {}", peer_id, topic);
                log_event(log_tx, format!("Отписка от {:?} от {}", topic, peer_id));
            }
            // Удалённая нода не умеет gossipsub.
            gossipsub::Event::GossipsubNotSupported { peer_id } => {
                println!("🤷‍♂️ {} не поддерживает Gossipsub", peer_id);
                log_event(log_tx, format!("{} не поддерживает Gossipsub", peer_id));
            }
        },
        // События mDNS discovery.
        SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(event)) => match event {
            mdns::Event::Discovered(list) => {
                for (peer_id, addr) in list {
                    println!("🔍 mDNS нашёл {} по {}", peer_id, addr);
                    log_event(log_tx, format!("mDNS обнаружил {} по {}", peer_id, addr));

                    // Если уже подключены — ничего делать не нужно.
                    if swarm.is_connected(&peer_id) {
                        log_event(
                            log_tx,
                            format!("Пропускаем dial к {}: уже есть соединение", peer_id),
                        );
                    // Если dial уже запущен — тоже пропускаем.
                    } else if dial_state.pending_dials.contains(&peer_id) {
                        log_event(
                            log_tx,
                            format!(
                                "Пропускаем dial к {}: исходящее соединение уже в процессе",
                                peer_id
                            ),
                        );
                    } else {
                        // Throttle: не дёргаем один и тот же peer слишком часто.
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

                        // Запоминаем новую попытку dial.
                        dial_state.pending_dials.insert(peer_id);
                        dial_state.last_dial_attempt.insert(peer_id, Instant::now());
                        match swarm.dial(addr.clone()) {
                            Ok(_) => log_event(
                                log_tx,
                                format!("Инициирован dial к {} по {}", peer_id, addr),
                            ),
                            Err(err) => {
                                dial_state.pending_dials.remove(&peer_id);
                                // Если dial не стартовал — убираем peer из pending.
                                log_event(
                                    log_tx,
                                    format!("Не удалось dial к {} по {}: {:?}", peer_id, addr, err),
                                )
                            }
                        }
                    }
                }
                // Полезный диагностический срез после очередной волны discovery.
                log_mesh_state(log_tx, swarm, &chat_topic.hash(), "После mDNS");
            }
            mdns::Event::Expired(list) => {
                // Узел пропал из mDNS-видимости.
                for (peer_id, addr) in list {
                    println!("⌛️ mDNS потерял {} с {}", peer_id, addr);
                    log_event(log_tx, format!("mDNS потерял {} с {}", peer_id, addr));
                    // Чистим служебные структуры состояния dial.
                    dial_state.pending_dials.remove(&peer_id);
                    dial_state.last_dial_attempt.remove(&peer_id);
                }
            }
        },
        // Остальные события пока игнорируем.
        _ => {}
    }
}
