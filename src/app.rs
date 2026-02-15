// Коротко про этот файл:
// 1) Собирает конфиг (файл + аргументы запуска).
// 2) Поднимает сеть (Swarm), логирование и подписку на чат-топик.
// 3) Крутит главный цикл: stdin -> publish, network events -> handlers.
// Главный оркестратор приложения:
// грузим конфиг, поднимаем сеть, запускаем цикл обработки stdin + сетевых событий.
use crate::config::{load_config, parse_startup_args, save_config};
use crate::handlers::{handle_stdin_line, handle_swarm_event, DialState};
use crate::logging::{log_event, run_log_writer};
use crate::p2p::build_swarm;
use libp2p::{futures::StreamExt, gossipsub, Multiaddr, PeerId};
use std::{error::Error, path::PathBuf};
use tokio::io::{self, AsyncBufReadExt};
use tokio::sync::mpsc::unbounded_channel;

// Основной жизненный цикл приложения.
pub async fn run() -> Result<(), Box<dyn Error>> {
    // 1) Загружаем/подготавливаем конфиг.
    let config_path = PathBuf::from("grokchat.conf");
    let loaded_config = load_config(&config_path)?;
    // first_run нужен, чтобы понять, нужно ли сохранять конфиг сразу.
    let first_run = loaded_config.is_none();
    let mut config = loaded_config.unwrap_or_default();

    // 2) Читаем аргументы CLI и накладываем их поверх конфига.
    let (arg_bootstrap_addr, arg_listen_port) = parse_startup_args()?;
    let bootstrap_from_args = arg_bootstrap_addr.is_some();
    let port_from_args = arg_listen_port.is_some();

    if let Some(addr) = arg_bootstrap_addr {
        config.bootstrap_addr = Some(addr);
    }
    if let Some(port) = arg_listen_port {
        config.listen_port = Some(port);
    }

    // Сохраняем конфиг на первый запуск и при явном переопределении через CLI.
    if first_run || bootstrap_from_args || port_from_args {
        save_config(&config_path, &config)?;
    }

    // 3) Создаём локальную крипто-идентичность и канал логирования.
    let id_keys = libp2p::identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(id_keys.public());
    let (log_tx, log_rx) = unbounded_channel();

    // Запускаем фонового писателя логов в файл.
    let log_path = PathBuf::from("grokchat.log");
    tokio::spawn(async move {
        run_log_writer(log_path, log_rx).await;
    });

    // Служебные диагностические записи в лог.
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

    // 4) Поднимаем p2p-сеть (Swarm) и подписываемся на чат-топик.
    let mut swarm = build_swarm(id_keys, local_peer_id)?;
    let chat_topic = gossipsub::IdentTopic::new("grok-chat");
    swarm.behaviour_mut().gossipsub.subscribe(&chat_topic)?;

    // 5) Начинаем слушать TCP-порт (из конфига или случайный, если 0).
    let listen_port = config.listen_port.unwrap_or(0);
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{listen_port}").parse()?)?;

    // 6) Если в конфиге есть bootstrap-адрес — пробуем подключиться сразу при старте.
    if let Some(addr_str) = config.bootstrap_addr.clone() {
        let remote: Multiaddr = addr_str.parse()?;
        swarm.dial(remote)?;
        println!("🚀 Попытка подключения к: {}", addr_str);
    }

    // 7) Готовим источники событий: stdin и сетевые события swarm.
    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut dial_state = DialState::new();

    // 8) Бесконечный цикл обработки:
    // - если ввели строку в консоль => публикуем сообщение
    // - если пришло сетевое событие => обрабатываем его
    loop {
        tokio::select! {
            line = stdin.next_line() => {
                let line = line?.expect("stdin closed");
                if !line.is_empty() {
                    // Обработка пользовательского сообщения.
                    handle_stdin_line(line, &mut swarm, &chat_topic, &log_tx).await;
                }
            }
            event = swarm.select_next_some() => {
                // Обработка сетевого события.
                handle_swarm_event(event, &mut swarm, &chat_topic, &log_tx, &mut dial_state);
            }
        }
    }
}
