// Коротко про этот файл:
// 1) Создаёт Swarm (TCP + Noise + Yamux + gossipsub + mdns).
// 2) Настраивает gossipsub для небольшого p2p-чата.
// 3) Даёт helper для логирования состояния mesh/known peers.
// Сетевой слой: создаём Swarm (transport + протоколы поведения) и утилиты по состоянию mesh.
use crate::logging::log_event;
use libp2p::{
    core::upgrade,
    gossipsub, mdns, noise,
    swarm::{NetworkBehaviour, Swarm},
    tcp, yamux, PeerId, Transport,
};
use std::{error::Error, io, time::Duration};
use tokio::sync::mpsc::UnboundedSender;

// Общий набор поведения ноды:
// - gossipsub: рассылка и получение сообщений по топикам
// - mdns: поиск соседей в локальной сети
#[derive(NetworkBehaviour)]
pub struct MyBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
}

// Собираем полностью готовый Swarm для работы чата.
pub fn build_swarm(
    id_keys: libp2p::identity::Keypair,
    local_peer_id: PeerId,
) -> Result<Swarm<MyBehaviour>, Box<dyn Error>> {
    // Базовый транспорт: TCP + апгрейд + Noise + Yamux.
    let transport = tcp::tokio::Transport::default()
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::Config::new(&id_keys)?)
        .multiplex(yamux::Config::default())
        .boxed();

    // Формируем MessageId как хеш payload, чтобы одинаковые сообщения
    // считались дублями и не гуляли бесконечно по сети.
    let message_id_fn = |message: &gossipsub::Message| {
        let mut state = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&message.data, &mut state);
        gossipsub::MessageId::from(std::hash::Hasher::finish(&state).to_string())
    };

    // Конфиг gossipsub под маленькие p2p-сети и быстрый прогрев mesh.
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

    // Подписываем сообщения приватным ключом ноды.
    let gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys),
        gossipsub_config,
    )?;

    // Финальная сборка Swarm с двумя поведениями.
    Ok(Swarm::new(
        transport,
        MyBehaviour {
            gossipsub,
            mdns: mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?,
        },
        local_peer_id,
        libp2p::swarm::Config::with_tokio_executor()
            .with_idle_connection_timeout(Duration::from_secs(300)),
    ))
}

// Диагностический лог состояния gossipsub:
// mesh_peers — активные соседи в mesh
// known_peers — все известные peers для топика
pub fn log_mesh_state(
    sender: &UnboundedSender<String>,
    swarm: &Swarm<MyBehaviour>,
    topic_hash: &gossipsub::TopicHash,
    note: &str,
) {
    let behaviour = swarm.behaviour();
    let mesh_peers: Vec<_> = behaviour.gossipsub.mesh_peers(topic_hash).cloned().collect();
    let known_peers: Vec<_> = behaviour
        .gossipsub
        .all_peers()
        .map(|(peer_id, _)| *peer_id)
        .collect();

    // Пишем подробный срез состояния в лог, чтобы отлаживать доставку.
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
