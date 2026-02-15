use crate::config::{load_config, parse_startup_args, save_config};
use crate::handlers::{handle_stdin_line, handle_swarm_event, DialState};
use crate::logging::{log_event, run_log_writer};
use crate::p2p::build_swarm;
use libp2p::{futures::StreamExt, gossipsub, Multiaddr, PeerId};
use std::{error::Error, path::PathBuf};
use tokio::io::{self, AsyncBufReadExt};
use tokio::sync::mpsc::unbounded_channel;

pub async fn run() -> Result<(), Box<dyn Error>> {
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

    let mut swarm = build_swarm(id_keys, local_peer_id)?;
    let chat_topic = gossipsub::IdentTopic::new("grok-chat");
    swarm.behaviour_mut().gossipsub.subscribe(&chat_topic)?;

    let listen_port = config.listen_port.unwrap_or(0);
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{listen_port}").parse()?)?;

    if let Some(addr_str) = config.bootstrap_addr.clone() {
        let remote: Multiaddr = addr_str.parse()?;
        swarm.dial(remote)?;
        println!("🚀 Попытка подключения к: {}", addr_str);
    }

    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut dial_state = DialState::new();

    loop {
        tokio::select! {
            line = stdin.next_line() => {
                let line = line?.expect("stdin closed");
                if !line.is_empty() {
                    handle_stdin_line(line, &mut swarm, &chat_topic, &log_tx).await;
                }
            }
            event = swarm.select_next_some() => {
                handle_swarm_event(event, &mut swarm, &chat_topic, &log_tx, &mut dial_state);
            }
        }
    }
}
