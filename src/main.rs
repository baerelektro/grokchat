use libp2p::futures::StreamExt; // Правильный импорт для select_next_some
use libp2p::{
    core::upgrade,
    gossipsub, mdns, noise,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, Transport,
};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs,
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};
use tokio::fs::OpenOptions;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(NetworkBehaviour)]
struct MyBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

#[derive(Debug, Clone, Default)]
struct AppConfig {
    bootstrap_addr: Option<String>,
    listen_port: Option<u16>,
}

fn load_config(config_path: &PathBuf) -> Result<Option<AppConfig>, Box<dyn Error>> {
    if !config_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(config_path)?;
    let mut config = AppConfig::default();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "bootstrap_addr" if !value.is_empty() => {
                    config.bootstrap_addr = Some(value.to_owned());
                }
                "listen_port" if !value.is_empty() => {
                    if let Ok(port) = value.parse::<u16>() {
                        config.listen_port = Some(port);
                    }
                }
                _ => {}
            }
        }
    }

    Ok(Some(config))
}

fn save_config(config_path: &PathBuf, config: &AppConfig) -> Result<(), Box<dyn Error>> {
    let mut lines = Vec::new();
    lines.push("# grokchat startup config".to_owned());
    lines.push(format!(
        "bootstrap_addr={}",
        config.bootstrap_addr.clone().unwrap_or_default()
    ));
    lines.push(format!(
        "listen_port={}",
        config
            .listen_port
            .map(|p| p.to_string())
            .unwrap_or_default()
    ));
    fs::write(config_path, lines.join("\n"))?;
    Ok(())
}

fn parse_startup_args() -> Result<(Option<String>, Option<u16>), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut bootstrap_addr: Option<String> = None;
    let mut listen_port: Option<u16> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--peer" => {
                if i + 1 >= args.len() {
                    return Err("Ожидается значение после --peer".into());
                }
                bootstrap_addr = Some(args[i + 1].clone());
                i += 2;
            }
            "--port" => {
                if i + 1 >= args.len() {
                    return Err("Ожидается значение после --port".into());
                }
                listen_port = Some(
                    args[i + 1]
                        .parse::<u16>()
                        .map_err(|_| "Порт должен быть числом 0..65535")?,
                );
                i += 2;
            }
            value if value.starts_with('-') => {
                return Err(format!("Неизвестный аргумент: {value}").into());
            }
            value => {
                if bootstrap_addr.is_none() {
                    bootstrap_addr = Some(value.to_owned());
                }
                i += 1;
            }
        }
    }

    Ok((bootstrap_addr, listen_port))
}

async fn run_log_writer(log_path: PathBuf, mut receiver: UnboundedReceiver<String>) {
    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .await
    {
        Ok(mut file) => {
            while let Some(mut line) = receiver.recv().await {
                if !line.ends_with('\n') {
                    line.push('\n');
                }
                if let Err(err) = file.write_all(line.as_bytes()).await {
                    eprintln!("Не удалось записать в лог {}: {err}", log_path.display());
                }
            }
        }
        Err(err) => {
            eprintln!("Не удалось открыть лог {}: {err}", log_path.display());
        }
    }
}

fn log_event(sender: &UnboundedSender<String>, message: String) {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let timestamp = format!("{}.{:03}", now.as_secs(), now.subsec_millis());
    let _ = sender.send(format!("[{timestamp}] {message}"));
}

fn log_mesh_state(
    sender: &UnboundedSender<String>,
    swarm: &Swarm<MyBehaviour>,
    topic_hash: &gossipsub::TopicHash,
    note: &str,
) {
    let behaviour = swarm.behaviour();
    let mesh_peers: Vec<_> = behaviour
        .gossipsub
        .mesh_peers(topic_hash)
        .cloned()
        .collect();
    let known_peers: Vec<_> = behaviour
        .gossipsub
        .all_peers()
        .map(|(peer_id, _)| *peer_id)
        .collect();
    log_event(
        sender,
        format!(
            "{} mesh={} known={} mesh_list={:?} known_list={:?}",
            note,
            mesh_peers.len(),
            known_peers.len(),
            mesh_peers,
            known_peers
        ),
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config_path = PathBuf::from("grokchat.conf");
    let loaded_config = load_config(&config_path)?;
    let first_run = loaded_config.is_none();
    let mut config = loaded_config.unwrap_or_default();
    let (arg_bootstrap_addr, arg_listen_port) = parse_startup_args()?;
    let bootstrap_from_args = arg_bootstrap_addr.is_some();
    let port_from_args = arg_listen_port.is_some();

    if let Some(addr) = arg_bootstrap_addr {
        config.bootstrap_addr = Some(addr);
    }
    if let Some(port) = arg_listen_port {
        config.listen_port = Some(port);
    }

    if first_run || bootstrap_from_args || port_from_args {
        save_config(&config_path, &config)?;
    }

    let id_keys = libp2p::identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(id_keys.public());
    let (log_tx, log_rx) = unbounded_channel();
    let log_path = PathBuf::from("grokchat.log");
    tokio::spawn(async move {
        run_log_writer(log_path, log_rx).await;
    });
    if first_run {
        log_event(
            &log_tx,
            format!("Первый запуск: конфиг сохранён в {}", config_path.display()),
        );
    }
    log_event(
        &log_tx,
        format!(
            "Конфиг: bootstrap_addr={:?}, listen_port={:?}",
            config.bootstrap_addr, config.listen_port
        ),
    );
    log_event(&log_tx, format!("Локальный Peer ID: {}", local_peer_id));
    println!("🔐 Твой Peer ID: {}", local_peer_id);

    let transport = tcp::tokio::Transport::default()
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::Config::new(&id_keys)?)
        .multiplex(yamux::Config::default())
        .boxed();

    let message_id_fn = |message: &gossipsub::Message| {
        let mut s = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&message.data, &mut s);
        gossipsub::MessageId::from(std::hash::Hasher::finish(&s).to_string())
    };

    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(Duration::from_millis(100))
        .validation_mode(gossipsub::ValidationMode::Permissive)
        .mesh_n_low(1)
        .mesh_n(2)
        .mesh_outbound_min(1)
        .flood_publish(true)
        .allow_self_origin(true)
        .do_px()
        .check_explicit_peers_ticks(1)
        .support_floodsub()
        .message_id_fn(message_id_fn)
        .build()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys),
        gossipsub_config,
    )?;

    let chat_topic = gossipsub::IdentTopic::new("grok-chat");
    gossipsub.subscribe(&chat_topic)?;

    let mut swarm = libp2p::Swarm::new(
        transport,
        MyBehaviour {
            gossipsub,
            mdns: mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?,
        },
        local_peer_id,
        libp2p::swarm::Config::with_tokio_executor()
            .with_idle_connection_timeout(Duration::from_secs(300)),
    );

    let listen_port = config.listen_port.unwrap_or(0);
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{listen_port}").parse()?)?;

    if let Some(addr_str) = config.bootstrap_addr.clone() {
        let remote: Multiaddr = addr_str.parse()?;
        swarm.dial(remote)?;
        println!("🚀 Попытка подключения к: {}", addr_str);
    }

    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut pending_dials: HashSet<PeerId> = HashSet::new();
    let mut last_dial_attempt: HashMap<PeerId, Instant> = HashMap::new();
    let min_dial_interval = Duration::from_secs(10);

    loop {
        tokio::select! {
            line = stdin.next_line() => {
                let line = line?.expect("stdin closed");
                if !line.is_empty() {
                    log_event(&log_tx, format!("Считываем ввод: {}", line));
                    let topic_hash = chat_topic.hash();
                    let (mesh_peer_count, known_peer_count) = {
                        let behaviour = swarm.behaviour();
                        (
                            behaviour.gossipsub.mesh_peers(&topic_hash).count(),
                            behaviour.gossipsub.all_peers().count(),
                        )
                    };
                    log_event(&log_tx, format!("Перед publish: mesh={} known={} для {:?}", mesh_peer_count, known_peer_count, topic_hash));
                    let mut mesh_peer_count = mesh_peer_count;
                    if mesh_peer_count == 0 && known_peer_count > 0 {
                        log_event(&log_tx, "Mesh=0 при известных пирах, ждём heartbeat перед publish".to_owned());
                        tokio::time::sleep(Duration::from_millis(350)).await;
                        mesh_peer_count = swarm
                            .behaviour()
                            .gossipsub
                            .mesh_peers(&topic_hash)
                            .count();
                        log_event(&log_tx, format!("После ожидания перед publish: mesh={} known={}", mesh_peer_count, known_peer_count));
                    }

                    if mesh_peer_count == 0 {
                        println!("⚠️ В mesh ещё нет подписанных пиров для {:?}. Публикация, скорее всего, не дойдёт.", topic_hash);
                    }

                    let result = swarm.behaviour_mut().gossipsub.publish(chat_topic.clone(), line.as_bytes());

                    match result {
                        Ok(_) => {
                            println!("📤 Отправлено!");
                            log_event(&log_tx, format!("Отправлено сообщение: {}", line));
                        }
                        Err(gossipsub::PublishError::InsufficientPeers) => {
                            println!("📡 Мало пиров для Mesh, пробуем принудительную активацию...");
                            log_event(&log_tx, format!("InsufficientPeers (mesh={}). Добавляем явных пиров.", mesh_peer_count));

                            let peers: Vec<PeerId> = swarm.behaviour_mut().gossipsub.all_peers()
                                .map(|p| *p.0)
                                .collect();
                            for peer_id in &peers {
                                swarm.behaviour_mut().gossipsub.add_explicit_peer(peer_id);
                            }

                            log_mesh_state(
                                &log_tx,
                                &swarm,
                                &chat_topic.hash(),
                                "После добавления явных пиров",
                            );

                            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(chat_topic.clone(), line.as_bytes()) {
                                println!("⚠️ Даже после активации: {:?}. Жди прогрева...", e);
                                log_event(&log_tx, format!("Повторная отправка не удалась: {:?}", e));
                            } else {
                                println!("📤 Протолкнули через Flood!");
                                log_event(&log_tx, "Удачная Flood-публикация".to_owned());
                            }
                        }
                        Err(e) => {
                            println!("❌ Ошибка: {:?}", e);
                            log_event(&log_tx, format!("Ошибка publish: {:?}", e));
                        }
                    }
                }
            }
            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("🎧 Слушаю на: {}", address);
                    log_event(&log_tx, format!("Новый адрес прослушивания: {}", address));
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    println!("✅ Есть контакт с: {}", peer_id);
                    log_event(&log_tx, format!("Контакт с {}", peer_id));
                    pending_dials.remove(&peer_id);
                    log_mesh_state(
                        &log_tx,
                        &swarm,
                        &chat_topic.hash(),
                        "После соединения",
                    );
                }
                SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                    log_event(
                        &log_tx,
                        format!("Соединение закрыто с {}: {:?}", peer_id, cause),
                    );
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    if let Some(peer_id) = peer_id {
                        pending_dials.remove(&peer_id);
                        log_event(&log_tx, format!("Ошибка исходящего соединения с {}: {:?}", peer_id, error));
                    } else {
                        log_event(&log_tx, format!("Ошибка исходящего соединения без peer_id: {:?}", error));
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
                        log_event(&log_tx, format!("Получено сообщение от {}: {}", propagation_source, text));
                        log_mesh_state(
                            &log_tx,
                            &swarm,
                            &chat_topic.hash(),
                            "После получения сообщения",
                        );
                    }
                    gossipsub::Event::Subscribed { peer_id, topic: event_topic } => {
                        println!("🛰️ Подписка: {} -> {}", peer_id, event_topic);
                        log_event(&log_tx, format!("Подписка на {:?} от {}", event_topic, peer_id));
                        log_mesh_state(
                            &log_tx,
                            &swarm,
                            &event_topic,
                            "После подписки",
                        );
                    }
                    gossipsub::Event::Unsubscribed { peer_id, topic } => {
                        println!("🚫 Отписка: {} от {}", peer_id, topic);
                        log_event(&log_tx, format!("Отписка от {:?} от {}", topic, peer_id));
                    }
                    gossipsub::Event::GossipsubNotSupported { peer_id } => {
                        println!("🤷‍♂️ {} не поддерживает Gossipsub", peer_id);
                        log_event(&log_tx, format!("{} не поддерживает Gossipsub", peer_id));
                    }
                },
                SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(event)) => match event {
                    mdns::Event::Discovered(list) => {
                        for (peer_id, addr) in list {
                            println!("🔍 mDNS нашёл {} по {}", peer_id, addr);
                            log_event(&log_tx, format!("mDNS обнаружил {} по {}", peer_id, addr));

                            if swarm.is_connected(&peer_id) {
                                log_event(
                                    &log_tx,
                                    format!("Пропускаем dial к {}: уже есть соединение", peer_id),
                                );
                            } else if pending_dials.contains(&peer_id) {
                                log_event(
                                    &log_tx,
                                    format!(
                                        "Пропускаем dial к {}: исходящее соединение уже в процессе",
                                        peer_id
                                    ),
                                );
                            } else {
                                if let Some(last) = last_dial_attempt.get(&peer_id) {
                                    if last.elapsed() < min_dial_interval {
                                        log_event(
                                            &log_tx,
                                            format!(
                                                "Пропускаем dial к {}: недавно уже пытались ({} ms назад)",
                                                peer_id,
                                                last.elapsed().as_millis()
                                            ),
                                        );
                                        continue;
                                    }
                                }

                                pending_dials.insert(peer_id);
                                last_dial_attempt.insert(peer_id, Instant::now());
                                match swarm.dial(addr.clone()) {
                                    Ok(_) => log_event(
                                        &log_tx,
                                        format!("Инициирован dial к {} по {}", peer_id, addr),
                                    ),
                                    Err(err) => {
                                        pending_dials.remove(&peer_id);
                                        log_event(
                                            &log_tx,
                                            format!(
                                                "Не удалось dial к {} по {}: {:?}",
                                                peer_id, addr, err
                                            ),
                                        )
                                    }
                                }
                            }
                        }
                        log_mesh_state(
                            &log_tx,
                            &swarm,
                            &chat_topic.hash(),
                            "После mDNS",
                        );
                    }
                    mdns::Event::Expired(list) => {
                        for (peer_id, addr) in list {
                            println!("⌛️ mDNS потерял {} с {}", peer_id, addr);
                            log_event(&log_tx, format!("mDNS потерял {} с {}", peer_id, addr));
                            pending_dials.remove(&peer_id);
                            last_dial_attempt.remove(&peer_id);
                        }
                    }
                },
                _ => {}
            }
        }
    }
}
