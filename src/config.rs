// Работа с конфигом приложения: загрузка/сохранение файла и разбор аргументов CLI.
use std::{error::Error, fs, path::PathBuf};

// Что мы сохраняем между запусками.
// bootstrap_addr — к какому пиру пробовать подключаться при старте.
// listen_port — на каком порту слушать входящие подключения.
#[derive(Debug, Clone, Default)]
pub struct AppConfig {
    pub bootstrap_addr: Option<String>,
    pub listen_port: Option<u16>,
}

// Читаем конфиг из файла.
// Если файла нет — это не ошибка, просто возвращаем None (первый запуск).
pub fn load_config(config_path: &PathBuf) -> Result<Option<AppConfig>, Box<dyn Error>> {
    if !config_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(config_path)?;
    let mut config = AppConfig::default();

    // Проходим по строкам и парсим формат key=value.
    for line in content.lines() {
        let trimmed = line.trim();
        // Пустые строки и комментарии игнорируем.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match key {
                // Адрес bootstrap-пира (multiaddr) сохраняем как строку.
                "bootstrap_addr" if !value.is_empty() => {
                    config.bootstrap_addr = Some(value.to_owned());
                }
                // Порт должен быть числом u16.
                "listen_port" if !value.is_empty() => {
                    if let Ok(port) = value.parse::<u16>() {
                        config.listen_port = Some(port);
                    }
                }
                // Неизвестные ключи молча пропускаем, чтобы конфиг был более гибким.
                _ => {}
            }
        }
    }

    Ok(Some(config))
}

// Сохраняем конфиг в простой ini-подобный текстовый файл.
pub fn save_config(config_path: &PathBuf, config: &AppConfig) -> Result<(), Box<dyn Error>> {
    let mut lines = Vec::new();
    // Комментарий в шапке файла, чтобы было понятно, что это.
    lines.push("# grokchat startup config".to_owned());
    // Если значения нет, пишем пустую строку после '='.
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

// Разбираем аргументы запуска.
// Поддержка:
//   --peer <MULTIADDR>
//   --port <PORT>
//   <MULTIADDR> (позиционный аргумент вместо --peer)
pub fn parse_startup_args() -> Result<(Option<String>, Option<u16>), Box<dyn Error>> {
    // skip(1): пропускаем имя исполняемого файла.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut bootstrap_addr: Option<String> = None;
    let mut listen_port: Option<u16> = None;

    // Ручной проход по массиву аргументов, чтобы удобно читать пары ключ-значение.
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--peer" => {
                // После --peer обязательно должно быть значение.
                if i + 1 >= args.len() {
                    return Err("Ожидается значение после --peer".into());
                }
                bootstrap_addr = Some(args[i + 1].clone());
                i += 2;
            }
            "--port" => {
                // После --port обязательно должно быть значение.
                if i + 1 >= args.len() {
                    return Err("Ожидается значение после --port".into());
                }
                // Валидируем, что порт — число в диапазоне u16.
                listen_port = Some(
                    args[i + 1]
                        .parse::<u16>()
                        .map_err(|_| "Порт должен быть числом 0..65535")?,
                );
                i += 2;
            }
            // Любой неизвестный флаг считаем ошибкой, чтобы не было тихих опечаток.
            value if value.starts_with('-') => {
                return Err(format!("Неизвестный аргумент: {value}").into());
            }
            value => {
                // Позиционный аргумент используем как bootstrap_addr,
                // но только если он ещё не задан.
                if bootstrap_addr.is_none() {
                    bootstrap_addr = Some(value.to_owned());
                }
                i += 1;
            }
        }
    }

    Ok((bootstrap_addr, listen_port))
}
